//! Deterministic experimental color tracing. No file I/O or external processes.
//! Alpha is explicitly composited onto a configurable matte before quantization.
//! `trace` uses polygonal pixel boundaries; `refine` optionally fits shared curves.
use image::{Rgb, RgbImage, RgbaImage};
use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

mod geometry;
mod refine;
mod smooth;
pub use refine::{Paint, RefineOptions, RefineStats, RefinedSvg, refine};

#[derive(Clone, Debug)]
pub struct Options {
    pub colors: usize,
    /// RDP tolerance in source pixels. Zero preserves the exact pixel boundary.
    /// Positive values may alter topology or open seams between adjacent colors.
    pub tolerance: f64,
    pub matte: [u8; 3],
}
impl Default for Options {
    fn default() -> Self {
        Self {
            colors: 16,
            tolerance: 0.0,
            matte: [255; 3],
        }
    }
}
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("image must be nonempty and at most 16 million pixels")]
    Dimensions,
    #[error("colors must be 1..=256 and tolerance must be finite and nonnegative")]
    Options,
    #[error("invalid refinement settings: {0}")]
    Refine(&'static str),
}
#[derive(Clone, Debug)]
pub struct IndexedImage {
    pub width: u32,
    pub height: u32,
    pub palette: Vec<[u8; 3]>,
    pub labels: Vec<u8>,
}
impl IndexedImage {
    pub fn to_rgb(&self) -> RgbImage {
        RgbImage::from_fn(self.width, self.height, |x, y| {
            Rgb(self.palette[self.labels[(y * self.width + x) as usize] as usize])
        })
    }
}
pub fn flatten(img: &RgbaImage, matte: [u8; 3]) -> RgbImage {
    RgbImage::from_fn(img.width(), img.height(), |x, y| {
        let p = img.get_pixel(x, y).0;
        let a = p[3] as u32;
        Rgb(std::array::from_fn(|c| {
            ((p[c] as u32 * a + matte[c] as u32 * (255 - a) + 127) / 255) as u8
        }))
    })
}
/// Weighted median-cut with deterministic ordering and nearest-palette assignment.
pub fn quantize(img: &RgbaImage, options: &Options) -> Result<IndexedImage, Error> {
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
    let rgb = flatten(img, options.matte);
    let mut hist = BTreeMap::<[u8; 3], u64>::new();
    for p in rgb.pixels() {
        *hist.entry(p.0).or_default() += 1;
    }
    let entries: Vec<_> = hist.into_iter().collect();
    let mut boxes = vec![entries];
    fn range(b: &[([u8; 3], u64)]) -> (u8, usize) {
        (0..3)
            .map(|c| {
                (
                    b.iter().map(|p| p.0[c]).max().unwrap()
                        - b.iter().map(|p| p.0[c]).min().unwrap(),
                    c,
                )
            })
            .max()
            .unwrap()
    }
    while boxes.len() < options.colors {
        let candidate = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .max_by_key(|(_, b)| {
                let (r, _) = range(b);
                u64::from(r) * b.iter().map(|p| p.1).sum::<u64>()
            });
        let Some((i, _)) = candidate else {
            break;
        };
        let mut b = boxes.remove(i);
        let (_, axis) = range(&b);
        b.sort_by_key(|p| (p.0[axis], p.0));
        let half = b.iter().map(|p| p.1).sum::<u64>().div_ceil(2);
        let mut sum = 0;
        let mut split = 1;
        for (i, p) in b.iter().enumerate().take(b.len() - 1) {
            sum += p.1;
            split = i + 1;
            if sum >= half {
                break;
            }
        }
        let tail = b.split_off(split);
        boxes.push(b);
        boxes.push(tail);
    }
    let mut palette: Vec<[u8; 3]> = boxes
        .iter()
        .map(|b| {
            let count = b.iter().map(|p| p.1).sum::<u64>();
            std::array::from_fn(|c| {
                ((b.iter().map(|p| u64::from(p.0[c]) * p.1).sum::<u64>() + count / 2) / count) as u8
            })
        })
        .collect();
    palette.sort();
    palette.dedup();
    let mut cache = HashMap::new();
    let labels = rgb
        .pixels()
        .map(|p| {
            *cache.entry(p.0).or_insert_with(|| {
                palette
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, q)| {
                        (0..3)
                            .map(|c| (i32::from(p.0[c]) - i32::from(q[c])).pow(2))
                            .sum::<i32>()
                    })
                    .unwrap()
                    .0 as u8
            })
        })
        .collect();
    Ok(IndexedImage {
        width: img.width(),
        height: img.height(),
        palette,
        labels,
    })
}
type Point = (i32, i32);
#[derive(Clone, Copy)]
struct Edge {
    a: Point,
    b: Point,
    dir: u8,
}
fn contours(indexed: &IndexedImage, label: u8) -> Vec<Vec<Point>> {
    let w = indexed.width as i32;
    let h = indexed.height as i32;
    let is = |x: i32, y: i32| {
        x >= 0 && y >= 0 && x < w && y < h && indexed.labels[(y * w + x) as usize] == label
    };
    let mut edges = Vec::new();
    for y in 0..h {
        for x in 0..w {
            if is(x, y) {
                if !is(x, y - 1) {
                    edges.push(Edge {
                        a: (x, y),
                        b: (x + 1, y),
                        dir: 0,
                    });
                }
                if !is(x + 1, y) {
                    edges.push(Edge {
                        a: (x + 1, y),
                        b: (x + 1, y + 1),
                        dir: 1,
                    });
                }
                if !is(x, y + 1) {
                    edges.push(Edge {
                        a: (x + 1, y + 1),
                        b: (x, y + 1),
                        dir: 2,
                    });
                }
                if !is(x - 1, y) {
                    edges.push(Edge {
                        a: (x, y + 1),
                        b: (x, y),
                        dir: 3,
                    });
                }
            }
        }
    }
    let mut outgoing = HashMap::<Point, Vec<usize>>::new();
    for (i, e) in edges.iter().enumerate() {
        outgoing.entry(e.a).or_default().push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut rings = Vec::new();
    for start in 0..edges.len() {
        if !used[start] {
            let mut points = Vec::new();
            let mut i = start;
            loop {
                used[i] = true;
                let e = edges[i];
                points.push(e.a);
                if e.b == edges[start].a {
                    break;
                }
                // Right turn first separates regions touching only at a corner.
                i = *outgoing[&e.b]
                    .iter()
                    .filter(|&&j| !used[j])
                    .min_by_key(|&&j| match (edges[j].dir + 4 - e.dir) % 4 {
                        1 => 0,
                        0 => 1,
                        3 => 2,
                        _ => 3,
                    })
                    .expect("closed boundary");
            }
            let n = points.len();
            let corners = (0..n)
                .filter_map(|j| {
                    let a = points[(j + n - 1) % n];
                    let b = points[j];
                    let c = points[(j + 1) % n];
                    ((b.0 - a.0) * (c.1 - b.1) != (b.1 - a.1) * (c.0 - b.0)).then_some(b)
                })
                .collect();
            rings.push(corners);
        }
    }
    rings
}
fn distance(p: Point, a: Point, b: Point) -> f64 {
    let (dx, dy) = ((b.0 - a.0) as f64, (b.1 - a.1) as f64);
    let (px, py) = ((p.0 - a.0) as f64, (p.1 - a.1) as f64);
    let t = if dx * dx + dy * dy == 0.0 {
        0.0
    } else {
        ((px * dx + py * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0)
    };
    (px - t * dx).hypot(py - t * dy)
}
fn rdp(p: &[Point], epsilon: f64) -> Vec<Point> {
    let mut keep = vec![false; p.len()];
    keep[0] = true;
    keep[p.len() - 1] = true;
    let mut stack = vec![(0, p.len() - 1)];
    while let Some((a, b)) = stack.pop() {
        if let Some((i, d)) = ((a + 1)..b)
            .map(|i| (i, distance(p[i], p[a], p[b])))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            && d > epsilon
        {
            keep[i] = true;
            stack.push((a, i));
            stack.push((i, b));
        }
    }
    p.iter()
        .enumerate()
        .filter_map(|(i, p)| keep[i].then_some(*p))
        .collect()
}
fn simplify(p: Vec<Point>, epsilon: f64) -> Vec<Point> {
    if epsilon == 0.0 || p.len() < 4 {
        return p;
    }
    let far = (1..p.len())
        .max_by_key(|&i| {
            let dx = i64::from(p[i].0 - p[0].0);
            let dy = i64::from(p[i].1 - p[0].1);
            dx * dx + dy * dy
        })
        .unwrap();
    let mut a = rdp(&p[..=far], epsilon);
    a.pop();
    let mut tail = p[far..].to_vec();
    tail.push(p[0]);
    let mut b = rdp(&tail, epsilon);
    b.pop();
    a.extend(b);
    if a.len() < 3 { p } else { a }
}
/// Quantize, extract closed contours including holes, and serialize true SVG paths.
pub fn trace(img: &RgbaImage, options: &Options) -> Result<String, Error> {
    let indexed = quantize(img, options)?;
    let mut svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{}\" height=\"{}\" viewBox=\"0 0 {} {}\">\n",
        indexed.width, indexed.height, indexed.width, indexed.height
    );
    for (label, color) in indexed.palette.iter().enumerate() {
        let rings = contours(&indexed, label as u8);
        if rings.is_empty() {
            continue;
        }
        write!(
            svg,
            "<path fill=\"#{:02x}{:02x}{:02x}\" fill-rule=\"evenodd\" d=\"",
            color[0], color[1], color[2]
        )
        .unwrap();
        for ring in rings {
            let ring = simplify(ring, options.tolerance);
            for (i, p) in ring.iter().enumerate() {
                write!(svg, "{}{} {}", if i == 0 { 'M' } else { 'L' }, p.0, p.1).unwrap();
            }
            svg.push('Z');
        }
        svg.push_str("\"/>\n");
    }
    svg.push_str("</svg>\n");
    Ok(svg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn invalid_and_alpha() {
        assert!(trace(&RgbaImage::new(0, 1), &Options::default()).is_err());
        let img = RgbaImage::from_pixel(1, 1, Rgba([200, 10, 20, 0]));
        assert_eq!(
            quantize(&img, &Options::default()).unwrap().palette,
            vec![[255; 3]]
        );
        assert!(
            trace(
                &img,
                &Options {
                    colors: 0,
                    ..Options::default()
                }
            )
            .is_err()
        );
        assert!(
            trace(
                &img,
                &Options {
                    tolerance: f64::NAN,
                    ..Options::default()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn all_small_masks_preserve_area_and_pixel_membership() {
        for mask in 0..512u32 {
            let idx = IndexedImage {
                width: 3,
                height: 3,
                palette: vec![[0; 3], [255; 3]],
                labels: (0..9).map(|i| ((mask >> i) & 1) as u8).collect(),
            };
            let rings = contours(&idx, 1);
            let area: i32 = rings
                .iter()
                .map(|r| {
                    (0..r.len())
                        .map(|i| {
                            let a = r[i];
                            let b = r[(i + 1) % r.len()];
                            a.0 * b.1 - b.0 * a.1
                        })
                        .sum::<i32>()
                })
                .sum();
            assert_eq!(area, 2 * mask.count_ones() as i32, "mask {mask}");
            for y in 0..3 {
                for x in 0..3 {
                    let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
                    let mut inside = false;
                    for r in &rings {
                        for i in 0..r.len() {
                            let a = r[i];
                            let b = r[(i + 1) % r.len()];
                            if (a.1 as f64 > py) != (b.1 as f64 > py)
                                && px
                                    < (b.0 - a.0) as f64 * (py - a.1 as f64) / (b.1 - a.1) as f64
                                        + a.0 as f64
                            {
                                inside = !inside;
                            }
                        }
                    }
                    assert_eq!(inside, idx.labels[y * 3 + x] == 1, "mask {mask} at {x},{y}");
                }
            }
        }
    }
    #[test]
    fn deterministic_and_palette_bound() {
        let img = RgbaImage::from_fn(32, 32, |x, y| Rgba([(x * 8) as u8, (y * 8) as u8, 80, 255]));
        let opt = Options {
            colors: 8,
            ..Options::default()
        };
        let q = quantize(&img, &opt).unwrap();
        assert!(q.palette.len() <= 8);
        assert_eq!(trace(&img, &opt).unwrap(), trace(&img, &opt).unwrap());
    }
    #[test]
    fn simplification_does_not_collapse_tiny_ring() {
        let p = vec![(0, 0), (1, 0), (1, 1), (0, 1)];
        assert_eq!(simplify(p.clone(), 100.0), p);
    }
}
