//! Handwriting engine: turns raw pointer input into smooth, pressure-sensitive
//! variable-width ink.
//!
//! Pipeline (all in page units):
//! 1. raw points (position, pressure, timestamp) are recorded as-is,
//! 2. positions are stabilised with a speed-adaptive low-pass filter,
//!    which removes hand / sensor jitter without adding lag to fast strokes,
//! 3. pressure is smoothed (real pen pressure) or simulated from speed (mouse),
//! 4. the polyline is resampled with a centripetal Catmull-Rom spline,
//! 5. each sample gets a width from a pressure curve plus optional tapering,
//! 6. renderers split the samples at sharp cusps and draw each piece as a
//!    variable-width ribbon with round caps.

use serde::{Deserialize, Serialize};

use crate::model::{InkKind, P, Stroke};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct InkPoint {
    pub x: f32,
    pub y: f32,
    /// Pen pressure in 0..=1. Ignored for strokes without real pressure.
    pub p: f32,
    /// Milliseconds since the stroke started.
    pub t: f32,
}

/// User-tunable feel of the pen. Stored per stroke so strokes keep their
/// look when the settings change later.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct InkParams {
    /// 0 = raw input, 1 = heavy stabilisation.
    pub smoothing: f32,
    /// Pressure curve exponent: < 1 makes light touches thicker.
    pub pressure_gamma: f32,
    /// How much the width varies with pressure (0 = constant width).
    pub thinning: f32,
}

impl Default for InkParams {
    fn default() -> Self {
        InkParams {
            smoothing: 0.65,
            pressure_gamma: 0.8,
            thinning: 0.78,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sample {
    pub pos: P,
    /// Full ink width at this sample.
    pub w: f32,
}

/// Distance between resampled points along the spline.
const RESAMPLE_STEP: f32 = 0.45;

/// Maps pressure to a width multiplier. Slightly concave by default so light
/// touches still produce a visible line while firm pressure gives the full nib.
fn pressure_curve(p: f32, params: &InkParams) -> f32 {
    let min = 1.0 - params.thinning.clamp(0.0, 1.0);
    min + (1.1 - min) * p.clamp(0.0, 1.0).powf(params.pressure_gamma.clamp(0.2, 3.0))
}

pub fn build_samples(stroke: &Stroke) -> Vec<Sample> {
    let points = &stroke.points[..];
    let (width, kind, real_pressure) = (stroke.width, stroke.kind, stroke.real_pressure);
    let params = &stroke.params;
    if points.is_empty() {
        return Vec::new();
    }
    let min_w = 0.3f32;
    if points.len() == 1 {
        let p = points[0];
        let w = match kind {
            InkKind::Highlighter => width,
            InkKind::Pen => width * pressure_curve(if real_pressure { p.p } else { 0.6 }, params),
        };
        return vec![Sample {
            pos: P::new(p.x, p.y),
            w: w.max(min_w),
        }];
    }

    // --- 2. stabilise positions ------------------------------------------
    // Speed-adaptive exponential smoothing (in the spirit of the 1€ filter):
    // slow, careful movement is smoothed strongly, fast flicks follow the pen.
    let mut smooth: Vec<InkPoint> = Vec::with_capacity(points.len() + 8);
    let mut cur = points[0];
    smooth.push(cur);
    let alpha_min = (0.95 - 0.85 * params.smoothing.clamp(0.0, 1.0)).clamp(0.1, 0.95);
    for raw in &points[1..] {
        let dt = (raw.t - cur.t).max(1.0);
        let d = ((raw.x - cur.x).powi(2) + (raw.y - cur.y).powi(2)).sqrt();
        let speed = d / dt; // units per ms
        let alpha = (alpha_min + speed * 0.6).clamp(alpha_min, 0.9_f32.max(alpha_min));
        cur = InkPoint {
            x: cur.x + (raw.x - cur.x) * alpha,
            y: cur.y + (raw.y - cur.y) * alpha,
            p: raw.p,
            t: raw.t,
        };
        smooth.push(cur);
    }
    // Let the filter catch up with the final pen position so the stroke ends
    // exactly where the pen lifted, with a smooth approach instead of a hook.
    let last = *points.last().unwrap();
    for _ in 0..6 {
        let d = ((last.x - cur.x).powi(2) + (last.y - cur.y).powi(2)).sqrt();
        if d < 0.15 {
            break;
        }
        cur.x += (last.x - cur.x) * 0.55;
        cur.y += (last.y - cur.y) * 0.55;
        cur.p = last.p;
        smooth.push(cur);
    }

    // --- 3. pressure -------------------------------------------------------
    let mut pressures = Vec::with_capacity(smooth.len());
    if real_pressure {
        // Many tablets report a low/zero first sample; seed from the first
        // non-trivial reading so the stroke doesn't start with a pin-prick.
        let seed = points.iter().map(|p| p.p).find(|&p| p > 0.05).unwrap_or(0.5);
        let mut pr = seed.min(points[0].p.max(seed * 0.6));
        for s in &smooth {
            let target = s.p.max(0.02);
            pr += (target - pr) * 0.45;
            pressures.push(pr);
        }
    } else {
        // Simulated: fast movement thins the line, slow movement thickens it.
        let mut pr = 0.62f32;
        pressures.push(pr);
        for i in 1..smooth.len() {
            let (a, b) = (smooth[i - 1], smooth[i]);
            let dt = (b.t - a.t).max(1.0);
            let speed = ((b.x - a.x).powi(2) + (b.y - a.y).powi(2)).sqrt() / dt;
            let target = (1.0 - (speed / 2.2).min(0.6)).clamp(0.4, 1.0) * 0.85;
            pr += (target - pr) * 0.18;
            pressures.push(pr);
        }
    }

    // Drop near-duplicate points (keeps the spline well-conditioned).
    let mut pts: Vec<(P, f32)> = Vec::with_capacity(smooth.len());
    for (s, &pr) in smooth.iter().zip(&pressures) {
        let p = P::new(s.x, s.y);
        if let Some(&(lp, _)) = pts.last() {
            if crate::model::dist(lp, p) < 0.12 {
                // keep the newest pressure
                pts.last_mut().unwrap().1 = pr;
                continue;
            }
        }
        pts.push((p, pr));
    }
    if pts.len() == 1 {
        let (p, pr) = pts[0];
        return vec![Sample {
            pos: p,
            w: width_for(width, kind, pr, params).max(min_w),
        }];
    }

    // --- 4. centripetal Catmull-Rom resampling -----------------------------
    let n = pts.len();
    let mut out: Vec<Sample> = Vec::with_capacity(n * 3);
    for i in 0..n - 1 {
        let p0 = if i == 0 { extrapolate(pts[0].0, pts[1].0) } else { pts[i - 1].0 };
        let p1 = pts[i].0;
        let p2 = pts[i + 1].0;
        let p3 = if i + 2 < n { pts[i + 2].0 } else { extrapolate(pts[n - 1].0, pts[n - 2].0) };
        let (pr1, pr2) = (pts[i].1, pts[i + 1].1);
        let seg = crate::model::dist(p1, p2);
        let steps = ((seg / RESAMPLE_STEP).ceil() as usize).clamp(1, 64);
        for s in 0..steps {
            let t = s as f32 / steps as f32;
            let pos = catmull_rom(p0, p1, p2, p3, t);
            let pr = pr1 + (pr2 - pr1) * t;
            out.push(Sample {
                pos,
                w: width_for(width, kind, pr, params),
            });
        }
    }
    let (lp, lpr) = pts[n - 1];
    out.push(Sample {
        pos: lp,
        w: width_for(width, kind, lpr, params),
    });

    // --- 5. tapering ---------------------------------------------------------
    if kind == InkKind::Pen && !real_pressure {
        let total: f32 = out.windows(2).map(|w| crate::model::dist(w[0].pos, w[1].pos)).sum();
        let taper_start = (width * 2.0).min(total * 0.2);
        let taper_end = (width * 3.0).min(total * 0.25);
        if !stroke.cut_start {
            apply_taper(&mut out, taper_start, 0.45);
        }
        if !stroke.cut_end {
            out.reverse();
            apply_taper(&mut out, taper_end, 0.3);
            out.reverse();
        }
    }

    for s in &mut out {
        s.w = s.w.max(min_w);
    }
    out
}

fn width_for(width: f32, kind: InkKind, pressure: f32, params: &InkParams) -> f32 {
    match kind {
        InkKind::Pen => width * pressure_curve(pressure, params),
        InkKind::Highlighter => width,
    }
}

fn apply_taper(samples: &mut [Sample], length: f32, min_factor: f32) {
    if length <= 0.0 {
        return;
    }
    let mut acc = 0.0;
    let mut prev = samples[0].pos;
    for s in samples.iter_mut() {
        acc += crate::model::dist(prev, s.pos);
        prev = s.pos;
        if acc >= length {
            break;
        }
        let t = acc / length;
        // ease-out so the taper looks like a natural pen landing
        let f = min_factor + (1.0 - min_factor) * (1.0 - (1.0 - t).powi(2));
        s.w *= f;
    }
}

fn extrapolate(a: P, b: P) -> P {
    P::new(a.x * 2.0 - b.x, a.y * 2.0 - b.y)
}

/// Centripetal Catmull-Rom (alpha = 0.5): no cusps or self-intersections
/// within a segment, which keeps curves of handwriting round and natural.
fn catmull_rom(p0: P, p1: P, p2: P, p3: P, t: f32) -> P {
    let knot = |a: P, b: P| crate::model::dist(a, b).max(1e-4).sqrt();
    let t0 = 0.0;
    let t1 = t0 + knot(p0, p1);
    let t2 = t1 + knot(p1, p2);
    let t3 = t2 + knot(p2, p3);
    let tt = t1 + (t2 - t1) * t;
    let lerp = |a: P, b: P, ta: f32, tb: f32| {
        let d = tb - ta;
        if d.abs() < 1e-6 {
            return a;
        }
        let u = (tt - ta) / d;
        P::new(a.x + (b.x - a.x) * u, a.y + (b.y - a.y) * u)
    };
    let a1 = lerp(p0, p1, t0, t1);
    let a2 = lerp(p1, p2, t1, t2);
    let a3 = lerp(p2, p3, t2, t3);
    let b1 = lerp(a1, a2, t0, t2);
    let b2 = lerp(a2, a3, t1, t3);
    lerp(b1, b2, t1, t2)
}

/// Splits samples at sharp direction changes. Each piece is rendered as its
/// own ribbon with round caps, which gives crisp, correctly shaped corners
/// (e.g. in "n", "m", "v") instead of pinched joints.
pub fn pieces(samples: &[Sample]) -> Vec<&[Sample]> {
    let mut out = Vec::new();
    if samples.len() < 3 {
        out.push(samples);
        return out;
    }
    let mut start = 0;
    for i in 1..samples.len() - 1 {
        let a = samples[i - 1].pos;
        let b = samples[i].pos;
        let c = samples[i + 1].pos;
        let (d1x, d1y) = (b.x - a.x, b.y - a.y);
        let (d2x, d2y) = (c.x - b.x, c.y - b.y);
        let l1 = (d1x * d1x + d1y * d1y).sqrt();
        let l2 = (d2x * d2x + d2y * d2y).sqrt();
        if l1 < 1e-5 || l2 < 1e-5 {
            continue;
        }
        let cos = (d1x * d2x + d1y * d2y) / (l1 * l2);
        if cos < 0.5 {
            out.push(&samples[start..=i]);
            start = i;
        }
    }
    out.push(&samples[start..]);
    out
}

/// Per-sample unit normals for a ribbon (averaged between adjacent segments).
pub fn normals(samples: &[Sample]) -> Vec<P> {
    let n = samples.len();
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let a = samples[i.saturating_sub(1)].pos;
        let b = samples[(i + 1).min(n - 1)].pos;
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-6 {
            out.push(out.last().copied().unwrap_or(P::new(0.0, 1.0)));
        } else {
            out.push(P::new(-dy / len, dx / len));
        }
    }
    out
}

/// Closed outline polygons covering the stroke (union with non-zero winding).
/// Used by vector (PDF) and raster exporters.
pub fn outline_polygons(samples: &[Sample]) -> Vec<Vec<P>> {
    let mut polys = Vec::new();
    for piece in pieces(samples) {
        if piece.is_empty() {
            continue;
        }
        let total: f32 = piece.windows(2).map(|w| crate::model::dist(w[0].pos, w[1].pos)).sum();
        if piece.len() == 1 || total < 0.05 {
            let s = piece[0];
            let r = piece.iter().map(|s| s.w).fold(0.0, f32::max) * 0.5;
            polys.push(circle(s.pos, r, 20));
            continue;
        }
        let nrm = normals(piece);
        let mut left = Vec::with_capacity(piece.len());
        let mut right = Vec::with_capacity(piece.len());
        for (s, n) in piece.iter().zip(&nrm) {
            let h = s.w * 0.5;
            left.push(P::new(s.pos.x + n.x * h, s.pos.y + n.y * h));
            right.push(P::new(s.pos.x - n.x * h, s.pos.y - n.y * h));
        }
        let mut poly = left;
        // end cap: from left normal to right normal, sweeping through forward dir
        let last = piece[piece.len() - 1];
        let nl = nrm[nrm.len() - 1];
        poly.extend(cap_arc(last.pos, nl, last.w * 0.5));
        poly.extend(right.into_iter().rev());
        let first = piece[0];
        let nf = nrm[0];
        poly.extend(cap_arc(first.pos, P::new(-nf.x, -nf.y), first.w * 0.5));
        polys.push(poly);
    }
    polys
}

/// Half circle from `c + n*r` to `c - n*r`, passing through the side the
/// normal rotates towards (the "outside" of the cap).
pub fn cap_arc(c: P, n: P, r: f32) -> Vec<P> {
    let start = n.y.atan2(n.x);
    let steps = 10;
    (1..steps)
        .map(|i| {
            // normals are (-dy, dx); rotating by -angle moves towards forward direction
            let a = start - std::f32::consts::PI * i as f32 / steps as f32;
            P::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}

pub fn circle(c: P, r: f32, n: usize) -> Vec<P> {
    (0..n)
        .map(|i| {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            P::new(c.x + r * a.cos(), c.y + r * a.sin())
        })
        .collect()
}
