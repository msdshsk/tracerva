//! Shared-boundary smoothing followed by least-squares cubic fitting.
//! Original implementation; inspired by digitized-curve fitting, not a port.
use crate::{IndexedImage, Point};
use std::{collections::HashMap, fmt::Write};

type P = [f64; 2];
fn add(a: P, b: P) -> P {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: P, s: f64) -> P {
    [a[0] * s, a[1] * s]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn unit(a: P) -> P {
    let n = norm(a);
    if n > 1e-12 {
        mul(a, 1.0 / n)
    } else {
        [1.0, 0.0]
    }
}
fn point(p: Point) -> P {
    [p.0 as f64, p.1 as f64]
}
fn key(a: Point, b: Point) -> (Point, Point) {
    if a < b { (a, b) } else { (b, a) }
}

#[derive(Clone, Debug)]
enum Segment {
    Line(P, P),
    Cubic([P; 4]),
    Arc {
        start: P,
        end: P,
        radius: f64,
        sweep: bool,
    },
}
impl Segment {
    fn reverse(&self) -> Self {
        match self {
            Self::Arc {
                start,
                end,
                radius,
                sweep,
            } => Self::Arc {
                start: *end,
                end: *start,
                radius: *radius,
                sweep: !*sweep,
            },
            Self::Line(a, b) => Self::Line(*b, *a),
            Self::Cubic(p) => Self::Cubic([p[3], p[2], p[1], p[0]]),
        }
    }
    fn start(&self) -> P {
        match self {
            Self::Arc { start, .. } => *start,
            Self::Line(a, _) => *a,
            Self::Cubic(p) => p[0],
        }
    }
    fn write(&self, svg: &mut String) {
        match self {
            Self::Arc {
                end, radius, sweep, ..
            } => write!(
                svg,
                "A{:.4} {:.4} 0 0 {} {:.4} {:.4}",
                radius,
                radius,
                u8::from(*sweep),
                end[0],
                end[1]
            )
            .unwrap(),
            Self::Line(_, b) => write!(svg, "L{:.4} {:.4}", b[0], b[1]).unwrap(),
            Self::Cubic(p) => write!(
                svg,
                "C{:.4} {:.4} {:.4} {:.4} {:.4} {:.4}",
                p[1][0], p[1][1], p[2][0], p[2][1], p[3][0], p[3][1]
            )
            .unwrap(),
        }
    }
}
struct Chain {
    points: Vec<Point>,
    segments: Vec<Segment>,
}
pub(crate) struct SmoothPaths {
    pub(crate) circles: usize,
    pub(crate) polygons: usize,
    chains: Vec<Chain>,
    edges: HashMap<(Point, Point), (usize, usize)>,
}
impl SmoothPaths {
    pub(crate) fn new(image: &IndexedImage, strength: f64) -> Self {
        Self::with_geometry(image, strength, false, 1.0, 3.0)
    }
    pub(crate) fn with_geometry(
        image: &IndexedImage,
        strength: f64,
        geometry: bool,
        tolerance: f64,
        extension: f64,
    ) -> Self {
        let (w, h) = (image.width as i32, image.height as i32);
        let label = |x: i32, y: i32| -> i16 {
            if x < 0 || y < 0 || x >= w || y >= h {
                -1
            } else {
                image.labels[(y * w + x) as usize] as i16
            }
        };
        let mut edges = Vec::new();
        for y in 0..=h {
            for x in 0..w {
                if label(x, y - 1) != label(x, y) {
                    edges.push(((x, y), (x + 1, y)));
                }
            }
        }
        for x in 0..=w {
            for y in 0..h {
                if label(x - 1, y) != label(x, y) {
                    edges.push(((x, y), (x, y + 1)));
                }
            }
        }
        let mut adjacent = HashMap::<Point, Vec<usize>>::new();
        for (i, (a, b)) in edges.iter().enumerate() {
            adjacent.entry(*a).or_default().push(i);
            adjacent.entry(*b).or_default().push(i);
        }
        let mut used = vec![false; edges.len()];
        let mut result = Self {
            circles: 0,
            polygons: 0,
            chains: vec![],
            edges: HashMap::new(),
        };
        // Junction-to-junction chains first, then remaining closed loops.
        for closed in [false, true] {
            for seed in 0..edges.len() {
                if used[seed] {
                    continue;
                }
                let (a, b) = edges[seed];
                let start = if adjacent[&a].len() != 2 {
                    a
                } else if adjacent[&b].len() != 2 {
                    b
                } else if closed {
                    a
                } else {
                    continue;
                };
                let mut points = vec![start];
                let mut current = start;
                let mut edge = seed;
                loop {
                    used[edge] = true;
                    let (a, b) = edges[edge];
                    let next = if a == current { b } else { a };
                    points.push(next);
                    if next == start || adjacent[&next].len() != 2 {
                        break;
                    }
                    let Some(&e) = adjacent[&next].iter().find(|&&e| !used[e]) else {
                        break;
                    };
                    current = next;
                    edge = e;
                }
                let id = result.chains.len();
                for (i, pair) in points.windows(2).enumerate() {
                    result.edges.insert(key(pair[0], pair[1]), (id, i));
                }
                let primitive = if geometry
                    && points.first() == points.last()
                    && adjacent[&points[0]].len() == 2
                    && points
                        .iter()
                        .all(|&(x, y)| x > 0 && y > 0 && x < w && y < h)
                {
                    crate::geometry::recognize(&points, tolerance, extension)
                } else {
                    None
                };
                let segments = match primitive {
                    Some(crate::geometry::Primitive::Circle { center, radius }) => {
                        result.circles += 1;
                        let a = [center[0] + radius, center[1]];
                        let b = [center[0] - radius, center[1]];
                        vec![
                            Segment::Arc {
                                start: a,
                                end: b,
                                radius,
                                sweep: true,
                            },
                            Segment::Arc {
                                start: b,
                                end: a,
                                radius,
                                sweep: true,
                            },
                        ]
                    }
                    Some(crate::geometry::Primitive::Polygon(vertices)) => {
                        result.polygons += 1;
                        (0..vertices.len())
                            .map(|i| Segment::Line(vertices[i], vertices[(i + 1) % vertices.len()]))
                            .collect()
                    }
                    None if strength == 0.0 => points
                        .windows(2)
                        .map(|w| Segment::Line(point(w[0]), point(w[1])))
                        .collect(),
                    None => fit_chain(&points, strength),
                };
                result.chains.push(Chain { points, segments });
            }
        }
        result
    }
    pub(crate) fn write_ring(&self, ring: &[Point], svg: &mut String) {
        let mut units = Vec::new();
        for i in 0..ring.len() {
            let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
            let step = ((b.0 - a.0).signum(), (b.1 - a.1).signum());
            let mut p = a;
            while p != b {
                units.push(p);
                p = (p.0 + step.0, p.1 + step.1);
            }
        }
        let n = units.len();
        let start = (0..n)
            .find(|&i| {
                let (id, k) = self.edges[&key(units[i], units[(i + 1) % n])];
                let c = &self.chains[id];
                (units[i] == c.points[k] && k == 0)
                    || (units[i] != c.points[k] && k + 2 == c.points.len())
            })
            .expect("ring contains a chain endpoint");
        let mut consumed = 0;
        while consumed < n {
            let i = (start + consumed) % n;
            let (id, k) = self.edges[&key(units[i], units[(i + 1) % n])];
            let c = &self.chains[id];
            let forward = units[i] == c.points[k];
            let segments: Vec<_> = if forward {
                c.segments.clone()
            } else {
                c.segments.iter().rev().map(Segment::reverse).collect()
            };
            if consumed == 0 {
                let p = segments[0].start();
                write!(svg, "M{:.4} {:.4}", p[0], p[1]).unwrap();
            }
            for segment in segments {
                segment.write(svg);
            }
            consumed += c.points.len() - 1;
        }
        debug_assert_eq!(consumed, n);
        svg.push('Z');
    }
}

fn angle(a: P, b: P) -> f64 {
    dot(unit(a), unit(b)).clamp(-1.0, 1.0).acos()
}
fn fit_chain(raw: &[Point], strength: f64) -> Vec<Segment> {
    let closed = raw.first() == raw.last();
    let n = raw.len() - usize::from(closed);
    let p: Vec<_> = raw[..n].iter().copied().map(point).collect();
    // Very small geometry remains exact rather than being collapsed by smoothing.
    if raw.len() < 17 {
        return raw
            .windows(2)
            .map(|p| Segment::Line(point(p[0]), point(p[1])))
            .collect();
    }
    let at = |i: isize| {
        p[if closed {
            i.rem_euclid(n as isize) as usize
        } else {
            i.clamp(0, n as isize - 1) as usize
        }]
    };
    let mut scores = vec![0.0; n];
    let mut forced = Vec::new();
    for i in 0..n {
        if !closed && (i == 0 || i + 1 == n) {
            continue;
        }
        let i0 = i as isize;
        let incoming = sub(at(i0), at(i0 - 1));
        let outgoing = sub(at(i0 + 1), at(i0));
        if incoming == outgoing {
            continue;
        }
        let mut before = 1isize;
        let mut after = 1isize;
        while before < n as isize && sub(at(i0 - before), at(i0 - before - 1)) == incoming {
            before += 1;
        }
        while after < n as isize && sub(at(i0 + after + 1), at(i0 + after)) == outgoing {
            after += 1;
        }
        // Long orthogonal runs and narrow U-turn end caps are deliberate corners.
        // Single-pixel stair steps have parallel, rather than opposing, outer runs.
        let end_cap = (before <= 2
            && after >= 3
            && dot(sub(at(i0 - before), at(i0 - before - 1)), outgoing) < -0.5)
            || (after <= 2
                && before >= 3
                && dot(incoming, sub(at(i0 + after + 1), at(i0 + after))) < -0.5);
        if (before >= 3 && after >= 3) || end_cap {
            forced.push(i);
        }
    }
    for (i, score) in scores.iter_mut().enumerate() {
        if !closed && (i < 6 || i + 6 >= n) {
            continue;
        }
        let i = i as isize;
        let near = angle(sub(at(i), at(i - 3)), sub(at(i + 3), at(i)));
        let far = angle(sub(at(i), at(i - 6)), sub(at(i + 6), at(i)));
        // Curvature grows with the observation window; a persistent sharp corner does not.
        if near > 60f64.to_radians() && far >= near * 0.8 && far <= near * 1.45 {
            *score = near;
        }
    }
    let mut cuts: Vec<usize> = (0..n)
        .filter(|&i| {
            scores[i] > 0.0
                && (1..=3).all(|d| {
                    let before = if closed {
                        (i + n - d) % n
                    } else {
                        i.saturating_sub(d)
                    };
                    let after = if closed {
                        (i + d) % n
                    } else {
                        (i + d).min(n - 1)
                    };
                    (scores[i] > scores[before] || (scores[i] == scores[before] && i < before))
                        && (scores[i] > scores[after] || (scores[i] == scores[after] && i < after))
                })
        })
        .collect();
    cuts.extend(forced);
    cuts.sort_unstable();
    cuts.dedup();
    let mut is_corner = vec![false; n];
    for &i in &cuts {
        is_corner[i] = true;
    }
    if closed {
        if cuts.is_empty() {
            cuts.push(0);
        }
        if cuts.len() == 1 {
            cuts.push((cuts[0] + n / 2) % n);
            cuts.sort_unstable();
        }
    } else {
        cuts.insert(0, 0);
        cuts.push(n - 1);
    }
    let radius = ((strength * 2.5).ceil() as isize).min(n as isize);
    let mut smooth = p.clone();
    for (i, out) in smooth.iter_mut().enumerate() {
        if is_corner[i] || (!closed && (i == 0 || i == n - 1)) {
            continue;
        }
        // Only corners inside the finite kernel can affect this sample.
        let r = (1..=radius)
            .find(|&d| {
                let d = d as usize;
                if closed {
                    is_corner[(i + n - d % n) % n] || is_corner[(i + d) % n]
                } else {
                    (i >= d && is_corner[i - d]) || (i + d < n && is_corner[i + d])
                }
            })
            .unwrap_or(radius);
        let mut total = 0.0;
        let mut sum = [0.0; 2];
        for d in -r..=r {
            let weight = (-0.5 * (d as f64 / strength).powi(2)).exp();
            sum = add(sum, mul(at(i as isize + d), weight));
            total += weight;
        }
        *out = mul(sum, 1.0 / total);
    }
    let tangent = |i: usize| unit(sub(smooth[(i + 1) % n], smooth[(i + n - 1) % n]));
    let mut result = Vec::new();
    for j in 0..if closed { cuts.len() } else { cuts.len() - 1 } {
        let a = cuts[j];
        let b = cuts[(j + 1) % cuts.len()];
        let len = if b > a { b - a } else { n - a + b };
        let points: Vec<_> = (0..=len).map(|k| smooth[(a + k) % n]).collect();
        let left = if is_corner[a] || !closed {
            unit(sub(points[1], points[0]))
        } else {
            tangent(a)
        };
        let right = if is_corner[b] || !closed {
            unit(sub(points[len - 1], points[len]))
        } else {
            mul(tangent(b), -1.0)
        };
        fit(
            &points,
            left,
            right,
            (strength * 0.45).max(0.2),
            &mut result,
        );
    }
    result
}
fn bezier(p: [P; 4], t: f64) -> P {
    let s = 1.0 - t;
    add(
        add(mul(p[0], s * s * s), mul(p[1], 3.0 * s * s * t)),
        add(mul(p[2], 3.0 * s * t * t), mul(p[3], t * t * t)),
    )
}
fn fit(points: &[P], left: P, right: P, error: f64, out: &mut Vec<Segment>) {
    // Explicit stack bounds recursion even on adversarial contours.
    let mut stack = vec![(0, points.len() - 1, left, right)];
    while let Some((start, end, left, right)) = stack.pop() {
        let p = &points[start..=end];
        let first = p[0];
        let last = p[p.len() - 1];
        let mut u = vec![0.0; p.len()];
        for i in 1..p.len() {
            u[i] = u[i - 1] + norm(sub(p[i], p[i - 1]));
        }
        let length = u[p.len() - 1];
        if length < 1e-10 {
            out.push(Segment::Line(first, last));
            continue;
        }
        for t in &mut u {
            *t /= length;
        }
        // Straight intervals stay straight, avoiding needless control points.
        let delta = sub(last, first);
        let len2 = dot(delta, delta);
        if p.iter()
            .zip(&u)
            .all(|(&q, &t)| norm(sub(q, add(first, mul(delta, t)))) <= error * 0.5)
        {
            out.push(Segment::Line(first, last));
            continue;
        }
        let (mut c00, mut c01, mut c11, mut x0, mut x1) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (&q, &t) in p.iter().zip(&u) {
            let s = 1.0 - t;
            let b1 = 3.0 * s * s * t;
            let b2 = 3.0 * s * t * t;
            let a = mul(left, b1);
            let b = mul(right, b2);
            let residual = sub(
                q,
                add(mul(first, s * s * s + b1), mul(last, t * t * t + b2)),
            );
            c00 += dot(a, a);
            c01 += dot(a, b);
            c11 += dot(b, b);
            x0 += dot(a, residual);
            x1 += dot(b, residual);
        }
        let det = c00 * c11 - c01 * c01;
        let (mut a, mut b) = if det.abs() > 1e-12 {
            ((x0 * c11 - x1 * c01) / det, (x1 * c00 - x0 * c01) / det)
        } else {
            (length / 3.0, length / 3.0)
        };
        if a <= 1e-6 || b <= 1e-6 || a > length || b > length {
            a = length / 3.0;
            b = a;
        }
        let curve = [
            first,
            add(first, mul(left, a)),
            add(last, mul(right, b)),
            last,
        ];
        let (split, max) = p
            .iter()
            .zip(&u)
            .enumerate()
            .skip(1)
            .take(p.len() - 2)
            .map(|(i, (&q, &t))| (i, norm(sub(q, bezier(curve, t)))))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((1, 0.0));
        if max <= error && len2 > 1e-12 {
            out.push(Segment::Cubic(curve));
        } else if p.len() <= 2 {
            out.push(Segment::Line(first, last));
        } else {
            let middle = unit(sub(p[split + 1], p[split - 1]));
            let split = start + split;
            stack.push((split, end, middle, right));
            stack.push((start, split, left, mul(middle, -1.0)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_small_mask_can_reassemble_shared_chains() {
        for mask in 0..512u32 {
            let image = IndexedImage {
                width: 3,
                height: 3,
                palette: vec![[0; 3], [255; 3]],
                labels: (0..9).map(|i| ((mask >> i) & 1) as u8).collect(),
            };
            let smooth = SmoothPaths::new(&image, 1.2);
            for label in [0, 1] {
                for ring in crate::contours(&image, label) {
                    let mut svg = String::new();
                    smooth.write_ring(&ring, &mut svg);
                    assert!(svg.starts_with('M') && svg.ends_with('Z'));
                    assert!(!svg.contains("NaN"));
                }
            }
        }
    }
    #[test]
    fn curved_boundary_is_identical_in_both_directions() {
        let image = IndexedImage {
            width: 64,
            height: 64,
            palette: vec![[0; 3], [255; 3]],
            labels: (0..4096)
                .map(|i| u8::from(i % 64 > (32.0 + 12.0 * ((i / 64) as f64 / 10.0).sin()) as usize))
                .collect(),
        };
        let smooth = SmoothPaths::new(&image, 1.2);
        let chain = smooth
            .chains
            .iter()
            .find(|c| c.points.len() > 64 && c.points.iter().all(|p| p.0 > 0 && p.0 < 64))
            .unwrap();
        let curve = chain
            .segments
            .iter()
            .find(|s| matches!(s, Segment::Cubic(_)))
            .unwrap();
        let mut forward = String::new();
        curve.write(&mut forward);
        let mut reverse = String::new();
        curve.reverse().write(&mut reverse);
        let mut outputs = Vec::new();
        for label in [0, 1] {
            let mut svg = String::new();
            for ring in crate::contours(&image, label) {
                smooth.write_ring(&ring, &mut svg);
            }
            outputs.push(svg);
        }
        assert!(
            (outputs[0].contains(&forward) && outputs[1].contains(&reverse))
                || (outputs[1].contains(&forward) && outputs[0].contains(&reverse))
        );
    }
    #[test]
    fn square_corners_and_shared_edges() {
        let image = IndexedImage {
            width: 32,
            height: 32,
            palette: vec![[0; 3], [255; 3]],
            labels: (0..1024).map(|i| u8::from(i % 32 >= 16)).collect(),
        };
        let smooth = SmoothPaths::new(&image, 1.2);
        for label in [0, 1] {
            let ring = crate::contours(&image, label).remove(0);
            let mut s = String::new();
            smooth.write_ring(&ring, &mut s);
            assert!(!s.contains('C'), "{s}");
        }
        let shared = smooth
            .chains
            .iter()
            .find(|c| c.points.iter().all(|p| p.0 == 16))
            .unwrap();
        assert_eq!(shared.segments.len(), 1);
    }
    #[test]
    fn circle_is_curved_and_radial_error_is_small() {
        let image = IndexedImage {
            width: 64,
            height: 64,
            palette: vec![[0; 3], [255; 3]],
            labels: (0..4096)
                .map(|i| {
                    u8::from(
                        ((i % 64) as f64 + 0.5 - 32.0).hypot((i / 64) as f64 + 0.5 - 32.0) < 22.0,
                    )
                })
                .collect(),
        };
        let smooth = SmoothPaths::new(&image, 1.2);
        let c = smooth
            .chains
            .iter()
            .find(|c| {
                c.points
                    .iter()
                    .all(|&(x, y)| x > 0 && x < 64 && y > 0 && y < 64)
            })
            .unwrap();
        assert!(c.segments.iter().any(|s| matches!(s, Segment::Cubic(_))));
        assert!(c.segments.len() < 40, "{}", c.segments.len());
        for segment in &c.segments {
            for i in 0..=30 {
                let t = i as f64 / 30.0;
                let p = match segment {
                    Segment::Line(a, b) => add(*a, mul(sub(*b, *a), t)),
                    Segment::Cubic(p) => bezier(*p, t),
                    Segment::Arc { .. } => panic!("geometry is disabled"),
                };
                assert!((norm(sub(p, [32.0, 32.0])) - 22.0).abs() < 1.0, "{p:?}");
            }
        }
    }
}
