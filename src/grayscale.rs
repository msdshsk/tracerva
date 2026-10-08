//! Luminance segmentation followed by per-component color recovery.
use crate::{Error, IndexedImage, Options, RefineOptions, RefinedSvg, flatten};
use image::RgbaImage;

#[derive(Clone, Debug)]
pub struct GrayscaleOptions {
    /// Maximum number of luminance classes (2..=64). Not the output color count.
    pub levels: usize,
    /// Median-filter radius in source pixels (0..=3). May remove small details.
    pub denoise: u32,
    /// Merge components of at most this area into larger neighbors. Zero disables.
    pub min_region_area: usize,
}
impl Default for GrayscaleOptions {
    fn default() -> Self {
        Self {
            levels: 8,
            denoise: 2,
            min_region_area: 16,
        }
    }
}

/// Segment a denoised grayscale copy and recover a median RGB color separately
/// for each connected region, favoring its interior over antialiased boundaries.
/// Same-luminance color boundaries and gradients cannot be recovered by this mode.
/// Palette, RGB merging and silhouette settings must be disabled.
pub fn refine_grayscale(
    img: &RgbaImage,
    options: &Options,
    settings: &RefineOptions,
    gray: &GrayscaleOptions,
) -> Result<RefinedSvg, Error> {
    crate::refine::validate(img, options, settings)?;
    if !(2..=64).contains(&gray.levels) || gray.denoise > 3 {
        return Err(Error::Refine(
            "grayscale levels must be 2..=64, denoise radius 0..=3",
        ));
    }
    if !settings.palette.is_empty()
        || settings.merge_distance != 0.0
        || settings.silhouette
        || settings.min_region_area != 0
    {
        return Err(Error::Refine(
            "grayscale uses its own segmentation and cleanup; disable palette, RGB merging, silhouette and refine min-region-area",
        ));
    }
    let rgb = flatten(img, options.matte);
    let w = img.width() as usize;
    let h = img.height() as usize;
    // Integer approximation to Rec.709 luma on the encoded RGB channels.
    let mut values: Vec<u8> = rgb
        .pixels()
        .map(|p| {
            ((54 * u32::from(p[0]) + 183 * u32::from(p[1]) + 19 * u32::from(p[2]) + 128) >> 8) as u8
        })
        .collect();
    if gray.denoise > 0 {
        let input = values.clone();
        let r = gray.denoise as usize;
        let mut window = [0u8; 49];
        for y in 0..h {
            for x in 0..w {
                let mut count = 0;
                for ny in y.saturating_sub(r)..=(y + r).min(h - 1) {
                    for nx in x.saturating_sub(r)..=(x + r).min(w - 1) {
                        window[count] = input[ny * w + nx];
                        count += 1;
                    }
                }
                let (_, median, _) = window[..count].select_nth_unstable(count / 2);
                values[y * w + x] = *median;
            }
        }
    }
    let (map, mut palette) = luminance_palette(&values, gray.levels);
    let before = palette.len();
    let mut labels: Vec<u8> = values.iter().map(|&v| map[v as usize]).collect();
    let mut omit = vec![false; palette.len()];
    if let Some(background) = settings.background {
        let bg_label = palette.len() as u8;
        palette.push(background);
        omit.push(true);
        for (i, p) in rgb.pixels().enumerate() {
            let distance: f64 = (0..3)
                .map(|c| (f64::from(p[c]) - f64::from(background[c])).powi(2))
                .sum();
            if distance <= settings.background_distance.powi(2) {
                labels[i] = bg_label;
            }
        }
    }
    let mut indexed = IndexedImage {
        width: img.width(),
        height: img.height(),
        palette,
        labels,
    };
    if gray.min_region_area > 0 {
        crate::refine::merge_small_regions(&mut indexed, &omit, gray.min_region_area);
    }
    crate::refine::render_indexed(&indexed, &omit, before, options, settings, Some(&rgb))
}

// Exact one-dimensional weighted least-squares quantization of the 256-bin
// histogram. Dynamic programming avoids random initialization and empty clusters.
fn luminance_palette(values: &[u8], requested: usize) -> ([u8; 256], Vec<[u8; 3]>) {
    let mut hist = [0f64; 256];
    for &v in values {
        hist[v as usize] += 1.0;
    }
    let bins: Vec<usize> = (0..256).filter(|&v| hist[v] > 0.0).collect();
    let n = bins.len();
    let k = requested.min(n);
    let mut count = vec![0.0; n + 1];
    let mut sum = vec![0.0; n + 1];
    let mut square = vec![0.0; n + 1];
    for (i, &v) in bins.iter().enumerate() {
        count[i + 1] = count[i] + hist[v];
        sum[i + 1] = sum[i] + hist[v] * v as f64;
        square[i + 1] = square[i] + hist[v] * (v * v) as f64;
    }
    let cost = |a: usize, b: usize| {
        let s = sum[b] - sum[a];
        (square[b] - square[a] - s * s / (count[b] - count[a])).max(0.0)
    };
    let mut dp = vec![vec![f64::INFINITY; n + 1]; k + 1];
    let mut split = vec![vec![0usize; n + 1]; k + 1];
    dp[0][0] = 0.0;
    for g in 1..=k {
        for end in g..=n {
            for start in g - 1..end {
                let error = dp[g - 1][start] + cost(start, end);
                if error < dp[g][end] {
                    dp[g][end] = error;
                    split[g][end] = start;
                }
            }
        }
    }
    let mut groups = Vec::new();
    let mut end = n;
    for g in (1..=k).rev() {
        let start = split[g][end];
        groups.push((start, end));
        end = start;
    }
    groups.reverse();
    let mut map = [0u8; 256];
    let mut palette = Vec::new();
    for (label, (a, b)) in groups.into_iter().enumerate() {
        let center = ((sum[b] - sum[a]) / (count[b] - count[a])).round() as u8;
        palette.push([center; 3]);
        for &v in &bins[a..b] {
            map[v] = label as u8;
        }
    }
    (map, palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn recovers_color_per_component_not_per_gray_class() {
        // These two foreground colors both round to luma 39.
        let image = RgbaImage::from_fn(9, 3, |x, _| {
            Rgba(if x < 3 {
                [183, 0, 0, 255]
            } else if x > 5 {
                [0, 54, 0, 255]
            } else {
                [255; 4]
            })
        });
        let settings = RefineOptions {
            background: Some([255; 3]),
            ..Default::default()
        };
        let gray = GrayscaleOptions {
            levels: 2,
            denoise: 0,
            min_region_area: 0,
        };
        let result = refine_grayscale(&image, &Options::default(), &settings, &gray).unwrap();
        assert_eq!(result.stats.paths, 2);
        assert!(result.svg.contains("fill=\"#b70000\""));
        assert!(result.svg.contains("fill=\"#003600\""));
        assert_eq!(
            result.svg,
            refine_grayscale(&image, &Options::default(), &settings, &gray)
                .unwrap()
                .svg
        );
    }
    #[test]
    fn removes_isolated_noise_but_preserves_low_contrast_regions() {
        let mut image = RgbaImage::from_fn(40, 24, |x, y| {
            Rgba(if (3..13).contains(&x) && (3..21).contains(&y) {
                [40, 70, 90, 255]
            } else if (23..36).contains(&x) && (6..18).contains(&y) {
                [170, 195, 210, 255]
            } else {
                [210, 220, 230, 255]
            })
        });
        image.put_pixel(7, 10, Rgba([210, 220, 230, 255]));
        image.put_pixel(29, 12, Rgba([40, 70, 90, 255]));
        let result = refine_grayscale(
            &image,
            &Options::default(),
            &RefineOptions::default(),
            &GrayscaleOptions {
                levels: 3,
                denoise: 1,
                min_region_area: 0,
            },
        )
        .unwrap();
        assert_eq!(result.stats.paths, 3);
        assert_eq!(result.stats.palette_after, 3);
        assert!(result.svg.contains("fill=\"#28465a\""));
        assert!(result.svg.contains("fill=\"#aac3d2\""));
    }
    #[test]
    fn equal_luminance_touching_colors_are_an_explicit_limitation() {
        let image = RgbaImage::from_fn(8, 4, |x, _| {
            Rgba(if x < 4 {
                [183, 0, 0, 255]
            } else {
                [0, 54, 0, 255]
            })
        });
        let result = refine_grayscale(
            &image,
            &Options::default(),
            &RefineOptions::default(),
            &GrayscaleOptions::default(),
        )
        .unwrap();
        assert_eq!(result.stats.paths, 1);
    }
    #[test]
    fn rejects_conflicts_and_invalid_settings() {
        let image = RgbaImage::from_pixel(1, 1, Rgba([255; 4]));
        for levels in [0, 1, 65] {
            assert!(
                refine_grayscale(
                    &image,
                    &Options::default(),
                    &RefineOptions::default(),
                    &GrayscaleOptions {
                        levels,
                        ..Default::default()
                    }
                )
                .is_err()
            );
        }
        assert!(
            refine_grayscale(
                &image,
                &Options::default(),
                &RefineOptions {
                    merge_distance: 1.0,
                    ..Default::default()
                },
                &GrayscaleOptions::default()
            )
            .is_err()
        );
        assert!(
            refine_grayscale(
                &image,
                &Options::default(),
                &RefineOptions::default(),
                &GrayscaleOptions {
                    denoise: 4,
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
