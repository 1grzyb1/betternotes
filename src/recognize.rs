//! Shape recognition for "draw and hold": turns a rough hand-drawn line,
//! ellipse, rectangle, triangle or polygon into a clean shape.

use crate::model::{Bounds, P, ShapeKind, dist, seg_dist};

#[derive(Clone, Debug, PartialEq)]
pub struct Recognized {
    pub kind: ShapeKind,
    pub a: P,
    pub b: P,
    pub pts: Vec<P>,
}

pub fn recognize(points: &[P]) -> Option<Recognized> {
    if points.len() < 3 {
        return None;
    }
    let length: f32 = points.windows(2).map(|w| dist(w[0], w[1])).sum();
    if length < 14.0 {
        return None;
    }
    let first = points[0];
    let last = points[points.len() - 1];
    let chord = dist(first, last);

    // straight line
    if chord > 0.85 * length {
        let dev = points.iter().map(|p| seg_dist(*p, first, last)).fold(0.0, f32::max);
        if dev < (chord * 0.06).max(2.5) {
            return Some(Recognized {
                kind: ShapeKind::Line,
                a: first,
                b: last,
                pts: Vec::new(),
            });
        }
    }

    // everything else must be (roughly) closed
    if chord > 0.2 * length {
        return None;
    }
    let mut b = Bounds::empty();
    for p in points {
        b.add(*p);
    }
    let (w, h) = (b.max.x - b.min.x, b.max.y - b.min.y);
    if w < 6.0 || h < 6.0 {
        return None;
    }

    // ellipse: mean deviation of the normalised radius from 1
    let c = b.center();
    let (rx, ry) = (w * 0.5, h * 0.5);
    let err = points
        .iter()
        .map(|p| {
            let (dx, dy) = ((p.x - c.x) / rx, (p.y - c.y) / ry);
            ((dx * dx + dy * dy).sqrt() - 1.0).abs()
        })
        .sum::<f32>()
        / points.len() as f32;
    if err < 0.085 {
        return Some(Recognized {
            kind: ShapeKind::Ellipse,
            a: b.min,
            b: b.max,
            pts: Vec::new(),
        });
    }

    // polygons via corner detection
    let mut closed: Vec<P> = points.to_vec();
    closed.push(first);
    let corners = closed_corners(&closed, length * 0.045);
    match corners.len() {
        3 => Some(Recognized {
            kind: ShapeKind::Polygon,
            a: b.min,
            b: b.max,
            pts: corners,
        }),
        4 => {
            let axis_aligned = (0..4).all(|i| {
                let (p, q) = (corners[i], corners[(i + 1) % 4]);
                let ang = (q.y - p.y).atan2(q.x - p.x).abs();
                let off = (ang % std::f32::consts::FRAC_PI_2).min(std::f32::consts::FRAC_PI_2 - ang % std::f32::consts::FRAC_PI_2);
                off < 0.21 // ~12°
            });
            if axis_aligned {
                let mut cb = Bounds::empty();
                for p in &corners {
                    cb.add(*p);
                }
                Some(Recognized {
                    kind: ShapeKind::Rect,
                    a: cb.min,
                    b: cb.max,
                    pts: Vec::new(),
                })
            } else {
                Some(Recognized {
                    kind: ShapeKind::Polygon,
                    a: b.min,
                    b: b.max,
                    pts: corners,
                })
            }
        }
        5 | 6 => Some(Recognized {
            kind: ShapeKind::Polygon,
            a: b.min,
            b: b.max,
            pts: corners,
        }),
        _ if err < 0.13 => Some(Recognized {
            kind: ShapeKind::Ellipse,
            a: b.min,
            b: b.max,
            pts: Vec::new(),
        }),
        _ => None,
    }
}

/// Corners of a closed path: Ramer–Douglas–Peucker started from the two
/// points farthest apart, so the result doesn't depend on where drawing began.
fn closed_corners(path: &[P], eps: f32) -> Vec<P> {
    let n = path.len() - 1; // last == first
    let mut far = (0, 0, 0.0f32);
    let step = (n / 64).max(1);
    for i in (0..n).step_by(step) {
        for j in (i..n).step_by(step) {
            let d = dist(path[i], path[j]);
            if d > far.2 {
                far = (i, j, d);
            }
        }
    }
    let (i, j) = (far.0, far.1);
    let a: Vec<P> = path[i..=j].to_vec();
    let mut b: Vec<P> = path[j..n].to_vec();
    b.extend_from_slice(&path[..=i]);
    let mut out = rdp(&a, eps);
    out.pop();
    let mut rest = rdp(&b, eps);
    rest.pop();
    out.extend(rest);
    merge_close(out, eps * 1.5)
}

fn rdp(pts: &[P], eps: f32) -> Vec<P> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let (a, b) = (pts[0], pts[pts.len() - 1]);
    let (idx, d) = pts[1..pts.len() - 1]
        .iter()
        .enumerate()
        .map(|(i, p)| (i + 1, seg_dist(*p, a, b)))
        .fold((0, 0.0f32), |acc, x| if x.1 > acc.1 { x } else { acc });
    if d > eps {
        let mut left = rdp(&pts[..=idx], eps);
        let right = rdp(&pts[idx..], eps);
        left.pop();
        left.extend(right);
        left
    } else {
        vec![a, b]
    }
}

fn merge_close(pts: Vec<P>, min: f32) -> Vec<P> {
    let mut out: Vec<P> = Vec::new();
    for p in pts {
        if out.last().is_none_or(|l| dist(*l, p) > min) {
            out.push(p);
        }
    }
    if out.len() > 2 && dist(out[0], *out.last().unwrap()) < min {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wobble(i: usize) -> f32 {
        ((i * 7919) % 11) as f32 / 11.0 - 0.5
    }

    #[test]
    fn recognises_basic_shapes() {
        let circle: Vec<P> = (0..=80)
            .map(|i| {
                let a = i as f32 / 80.0 * std::f32::consts::TAU;
                P::new(100.0 + 40.0 * a.cos() + wobble(i), 100.0 + 30.0 * a.sin() + wobble(i))
            })
            .collect();
        assert_eq!(recognize(&circle).unwrap().kind, ShapeKind::Ellipse);

        let line: Vec<P> = (0..40).map(|i| P::new(i as f32 * 3.0, 10.0 + wobble(i))).collect();
        assert_eq!(recognize(&line).unwrap().kind, ShapeKind::Line);

        let mut rect = Vec::new();
        for (a, b) in [((0.0, 0.0), (120.0, 0.0)), ((120.0, 0.0), (120.0, 80.0)), ((120.0, 80.0), (0.0, 80.0)), ((0.0, 80.0), (0.0, 1.0))] {
            for k in 0..30 {
                let t = k as f32 / 30.0;
                rect.push(P::new(a.0 + (b.0 - a.0) * t + wobble(k), a.1 + (b.1 - a.1) * t + wobble(k + 3)));
            }
        }
        assert_eq!(recognize(&rect).unwrap().kind, ShapeKind::Rect);

        let mut tri = Vec::new();
        for (a, b) in [((50.0, 0.0), (100.0, 90.0)), ((100.0, 90.0), (0.0, 90.0)), ((0.0, 90.0), (49.0, 2.0))] {
            for k in 0..30 {
                let t = k as f32 / 30.0;
                tri.push(P::new(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
        }
        let r = recognize(&tri).unwrap();
        assert_eq!(r.kind, ShapeKind::Polygon);
        assert_eq!(r.pts.len(), 3);

        let scribble: Vec<P> = (0..60).map(|i| P::new(i as f32 * 2.0, (i as f32 * 0.9).sin() * 20.0)).collect();
        assert!(recognize(&scribble).is_none());
    }
}
