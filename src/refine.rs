//! Explicit flat-art cleanup and recoloring templates. Never enabled by `trace`.
use crate::{Error, IndexedImage, Options, contours, flatten, quantize, simplify};
use image::RgbaImage;
use std::{collections::HashMap, fmt::Write};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Paint {
    #[default]
    Flat,
    /// Closed, independently selectable regions with visible editing guides.
    Outline,
}

#[derive(Clone, Debug)]
pub struct RefineOptions {
    /// Recognize isolated circles and convex straight-sided polygons before smoothing.
    pub geometry: bool,
    /// Pixel-boundary fitting tolerance in source pixels (0.1..=3).
    pub geometry_tolerance: f64,
    /// Maximum reconstructed corner extension in source pixels (0..=8).
    pub corner_extension: f64,
    /// Gaussian contour smoothing scale in source pixels, 0 disables, maximum 8.
    /// Shared boundaries are fitted once. Not a strict displacement/topology bound.
    pub smooth: f64,
    /// Merge foreground components of at most this many source pixels into a
    /// larger adjacent region. Zero disables cleanup. Background holes are kept.
    pub min_region_area: usize,
    /// Maximum pairwise RGB Euclidean distance within a merged palette group.
    /// RGB is 0..255 per channel; this is NOT a perceptual Delta-E threshold.
    pub merge_distance: f64,
    /// Optional exact palette. Assign original flattened pixels directly to it.
    pub palette: Vec<[u8; 3]>,
    /// Omit every region matching this color, including enclosed negative space.
    pub background: Option<[u8; 3]>,
    pub background_distance: f64,
    /// Group all non-background pixels into foreground, ignoring interior colors.
    /// Requires an explicit background; useful for recoloring gradient silhouettes.
    pub silhouette: bool,
    pub paint: Paint,
    /// Editing guide width, in input pixels. No guide stroke on Flat output.
    pub stroke_width: f64,
}
impl Default for RefineOptions {
    fn default() -> Self {
        Self {
            geometry: false,
            geometry_tolerance: 1.0,
            corner_extension: 3.0,
            smooth: 0.0,
            min_region_area: 0,
            merge_distance: 0.0,
            palette: vec![],
            background: None,
            background_distance: 24.0,
            silhouette: false,
            paint: Paint::Flat,
            stroke_width: 1.0,
        }
    }
}
#[derive(Clone, Debug)]
pub struct RefineStats {
    pub geometry_circles: usize,
    pub geometry_polygons: usize,
    pub palette_before: usize,
    pub palette_after: usize,
    pub paths: usize,
    pub contours: usize,
    pub background_pixels: usize,
}
pub struct RefinedSvg {
    pub svg: String,
    pub stats: RefineStats,
}
fn distance2(a: [u8; 3], b: [u8; 3]) -> f64 {
    (0..3)
        .map(|c| (f64::from(a[c]) - f64::from(b[c])).powi(2))
        .sum()
}
fn validate(img: &RgbaImage, options: &Options, refine: &RefineOptions) -> Result<(), Error> {
    if img.width() == 0
        || img.height() == 0
        || u64::from(img.width()) * u64::from(img.height()) > 16_000_000
    {
        return Err(Error::Dimensions);
    }
    if !(1..=256).contains(&options.colors)
        || !options.tolerance.is_finite()
        || options.tolerance < 0.0
    {
        return Err(Error::Options);
    }
    if !refine.smooth.is_finite() || !(0.0..=8.0).contains(&refine.smooth) {
        return Err(Error::Refine(
            "smooth must be finite in 0..=8 source pixels",
        ));
    }
    if !refine.geometry_tolerance.is_finite()
        || !(0.1..=3.0).contains(&refine.geometry_tolerance)
        || !refine.corner_extension.is_finite()
        || !(0.0..=8.0).contains(&refine.corner_extension)
    {
        return Err(Error::Refine(
            "geometry tolerance must be 0.1..=3, corner extension 0..=8",
        ));
    }
    if (refine.smooth > 0.0 || refine.geometry) && options.tolerance > 0.0 {
        return Err(Error::Refine(
            "smooth and polygon tolerance are separate modes",
        ));
    }
    let max = 255.0 * 3.0_f64.sqrt();
    if !refine.merge_distance.is_finite()
        || !(0.0..=max).contains(&refine.merge_distance)
        || !refine.background_distance.is_finite()
        || !(0.0..=max).contains(&refine.background_distance)
        || !refine.stroke_width.is_finite()
        || refine.stroke_width <= 0.0
        || refine.palette.len() > 256
    {
        return Err(Error::Refine(
            "distances must be finite RGB distances in 0..=441.67, palette <=256, stroke width >0",
        ));
    }
    if refine.silhouette && refine.background.is_none() {
        return Err(Error::Refine(
            "silhouette requires an explicit background color",
        ));
    }
    if refine.silhouette
        && (!refine.palette.is_empty()
            || refine.merge_distance != 0.0
            || refine.min_region_area != 0)
    {
        return Err(Error::Refine(
            "silhouette cannot be combined with palette, merge-distance or min-region-area",
        ));
    }
    if !refine.palette.is_empty() && refine.merge_distance != 0.0 {
        return Err(Error::Refine(
            "fixed palette and merge-distance are separate modes",
        ));
    }
    Ok(())
}

fn prepare(
    img: &RgbaImage,
    options: &Options,
    settings: &RefineOptions,
) -> Result<(IndexedImage, Vec<bool>, usize), Error> {
    validate(img, options, settings)?;
    if settings.silhouette {
        let background = settings.background.unwrap();
        let labels = flatten(img, options.matte)
            .pixels()
            .map(|p| u8::from(distance2(p.0, background) > settings.background_distance.powi(2)))
            .collect();
        return Ok((
            IndexedImage {
                width: img.width(),
                height: img.height(),
                palette: vec![background, [0; 3]],
                labels,
            },
            vec![true, false],
            2,
        ));
    }
    let mut indexed = if settings.palette.is_empty() {
        quantize(img, options)?
    } else {
        let mut palette = settings.palette.clone();
        palette.sort();
        palette.dedup();
        let mut cache = HashMap::new();
        let labels = flatten(img, options.matte)
            .pixels()
            .map(|p| {
                *cache.entry(p.0).or_insert_with(|| {
                    palette
                        .iter()
                        .enumerate()
                        .min_by(|a, b| distance2(p.0, *a.1).total_cmp(&distance2(p.0, *b.1)))
                        .unwrap()
                        .0 as u8
                })
            })
            .collect();
        IndexedImage {
            width: img.width(),
            height: img.height(),
            palette,
            labels,
        }
    };
    let before = indexed.palette.len();
    if settings.merge_distance > 0.0 {
        let mut counts = vec![0usize; before];
        for &label in &indexed.labels {
            counts[label as usize] += 1;
        }
        let mut order: Vec<_> = (0..before).filter(|&i| counts[i] > 0).collect();
        order.sort_by_key(|&i| (std::cmp::Reverse(counts[i]), i));
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for i in order {
            // Complete-link bound prevents a long chain of near colors from
            // swallowing an entire gradient through transitive merging.
            if let Some(group) = groups.iter_mut().find(|g| {
                g.iter().all(|&j| {
                    distance2(indexed.palette[i], indexed.palette[j])
                        <= settings.merge_distance.powi(2)
                })
            }) {
                group.push(i);
            } else {
                groups.push(vec![i]);
            }
        }
        let mut mapping = vec![0u8; before];
        let palette = groups
            .iter()
            .enumerate()
            .map(|(label, group)| {
                for &i in group {
                    mapping[i] = label as u8;
                }
                let total = group.iter().map(|&i| counts[i] as u64).sum::<u64>();
                std::array::from_fn(|c| {
                    ((group
                        .iter()
                        .map(|&i| u64::from(indexed.palette[i][c]) * counts[i] as u64)
                        .sum::<u64>()
                        + total / 2)
                        / total) as u8
                })
            })
            .collect();
        for label in &mut indexed.labels {
            *label = mapping[*label as usize];
        }
        indexed.palette = palette;
    }
    let omit: Vec<bool> = indexed
        .palette
        .iter()
        .map(|&color| {
            settings
                .background
                .is_some_and(|bg| distance2(color, bg) <= settings.background_distance.powi(2))
        })
        .collect();
    if settings.min_region_area > 0 {
        merge_small_regions(&mut indexed, &omit, settings.min_region_area);
    }
    Ok((indexed, omit, before))
}

fn merge_small_regions(indexed: &mut IndexedImage, omit: &[bool], max_area: usize) {
    let w = indexed.width as usize;
    let h = indexed.height as usize;
    let neighbors = |p: usize| {
        let (x, y) = (p % w, p / w);
        [
            x.checked_sub(1).map(|_| p - 1),
            (x + 1 < w).then_some(p + 1),
            y.checked_sub(1).map(|_| p - w),
            (y + 1 < h).then_some(p + w),
        ]
    };
    let mut membership = vec![usize::MAX; w * h];
    let mut labels = Vec::new();
    let mut areas = Vec::new();
    for seed in 0..w * h {
        if membership[seed] != usize::MAX {
            continue;
        }
        let label = indexed.labels[seed];
        let id = labels.len();
        labels.push(label);
        areas.push(0usize);
        let mut stack = vec![seed];
        membership[seed] = id;
        while let Some(p) = stack.pop() {
            areas[id] += 1;
            for n in neighbors(p).into_iter().flatten() {
                if membership[n] == usize::MAX && indexed.labels[n] == label {
                    membership[n] = id;
                    stack.push(n);
                }
            }
        }
    }
    let mut contacts = HashMap::<usize, HashMap<usize, usize>>::new();
    for p in 0..w * h {
        let id = membership[p];
        if areas[id] > max_area || omit[labels[id] as usize] {
            continue;
        }
        for n in neighbors(p).into_iter().flatten() {
            let target = membership[n];
            if target != id && areas[target] > max_area {
                *contacts.entry(id).or_default().entry(target).or_default() += 1;
            }
        }
    }
    let mut replacement = labels.clone();
    for (id, adjacent) in contacts {
        if let Some((&target, _)) = adjacent.iter().min_by_key(|&(&target, &count)| {
            (
                std::cmp::Reverse(count),
                distance2(
                    indexed.palette[labels[id] as usize],
                    indexed.palette[labels[target] as usize],
                ) as u32,
                target,
            )
        }) {
            replacement[id] = labels[target];
        }
    }
    for (p, label) in indexed.labels.iter_mut().enumerate() {
        *label = replacement[membership[p]];
    }
}

/// Merge color regions before extracting geometry. Every foreground connected
/// component is an individual closed path; its holes remain in the same path.
/// Outline mode removes paint, not internal boundaries: use silhouette explicitly
/// when all internal color structure (including gradients) should be discarded.
pub fn refine(
    img: &RgbaImage,
    options: &Options,
    settings: &RefineOptions,
) -> Result<RefinedSvg, Error> {
    let (indexed, omit, before) = prepare(img, options, settings)?;
    let smooth = if settings.geometry {
        Some(crate::smooth::SmoothPaths::with_geometry(
            &indexed,
            settings.smooth,
            true,
            settings.geometry_tolerance,
            settings.corner_extension,
        ))
    } else if settings.smooth > 0.0 {
        Some(crate::smooth::SmoothPaths::new(&indexed, settings.smooth))
    } else {
        None
    };
    let w = indexed.width as usize;
    let h = indexed.height as usize;
    let mut membership = vec![usize::MAX; w * h];
    let mut used_colors = vec![false; indexed.palette.len()];
    let mut stats = RefineStats {
        geometry_circles: smooth.as_ref().map_or(0, |s| s.circles),
        geometry_polygons: smooth.as_ref().map_or(0, |s| s.polygons),
        palette_before: before,
        palette_after: 0,
        paths: 0,
        contours: 0,
        background_pixels: 0,
    };
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" viewBox=\"0 0 {w} {h}\">\n"
    );
    for seed in 0..w * h {
        let label = indexed.labels[seed] as usize;
        if omit[label] {
            stats.background_pixels += 1;
            continue;
        }
        if membership[seed] != usize::MAX {
            continue;
        }
        let id = stats.paths;
        membership[seed] = id;
        let mut stack = vec![seed];
        let (mut left, mut right, mut top, mut bottom) = (seed % w, seed % w, seed / w, seed / w);
        while let Some(p) = stack.pop() {
            let (x, y) = (p % w, p / w);
            left = left.min(x);
            right = right.max(x);
            top = top.min(y);
            bottom = bottom.max(y);
            for neighbor in [
                x.checked_sub(1).map(|_| p - 1),
                (x + 1 < w).then_some(p + 1),
                y.checked_sub(1).map(|_| p - w),
                (y + 1 < h).then_some(p + w),
            ]
            .into_iter()
            .flatten()
            {
                if membership[neighbor] == usize::MAX && indexed.labels[neighbor] as usize == label
                {
                    membership[neighbor] = id;
                    stack.push(neighbor);
                }
            }
        }
        let bw = right - left + 1;
        let bh = bottom - top + 1;
        let mut local = IndexedImage {
            width: bw as u32,
            height: bh as u32,
            palette: vec![[0; 3], [255; 3]],
            labels: vec![0; bw * bh],
        };
        for y in 0..bh {
            for x in 0..bw {
                local.labels[y * bw + x] = u8::from(membership[(top + y) * w + left + x] == id);
            }
        }
        let rings = contours(&local, 1);
        stats.contours += rings.len();
        used_colors[label] = true;
        let color = indexed.palette[label];
        write!(svg,"<path id=\"region-{:05}\" data-source-fill=\"#{:02x}{:02x}{:02x}\" fill-rule=\"evenodd\" ",id+1,color[0],color[1],color[2]).unwrap();
        match settings.paint {
            Paint::Flat => write!(
                svg,
                "fill=\"#{:02x}{:02x}{:02x}\" ",
                color[0], color[1], color[2]
            )
            .unwrap(),
            Paint::Outline => write!(
                svg,
                "fill=\"none\" stroke=\"#404040\" stroke-width=\"{}\" stroke-linejoin=\"round\" ",
                settings.stroke_width
            )
            .unwrap(),
        }
        svg.push_str("d=\"");
        for ring in rings {
            if let Some(smooth) = &smooth {
                let global: Vec<_> = ring
                    .iter()
                    .map(|&(x, y)| (x + left as i32, y + top as i32))
                    .collect();
                smooth.write_ring(&global, &mut svg);
                continue;
            }
            for (i, (x, y)) in simplify(ring, options.tolerance).iter().enumerate() {
                write!(
                    svg,
                    "{}{} {}",
                    if i == 0 { 'M' } else { 'L' },
                    x + left as i32,
                    y + top as i32
                )
                .unwrap();
            }
            svg.push('Z');
        }
        svg.push_str("\"/>\n");
        stats.paths += 1;
    }
    stats.palette_after = used_colors.into_iter().filter(|v| *v).count();
    svg.push_str("</svg>\n");
    Ok(RefinedSvg { svg, stats })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn merge_removes_internal_boundary_without_chain_collapse() {
        let img = RgbaImage::from_fn(3, 1, |x, _| Rgba([10 + x as u8 * 10, 0, 0, 255]));
        let settings = RefineOptions {
            merge_distance: 11.0,
            ..RefineOptions::default()
        };
        let result = refine(&img, &Options::default(), &settings).unwrap();
        assert_eq!(result.stats.paths, 2);
        assert_eq!(result.stats.palette_after, 2);
        let all = refine(
            &img,
            &Options::default(),
            &RefineOptions {
                merge_distance: 21.0,
                ..settings
            },
        )
        .unwrap();
        assert_eq!(all.stats.paths, 1);
        assert_eq!(all.stats.contours, 1);
    }
    #[test]
    fn fixed_palette_keeps_holes_and_individual_islands() {
        let img = RgbaImage::from_fn(7, 7, |x, y| {
            let black = (1..=5).contains(&x)
                && (1..=5).contains(&y)
                && (x == 1 || x == 5 || y == 1 || y == 5)
                || (x == 3 && y == 3);
            Rgba(if black {
                [10, 10, 10, 255]
            } else {
                [250, 250, 250, 255]
            })
        });
        let settings = RefineOptions {
            palette: vec![[0; 3], [255; 3]],
            background: Some([255; 3]),
            ..RefineOptions::default()
        };
        let flat = refine(&img, &Options::default(), &settings).unwrap();
        let outline = refine(
            &img,
            &Options::default(),
            &RefineOptions {
                paint: Paint::Outline,
                ..settings
            },
        )
        .unwrap();
        assert_eq!(flat.stats.paths, 2);
        assert_eq!(flat.stats.contours, 3);
        assert_eq!(flat.stats.background_pixels, 32);
        let paths = |svg: &str| {
            svg.split(" d=\"")
                .skip(1)
                .map(|s| s.split('"').next().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(paths(&flat.svg), paths(&outline.svg));
        assert_eq!(outline.svg.matches("fill=\"none\"").count(), 2);
    }
    #[test]
    fn silhouette_discards_gradient_bands_but_preserves_hole() {
        let img = RgbaImage::from_fn(10, 10, |x, y| {
            if x == 0 || x == 9 || y == 0 || y == 9 || (x == 5 && y == 5) {
                Rgba([255; 4])
            } else {
                Rgba([x as u8 * 20, 50, 70, 255])
            }
        });
        let settings = RefineOptions {
            silhouette: true,
            background: Some([255; 3]),
            paint: Paint::Outline,
            ..RefineOptions::default()
        };
        let result = refine(&img, &Options::default(), &settings).unwrap();
        assert_eq!(result.stats.paths, 1);
        assert_eq!(result.stats.contours, 2);
        assert!(
            refine(
                &img,
                &Options::default(),
                &RefineOptions {
                    silhouette: true,
                    ..RefineOptions::default()
                }
            )
            .is_err()
        );
        assert!(
            refine(
                &img,
                &Options::default(),
                &RefineOptions {
                    merge_distance: f64::NAN,
                    ..RefineOptions::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn diagonal_pixels_are_independent_shapes_and_empty_is_valid() {
        let img = RgbaImage::from_fn(2, 2, |x, y| {
            if x == y {
                Rgba([0, 0, 0, 255])
            } else {
                Rgba([255; 4])
            }
        });
        let settings = RefineOptions {
            background: Some([255; 3]),
            ..RefineOptions::default()
        };
        assert_eq!(
            refine(&img, &Options::default(), &settings)
                .unwrap()
                .stats
                .paths,
            2
        );
        assert_eq!(
            refine(
                &RgbaImage::from_pixel(2, 2, Rgba([255; 4])),
                &Options::default(),
                &settings
            )
            .unwrap()
            .stats
            .paths,
            0
        );
    }
    #[test]
    fn speck_cleanup_preserves_background_hole_and_larger_dot() {
        let mut img = RgbaImage::from_pixel(12, 8, Rgba([255; 4]));
        for y in 1..7 {
            for x in 1..6 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        img.put_pixel(3, 3, Rgba([255; 4])); // must stay a hole, despite area=1
        img.put_pixel(10, 1, Rgba([0, 0, 0, 255])); // isolated speck
        for y in 4..6 {
            for x in 9..11 {
                img.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        } // 4px dot
        let settings = RefineOptions {
            min_region_area: 1,
            background: Some([255; 3]),
            ..RefineOptions::default()
        };
        let result = refine(&img, &Options::default(), &settings).unwrap();
        assert_eq!(result.stats.paths, 2);
        assert_eq!(result.stats.contours, 3);
        assert_eq!(
            result.svg,
            refine(&img, &Options::default(), &settings).unwrap().svg
        );
    }
}
