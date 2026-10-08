//! Conservative primitive proposals from unsmoothed pixel boundaries.
use crate::Point;
type P = [f64; 2];
fn sub(a: P, b: P) -> P {
    [a[0] - b[0], a[1] - b[1]]
}
fn dot(a: P, b: P) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}
fn norm(a: P) -> f64 {
    dot(a, a).sqrt()
}
fn cross(a: P, b: P) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn p(a: Point) -> P {
    [a.0 as f64, a.1 as f64]
}
pub(crate) enum Primitive {
    Circle { center: P, radius: f64 },
    Polygon(Vec<P>),
}

pub(crate) fn recognize(raw: &[Point], tolerance: f64, extension: f64) -> Option<Primitive> {
    if raw.len() < 17 || raw.first() != raw.last() {
        return None;
    }
    let points: Vec<_> = raw[..raw.len() - 1].iter().copied().map(p).collect();
    circle(&points, tolerance).or_else(|| polygon(raw, tolerance, extension))
}
fn solve(mut a: [[f64; 4]; 3]) -> Option<P3> {
    for c in 0..3 {
        let row = (c..3).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        a.swap(c, row);
        let pivot = a[c][c];
        if pivot.abs() < 1e-10 {
            return None;
        }
        for value in &mut a[c][c..] {
            *value /= pivot;
        }
        let pivot_row = a[c];
        for (i, row) in a.iter_mut().enumerate() {
            if i != c {
                let f = row[c];
                for (value, coefficient) in row[c..].iter_mut().zip(&pivot_row[c..]) {
                    *value -= f * coefficient;
                }
            }
        }
    }
    Some([a[0][3], a[1][3], a[2][3]])
}
type P3 = [f64; 3];
fn circle(points: &[P], tolerance: f64) -> Option<Primitive> {
    let n = points.len() as f64;
    let mean = [
        points.iter().map(|p| p[0]).sum::<f64>() / n,
        points.iter().map(|p| p[1]).sum::<f64>() / n,
    ];
    let mut matrix = [[0.0; 4]; 3];
    for &q in points {
        let q = sub(q, mean);
        let v = [2.0 * q[0], 2.0 * q[1], 1.0];
        let z = dot(q, q);
        for i in 0..3 {
            for j in 0..3 {
                matrix[i][j] += v[i] * v[j];
            }
            matrix[i][3] += v[i] * z;
        }
    }
    let guess = solve(matrix)?;
    let mut center = [guess[0] + mean[0], guess[1] + mean[1]];
    let mut radius = points.iter().map(|&q| norm(sub(q, center))).sum::<f64>() / n;
    // Geometric (radial) least squares, initialized by the algebraic circle.
    for _ in 0..12 {
        let mut matrix = [[0.0; 4]; 3];
        for &q in points {
            let d = sub(center, q);
            let length = norm(d);
            if length < 1e-8 {
                return None;
            }
            let v = [d[0] / length, d[1] / length, -1.0];
            let residual = length - radius;
            for i in 0..3 {
                for j in 0..3 {
                    matrix[i][j] += v[i] * v[j];
                }
                matrix[i][3] -= v[i] * residual;
            }
        }
        let delta = solve(matrix)?;
        center[0] += delta[0];
        center[1] += delta[1];
        radius += delta[2];
        if delta.iter().map(|x| x * x).sum::<f64>() < 1e-14 {
            break;
        }
    }
    if !radius.is_finite() || radius < 3.0 {
        return None;
    }
    let errors: Vec<_> = points
        .iter()
        .map(|&q| (norm(sub(q, center)) - radius).abs())
        .collect();
    let rms = (errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt();
    // Relative gate prevents large, visibly elliptical shapes becoming circles.
    if rms > (tolerance * 0.5).min((radius * 0.02).max(0.45))
        || errors.iter().any(|&e| e > tolerance)
    {
        return None;
    }
    let area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(points.len())
        .map(|(&a, &b)| cross(sub(a, center), sub(b, center)))
        .sum::<f64>()
        .abs()
        * 0.5;
    if (area / (std::f64::consts::PI * radius * radius) - 1.0).abs() > 0.08 {
        return None;
    }
    Some(Primitive::Circle { center, radius })
}
#[derive(Clone, Copy)]
struct Line {
    center: P,
    direction: P,
    length: f64,
}
fn line(points: &[P], tolerance: f64) -> Option<Line> {
    if points.len() < 4 {
        return None;
    }
    let n = points.len() as f64;
    let center = [
        points.iter().map(|p| p[0]).sum::<f64>() / n,
        points.iter().map(|p| p[1]).sum::<f64>() / n,
    ];
    let (mut xx, mut xy, mut yy) = (0.0, 0.0, 0.0);
    for &q in points {
        let d = sub(q, center);
        xx += d[0] * d[0];
        xy += d[0] * d[1];
        yy += d[1] * d[1];
    }
    let angle = 0.5 * (2.0 * xy).atan2(xx - yy);
    let direction = [angle.cos(), angle.sin()];
    let errors: Vec<_> = points
        .iter()
        .map(|&q| cross(sub(q, center), direction).abs())
        .collect();
    if errors.iter().any(|&e| e > tolerance)
        || (errors.iter().map(|e| e * e).sum::<f64>() / n).sqrt() > tolerance * 0.5
    {
        return None;
    }
    // Reject coherent bending, even when a short arc fits inside a narrow band.
    // Pixel staircases oscillate around a line; arcs bend to one side systematically.
    if points.len() >= 9 {
        let third = points.len() / 3;
        let average = |slice: &[P]| {
            slice
                .iter()
                .map(|&q| cross(sub(q, center), direction))
                .sum::<f64>()
                / slice.len() as f64
        };
        let bend = (average(&points[..third]) + average(&points[points.len() - third..])) * 0.5
            - average(&points[third..points.len() - third]);
        if bend.abs() > tolerance * 0.4 {
            return None;
        }
    }
    Some(Line {
        center,
        direction,
        length: norm(sub(*points.last()?, points[0])),
    })
}
fn intersection(a: Line, b: Line) -> Option<P> {
    let det = cross(a.direction, b.direction);
    if det.abs() < 0.15 {
        return None;
    }
    let t = cross(sub(b.center, a.center), b.direction) / det;
    Some([
        a.center[0] + t * a.direction[0],
        a.center[1] + t * a.direction[1],
    ])
}
fn polygon(raw: &[Point], tolerance: f64, extension: f64) -> Option<Primitive> {
    let n = raw.len() - 1;
    // Bound the quadratic bidirectional validation for very large contours.
    if n > 8192 {
        return None;
    }
    let mut coarse = crate::simplify(raw[..n].to_vec(), (tolerance * 1.75).max(1.25));
    // RDP's closed-ring anchor can lie in the middle of a genuine straight edge.
    loop {
        if coarse.len() <= 3 {
            break;
        }
        let count = coarse.len();
        let remove = (0..count).find(|&i| {
            let a = p(coarse[(i + count - 1) % count]);
            let b = p(coarse[i]);
            let c = p(coarse[(i + 1) % count]);
            let d = sub(c, a);
            let t = dot(sub(b, a), d) / dot(d, d).max(1e-12);
            (0.0..=1.0).contains(&t)
                && cross(sub(b, a), d).abs() / norm(d).max(1e-12) < tolerance * 0.7
        });
        if let Some(i) = remove {
            coarse.remove(i);
        } else {
            break;
        }
    }
    if !(3..=12).contains(&coarse.len()) {
        return None;
    }
    let mut lines = Vec::new();
    for i in 0..coarse.len() {
        let a = raw[..n].iter().position(|q| *q == coarse[i])?;
        let b = raw[..n]
            .iter()
            .position(|q| *q == coarse[(i + 1) % coarse.len()])?;
        let len = (b + n - a) % n;
        let full: Vec<_> = (0..=len).map(|k| p(raw[(a + k) % n])).collect();
        line(&full, tolerance * 1.5)?;
        let trim = if len >= 12 { 2 } else { 0 };
        let samples: Vec<_> = (trim..=len - trim).map(|k| p(raw[(a + k) % n])).collect();
        let fitted = line(&samples, tolerance);
        lines.push(fitted?);
    }
    // Remove only short bevel candidates, where two long supporting lines meet nearby.
    loop {
        let count = lines.len();
        if count <= 3 {
            break;
        }
        let candidate = (0..count).find(|&i| {
            let (a, b, c) = (
                lines[(i + count - 1) % count],
                lines[i],
                lines[(i + 1) % count],
            );
            b.length <= extension * 2.0
                && a.length > b.length * 3.0
                && c.length > b.length * 3.0
                && intersection(a, c).is_some_and(|q| norm(sub(q, b.center)) <= extension)
        });
        if let Some(i) = candidate {
            lines.remove(i);
        } else {
            break;
        }
    }
    if lines.len() > 8 {
        return None;
    }
    // A near-rectangle gets a common pair of perpendicular directions. Offsets
    // still come from its measured sides; subsequent proximity checks must pass.
    if lines.len() == 4
        && (0..4).all(|i| {
            dot(lines[i].direction, lines[(i + 1) % 4].direction).abs() < 2f64.to_radians().sin()
        })
    {
        let (mut c, mut s) = (0.0, 0.0);
        for line in &lines {
            let angle = line.direction[1].atan2(line.direction[0]);
            c += line.length * (4.0 * angle).cos();
            s += line.length * (4.0 * angle).sin();
        }
        let angle = s.atan2(c) / 4.0;
        let axis = [angle.cos(), angle.sin()];
        let normal = [-axis[1], axis[0]];
        for line in &mut lines {
            line.direction = if dot(line.direction, axis).abs() > 0.5f64.sqrt() {
                axis
            } else {
                normal
            };
        }
    }
    let mut vertices = Vec::new();
    for i in 0..lines.len() {
        vertices.push(intersection(
            lines[(i + lines.len() - 1) % lines.len()],
            lines[i],
        )?);
    }
    // Initially limit recognition to convex polygons; concave logos use the existing fitter.
    let turns: Vec<_> = (0..vertices.len())
        .map(|i| {
            cross(
                sub(vertices[(i + 1) % vertices.len()], vertices[i]),
                sub(
                    vertices[(i + 2) % vertices.len()],
                    vertices[(i + 1) % vertices.len()],
                ),
            )
        })
        .collect();
    if !(turns.iter().all(|&v| v > 0.01) || turns.iter().all(|&v| v < -0.01)) {
        return None;
    }
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        for j in i + 2..vertices.len() {
            if (j + 1) % vertices.len() == i {
                continue;
            }
            let c = vertices[j];
            let d = vertices[(j + 1) % vertices.len()];
            if cross(sub(b, a), sub(c, a)) * cross(sub(b, a), sub(d, a)) <= 0.0
                && cross(sub(d, c), sub(a, c)) * cross(sub(d, c), sub(b, c)) <= 0.0
            {
                return None;
            }
        }
    }
    // Bidirectional boundary proximity, with a separate, bounded corner extension budget.
    let dist = |q: P, a: P, b: P| {
        let d = sub(b, a);
        let t = (dot(sub(q, a), d) / dot(d, d).max(1e-12)).clamp(0.0, 1.0);
        norm(sub(q, [a[0] + t * d[0], a[1] + t * d[1]]))
    };
    for &vertex in &vertices {
        if raw
            .windows(2)
            .map(|w| dist(vertex, p(w[0]), p(w[1])))
            .fold(f64::INFINITY, f64::min)
            > extension.max(tolerance)
        {
            return None;
        }
    }
    for &q in &raw[..n] {
        let q = p(q);
        let error = (0..vertices.len())
            .map(|i| dist(q, vertices[i], vertices[(i + 1) % vertices.len()]))
            .fold(f64::INFINITY, f64::min);
        if error > tolerance && vertices.iter().all(|&v| norm(sub(q, v)) > extension) {
            return None;
        }
    }
    for i in 0..vertices.len() {
        let a = vertices[i];
        let b = vertices[(i + 1) % vertices.len()];
        let length = norm(sub(b, a));
        for k in 0..=(length.ceil() as usize) {
            let t = (k as f64 / length).min(1.0);
            let q = [a[0] + t * (b[0] - a[0]), a[1] + t * (b[1] - a[1])];
            if norm(sub(q, a)) <= extension || norm(sub(q, b)) <= extension {
                continue;
            }
            if raw
                .windows(2)
                .map(|w| dist(q, p(w[0]), p(w[1])))
                .fold(f64::INFINITY, f64::min)
                > tolerance
            {
                return None;
            }
        }
    }
    Some(Primitive::Polygon(vertices))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn boundary(f: impl Fn(f64, f64) -> bool) -> Vec<Point> {
        let image = crate::IndexedImage {
            width: 96,
            height: 96,
            palette: vec![[0; 3], [255; 3]],
            labels: (0..96 * 96)
                .map(|i| u8::from(f((i % 96) as f64 + 0.5, (i / 96) as f64 + 0.5)))
                .collect(),
        };
        let mut ring = crate::contours(&image, 1)
            .into_iter()
            .max_by_key(Vec::len)
            .unwrap();
        ring.push(ring[0]);
        let mut full = Vec::new();
        for w in ring.windows(2) {
            let mut q = w[0];
            let d = ((w[1].0 - q.0).signum(), (w[1].1 - q.1).signum());
            while q != w[1] {
                full.push(q);
                q = (q.0 + d.0, q.1 + d.1);
            }
        }
        full.push(full[0]);
        full
    }
    #[test]
    fn circles_are_recognized_but_ellipses_and_rounded_squares_are_not() {
        for radius in [5.0, 10.0, 25.0] {
            let raw = boundary(|x, y| (x - 48.3).hypot(y - 47.7) < radius);
            let Some(Primitive::Circle { center, radius: r }) = recognize(&raw, 1.0, 3.0) else {
                panic!("circle radius {radius} rejected")
            };
            assert!(norm(sub(center, [48.3, 47.7])) < 0.4);
            assert!((r - radius).abs() < 0.3);
        }
        let ellipse =
            boundary(|x, y| ((x - 48.0) / 25.0).powi(2) + ((y - 48.0) / 17.0).powi(2) < 1.0);
        assert!(recognize(&ellipse, 1.0, 3.0).is_none());
        let rounded =
            boundary(|x, y| ((x - 48.0) / 25.0).powi(4) + ((y - 48.0) / 25.0).powi(4) < 1.0);
        assert!(recognize(&rounded, 1.0, 3.0).is_none());
    }
    #[test]
    fn restores_a_clipped_triangle_tip_from_supporting_lines() {
        let raw = boundary(|x, y| {
            (14.0..=80.0).contains(&y)
                && x >= 48.0 - (y - 12.0) * 0.5
                && x <= 48.0 + (y - 12.0) * 0.5
        });
        let Some(Primitive::Polygon(vertices)) = recognize(&raw, 1.0, 3.0) else {
            panic!("triangle rejected")
        };
        assert_eq!(vertices.len(), 3, "{vertices:?}");
        let tip = vertices
            .iter()
            .min_by(|a, b| a[1].total_cmp(&b[1]))
            .unwrap();
        assert!(norm(sub(*tip, [48.0, 12.0])) < 1.0, "{vertices:?}");
    }
    #[test]
    fn preserves_rotated_rectangle_and_an_intentional_trapezoid() {
        let rectangle = boundary(|x, y| {
            let (x, y) = (x - 48.0, y - 48.0);
            (x * 0.8 + y * 0.6).abs() < 26.0 && (-x * 0.6 + y * 0.8).abs() < 15.0
        });
        let Some(Primitive::Polygon(v)) = recognize(&rectangle, 1.0, 3.0) else {
            panic!("rectangle rejected")
        };
        assert_eq!(v.len(), 4);
        for i in 0..4 {
            let a = sub(v[(i + 1) % 4], v[i]);
            let b = sub(v[(i + 2) % 4], v[(i + 1) % 4]);
            assert!(dot(a, b).abs() / (norm(a) * norm(b)) < 1e-10);
        }
        let trapezoid = boundary(|x, y| {
            (20.0..=78.0).contains(&y) && (x - 48.0).abs() < 12.0 + (y - 20.0) * 0.3
        });
        assert!(matches!(recognize(&trapezoid,1.0,3.0),Some(Primitive::Polygon(v)) if v.len()==4));
    }
}
