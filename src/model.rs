//! Notebook data model: notebooks, pages and the elements drawn on them.
//!
//! All geometry is expressed in *page units*, which are PDF points
//! (1/72 inch). An A4 page is 595 x 842 units.

use std::sync::Arc;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::ink::{self, InkParams, InkPoint, Sample};
use crate::text::{self, TextLayout};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct P {
    pub x: f32,
    pub y: f32,
}

impl P {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// Straight-alpha RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    pub const BLACK: Rgba = Rgba([20, 20, 24, 255]);

    pub fn with_alpha(self, a: u8) -> Self {
        Rgba([self.0[0], self.0[1], self.0[2], a])
    }

    pub fn rgb_f32(self) -> (f32, f32, f32) {
        (
            self.0[0] as f32 / 255.0,
            self.0[1] as f32 / 255.0,
            self.0[2] as f32 / 255.0,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageTemplate {
    Blank,
    Lined,
    Grid,
    Dotted,
}

impl PageTemplate {
    pub const ALL: [PageTemplate; 4] = [
        PageTemplate::Blank,
        PageTemplate::Lined,
        PageTemplate::Grid,
        PageTemplate::Dotted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            PageTemplate::Blank => "Blank",
            PageTemplate::Lined => "Lined",
            PageTemplate::Grid => "Grid",
            PageTemplate::Dotted => "Dotted",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PageSize {
    A4,
    A5,
    Letter,
}

impl PageSize {
    pub const ALL: [PageSize; 3] = [PageSize::A4, PageSize::A5, PageSize::Letter];

    pub fn dims(self) -> (f32, f32) {
        match self {
            PageSize::A4 => (595.0, 842.0),
            PageSize::A5 => (420.0, 595.0),
            PageSize::Letter => (612.0, 792.0),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            PageSize::A4 => "A4",
            PageSize::A5 => "A5",
            PageSize::Letter => "Letter",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum InkKind {
    Pen,
    Highlighter,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Stroke {
    pub kind: InkKind,
    pub color: Rgba,
    /// Nominal nib width in page units.
    pub width: f32,
    /// Whether the points carry real pen pressure (vs. simulated from speed).
    pub real_pressure: bool,
    pub points: Vec<InkPoint>,
    /// Smoothing / pressure response the stroke was drawn with.
    #[serde(default)]
    pub params: InkParams,
    /// The stroke was cut by the eraser at its start / end: don't taper there.
    #[serde(default)]
    pub cut_start: bool,
    #[serde(default)]
    pub cut_end: bool,
    /// Smoothed, resampled outline samples used for rendering. Derived data.
    #[serde(skip)]
    pub samples: Vec<Sample>,
    #[serde(skip)]
    pub bbox: Bounds,
}

impl Stroke {
    pub fn new(kind: InkKind, color: Rgba, width: f32, real_pressure: bool, params: InkParams, points: Vec<InkPoint>) -> Self {
        let mut s = Stroke {
            kind,
            color,
            width,
            real_pressure,
            points,
            params,
            cut_start: false,
            cut_end: false,
            samples: Vec::new(),
            bbox: Bounds::empty(),
        };
        s.rebuild();
        s
    }

    pub fn rebuild(&mut self) {
        self.samples = ink::build_samples(self);
        self.bbox = Bounds::empty();
        for s in &self.samples {
            self.bbox.add_circle(s.pos, s.w * 0.5);
        }
    }

    /// Removes the parts of the stroke that lie within `r` of the eraser
    /// path `from`→`to`. Returns `None` when the stroke isn't touched,
    /// otherwise the remaining pieces (possibly none).
    pub fn erase_partial(&self, from: P, to: P, r: f32) -> Option<Vec<Stroke>> {
        let reach = r + self.width * 0.35;
        if !self.bbox.expand(reach).intersects(&Bounds::from_points(from, to).expand(reach)) {
            return None;
        }
        let inside = |p: P| seg_dist(p, from, to) <= reach;
        let lerp = |a: InkPoint, b: InkPoint, t: f32| InkPoint {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
            p: a.p + (b.p - a.p) * t,
            t: a.t + (b.t - a.t) * t,
        };
        let pts = &self.points;
        let mut pieces: Vec<(Vec<InkPoint>, bool, bool)> = Vec::new();
        let mut cur: Vec<InkPoint> = Vec::new();
        let mut cur_cut_start = self.cut_start;
        let mut touched = false;
        let mut was_in = inside(P::new(pts[0].x, pts[0].y));
        if was_in {
            touched = true;
        } else {
            cur.push(pts[0]);
        }
        for i in 1..pts.len() {
            let (a, b) = (pts[i - 1], pts[i]);
            // test intermediate positions so fast strokes can't slip past the eraser
            let len = dist(P::new(a.x, a.y), P::new(b.x, b.y));
            let steps = ((len / (r * 0.4).max(0.2)).ceil() as usize).clamp(1, 256);
            for k in 1..=steps {
                let t = k as f32 / steps as f32;
                let q = lerp(a, b, t);
                let is_in = inside(P::new(q.x, q.y));
                if is_in != was_in {
                    touched = true;
                    let edge = lerp(a, b, (k as f32 - 0.5) / steps as f32);
                    if is_in {
                        cur.push(edge);
                        pieces.push((std::mem::take(&mut cur), cur_cut_start, true));
                    } else {
                        cur_cut_start = true;
                        cur.push(edge);
                    }
                    was_in = is_in;
                }
                if k == steps && !is_in {
                    cur.push(b);
                }
            }
        }
        if !touched {
            return None;
        }
        if !cur.is_empty() {
            pieces.push((cur, cur_cut_start, self.cut_end));
        }
        Some(
            pieces
                .into_iter()
                .filter(|(p, _, _)| {
                    let l: f32 = p.windows(2).map(|w| dist(P::new(w[0].x, w[0].y), P::new(w[1].x, w[1].y))).sum();
                    l > self.width * 0.6
                })
                .map(|(points, cs, ce)| {
                    let mut s = Stroke {
                        points,
                        cut_start: cs,
                        cut_end: ce,
                        samples: Vec::new(),
                        bbox: Bounds::empty(),
                        ..self.clone()
                    };
                    s.rebuild();
                    s
                })
                .collect(),
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShapeKind {
    Line,
    Arrow,
    Rect,
    Ellipse,
    Triangle,
    /// Closed polygon through `Shape::pts` (from shape recognition).
    Polygon,
}

impl ShapeKind {
    pub const TOOLS: [ShapeKind; 5] = [
        ShapeKind::Line,
        ShapeKind::Arrow,
        ShapeKind::Rect,
        ShapeKind::Ellipse,
        ShapeKind::Triangle,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ShapeKind::Line => "Line",
            ShapeKind::Arrow => "Arrow",
            ShapeKind::Rect => "Rectangle",
            ShapeKind::Ellipse => "Ellipse",
            ShapeKind::Triangle => "Triangle",
            ShapeKind::Polygon => "Polygon",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Shape {
    pub kind: ShapeKind,
    pub a: P,
    pub b: P,
    pub color: Rgba,
    pub width: f32,
    pub fill: Option<Rgba>,
    #[serde(default)]
    pub pts: Vec<P>,
}

impl Shape {
    pub fn new(kind: ShapeKind, a: P, b: P, color: Rgba, width: f32, fill: Option<Rgba>) -> Self {
        Shape {
            kind,
            a,
            b,
            color,
            width,
            fill,
            pts: Vec::new(),
        }
    }

    /// Outline of the shape as polylines (each `(points, closed)`).
    pub fn polylines(&self) -> Vec<(Vec<P>, bool)> {
        let (a, b) = (self.a, self.b);
        match self.kind {
            ShapeKind::Line => vec![(vec![a, b], false)],
            ShapeKind::Arrow => {
                let dx = b.x - a.x;
                let dy = b.y - a.y;
                let len = (dx * dx + dy * dy).sqrt().max(0.001);
                let (ux, uy) = (dx / len, dy / len);
                let head = (self.width * 4.0 + 6.0).min(len * 0.5);
                let spread = 0.5f32;
                let (s, c) = spread.sin_cos();
                let l = P::new(
                    b.x - head * (ux * c - uy * s),
                    b.y - head * (uy * c + ux * s),
                );
                let r = P::new(
                    b.x - head * (ux * c + uy * s),
                    b.y - head * (uy * c - ux * s),
                );
                vec![(vec![a, b], false), (vec![l, b, r], false)]
            }
            ShapeKind::Rect => {
                let r = Bounds::from_points(a, b);
                vec![(
                    vec![
                        P::new(r.min.x, r.min.y),
                        P::new(r.max.x, r.min.y),
                        P::new(r.max.x, r.max.y),
                        P::new(r.min.x, r.max.y),
                    ],
                    true,
                )]
            }
            ShapeKind::Ellipse => {
                let r = Bounds::from_points(a, b);
                let cx = (r.min.x + r.max.x) * 0.5;
                let cy = (r.min.y + r.max.y) * 0.5;
                let rx = (r.max.x - r.min.x) * 0.5;
                let ry = (r.max.y - r.min.y) * 0.5;
                let n = 96;
                let pts = (0..n)
                    .map(|i| {
                        let t = i as f32 / n as f32 * std::f32::consts::TAU;
                        P::new(cx + rx * t.cos(), cy + ry * t.sin())
                    })
                    .collect();
                vec![(pts, true)]
            }
            ShapeKind::Triangle => {
                let r = Bounds::from_points(a, b);
                vec![(
                    vec![
                        P::new((r.min.x + r.max.x) * 0.5, r.min.y),
                        P::new(r.max.x, r.max.y),
                        P::new(r.min.x, r.max.y),
                    ],
                    true,
                )]
            }
            ShapeKind::Polygon => vec![(self.pts.clone(), true)],
        }
    }

    pub fn fillable(&self) -> bool {
        !matches!(self.kind, ShapeKind::Line | ShapeKind::Arrow)
    }

    pub fn bounds(&self) -> Bounds {
        let mut b = Bounds::empty();
        for (pts, _) in self.polylines() {
            for p in pts {
                b.add_circle(p, self.width * 0.5);
            }
        }
        b
    }
}

/// Encoded image bytes (PNG/JPEG) shared between undo snapshots.
#[derive(Debug)]
pub struct ImageAsset {
    pub id: u64,
    pub bytes: Vec<u8>,
    pub px_w: u32,
    pub px_h: u32,
}

impl ImageAsset {
    pub fn from_encoded(bytes: Vec<u8>) -> Result<Arc<Self>, String> {
        let img = image::load_from_memory(&bytes).map_err(|e| e.to_string())?;
        let (px_w, px_h) = (img.width(), img.height());
        // Only keep formats every backend understands; recode everything else to PNG.
        let bytes = match image::guess_format(&bytes) {
            Ok(image::ImageFormat::Png) | Ok(image::ImageFormat::Jpeg) => bytes,
            _ => encode_png(&img.to_rgba8())?,
        };
        Ok(Arc::new(ImageAsset {
            id: next_id(),
            bytes,
            px_w,
            px_h,
        }))
    }

    pub fn from_rgba(w: u32, h: u32, rgba: Vec<u8>) -> Result<Arc<Self>, String> {
        let buf = image::RgbaImage::from_raw(w, h, rgba).ok_or("bad image buffer")?;
        let bytes = encode_png(&buf)?;
        Ok(Arc::new(ImageAsset {
            id: next_id(),
            bytes,
            px_w: w,
            px_h: h,
        }))
    }

    pub fn decode(&self) -> Option<image::DynamicImage> {
        image::load_from_memory(&self.bytes).ok()
    }
}

fn encode_png(img: &image::RgbaImage) -> Result<Vec<u8>, String> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png)
        .map_err(|e| e.to_string())?;
    Ok(out.into_inner())
}

pub fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Serialize for ImageAsset {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&base64::engine::general_purpose::STANDARD.encode(&self.bytes))
    }
}

impl<'de> Deserialize<'de> for ImageAsset {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(s)
            .map_err(serde::de::Error::custom)?;
        let asset = ImageAsset::from_encoded(bytes).map_err(serde::de::Error::custom)?;
        Ok(Arc::try_unwrap(asset).unwrap_or_else(|a| ImageAsset {
            id: a.id,
            bytes: a.bytes.clone(),
            px_w: a.px_w,
            px_h: a.px_h,
        }))
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageElem {
    pub asset: Arc<ImageAsset>,
    /// Top-left corner and size in page units.
    pub pos: P,
    pub w: f32,
    pub h: f32,
}

impl ImageElem {
    pub fn bounds(&self) -> Bounds {
        Bounds {
            min: self.pos,
            max: P::new(self.pos.x + self.w, self.pos.y + self.h),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TextElem {
    /// Top-left corner of the text box.
    pub pos: P,
    pub text: String,
    /// Font size in page units.
    pub size: f32,
    pub color: Rgba,
    /// Wrapping width in page units.
    pub width: f32,
    #[serde(skip)]
    pub layout: TextLayout,
}

impl TextElem {
    pub fn new(pos: P, text: String, size: f32, color: Rgba, width: f32) -> Self {
        let mut t = TextElem {
            pos,
            text,
            size,
            color,
            width,
            layout: TextLayout::default(),
        };
        t.rebuild();
        t
    }

    pub fn rebuild(&mut self) {
        self.layout = text::layout(&self.text, self.size, self.width);
    }

    pub fn bounds(&self) -> Bounds {
        Bounds {
            min: self.pos,
            max: P::new(
                self.pos.x + self.layout.width.max(self.size * 0.5),
                self.pos.y + self.layout.height.max(self.size),
            ),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Element {
    Stroke(Stroke),
    Shape(Shape),
    Image(ImageElem),
    Text(TextElem),
}

impl Element {
    pub fn bounds(&self) -> Bounds {
        match self {
            Element::Stroke(s) => s.bbox,
            Element::Shape(s) => s.bounds(),
            Element::Image(i) => i.bounds(),
            Element::Text(t) => t.bounds(),
        }
    }

    pub fn rebuild(&mut self) {
        match self {
            Element::Stroke(s) => s.rebuild(),
            Element::Text(t) => t.rebuild(),
            _ => {}
        }
    }

    /// Apply `p' = origin + (p - origin) * scale + offset` to the element.
    pub fn transform(&mut self, origin: P, scale: f32, offset: P) {
        let f = |p: P| {
            P::new(
                origin.x + (p.x - origin.x) * scale + offset.x,
                origin.y + (p.y - origin.y) * scale + offset.y,
            )
        };
        match self {
            Element::Stroke(s) => {
                for p in &mut s.points {
                    let q = f(P::new(p.x, p.y));
                    p.x = q.x;
                    p.y = q.y;
                }
                if scale == 1.0 {
                    // pure translation: shift derived data instead of rebuilding it
                    for smp in &mut s.samples {
                        smp.pos = f(smp.pos);
                    }
                    s.bbox.min = f(s.bbox.min);
                    s.bbox.max = f(s.bbox.max);
                } else {
                    s.width *= scale;
                    s.rebuild();
                }
            }
            Element::Shape(s) => {
                s.a = f(s.a);
                s.b = f(s.b);
                for p in &mut s.pts {
                    *p = f(*p);
                }
                s.width *= scale;
            }
            Element::Image(i) => {
                i.pos = f(i.pos);
                i.w *= scale;
                i.h *= scale;
            }
            Element::Text(t) => {
                t.pos = f(t.pos);
                if scale != 1.0 {
                    t.size *= scale;
                    t.width *= scale;
                    t.rebuild();
                }
            }
        }
    }

    pub fn set_color(&mut self, c: Rgba) {
        match self {
            Element::Stroke(s) => {
                let a = s.color.0[3];
                s.color = c.with_alpha(a);
            }
            Element::Shape(s) => s.color = c,
            Element::Text(t) => t.color = c,
            Element::Image(_) => {}
        }
    }

    /// Distance-based hit test with a tolerance radius, in page units.
    pub fn hit(&self, p: P, radius: f32) -> bool {
        match self {
            Element::Stroke(s) => {
                if !s.bbox.expand(radius).contains(p) {
                    return false;
                }
                if s.samples.len() == 1 {
                    return dist(p, s.samples[0].pos) <= radius + s.samples[0].w * 0.5;
                }
                s.samples.windows(2).any(|w| {
                    seg_dist(p, w[0].pos, w[1].pos) <= radius + w[0].w.max(w[1].w) * 0.5
                })
            }
            Element::Shape(s) => {
                if let Some(_) = s.fill {
                    if s.bounds().contains(p) {
                        return true;
                    }
                }
                s.polylines().iter().any(|(pts, closed)| {
                    let n = pts.len();
                    let segs = if *closed { n } else { n - 1 };
                    (0..segs).any(|i| seg_dist(p, pts[i], pts[(i + 1) % n]) <= radius + s.width * 0.5)
                })
            }
            Element::Image(i) => i.bounds().expand(radius).contains(p),
            Element::Text(t) => t.bounds().expand(radius).contains(p),
        }
    }

    /// Points that represent the element for lasso selection.
    pub fn probe_points(&self) -> Vec<P> {
        match self {
            Element::Stroke(s) => s.points.iter().map(|p| P::new(p.x, p.y)).collect(),
            Element::Shape(s) => s.polylines().into_iter().flat_map(|(p, _)| p).collect(),
            Element::Image(i) => vec![i.bounds().center()],
            Element::Text(t) => vec![t.bounds().center()],
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Bounds {
    pub min: P,
    pub max: P,
}

impl Default for P {
    fn default() -> Self {
        P::new(0.0, 0.0)
    }
}

impl Bounds {
    pub fn empty() -> Self {
        Bounds {
            min: P::new(f32::INFINITY, f32::INFINITY),
            max: P::new(f32::NEG_INFINITY, f32::NEG_INFINITY),
        }
    }

    pub fn from_points(a: P, b: P) -> Self {
        Bounds {
            min: P::new(a.x.min(b.x), a.y.min(b.y)),
            max: P::new(a.x.max(b.x), a.y.max(b.y)),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x
    }

    pub fn add(&mut self, p: P) {
        self.min.x = self.min.x.min(p.x);
        self.min.y = self.min.y.min(p.y);
        self.max.x = self.max.x.max(p.x);
        self.max.y = self.max.y.max(p.y);
    }

    pub fn add_circle(&mut self, p: P, r: f32) {
        self.add(P::new(p.x - r, p.y - r));
        self.add(P::new(p.x + r, p.y + r));
    }

    pub fn union(&mut self, o: Bounds) {
        if !o.is_empty() {
            self.add(o.min);
            self.add(o.max);
        }
    }

    pub fn expand(&self, r: f32) -> Bounds {
        Bounds {
            min: P::new(self.min.x - r, self.min.y - r),
            max: P::new(self.max.x + r, self.max.y + r),
        }
    }

    pub fn contains(&self, p: P) -> bool {
        p.x >= self.min.x && p.x <= self.max.x && p.y >= self.min.y && p.y <= self.max.y
    }

    pub fn intersects(&self, o: &Bounds) -> bool {
        self.min.x <= o.max.x && self.max.x >= o.min.x && self.min.y <= o.max.y && self.max.y >= o.min.y
    }

    pub fn center(&self) -> P {
        P::new((self.min.x + self.max.x) * 0.5, (self.min.y + self.max.y) * 0.5)
    }
}

pub fn dist(a: P, b: P) -> f32 {
    ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
}

pub fn seg_dist(p: P, a: P, b: P) -> f32 {
    let (abx, aby) = (b.x - a.x, b.y - a.y);
    let len2 = abx * abx + aby * aby;
    if len2 < 1e-9 {
        return dist(p, a);
    }
    let t = (((p.x - a.x) * abx + (p.y - a.y) * aby) / len2).clamp(0.0, 1.0);
    dist(p, P::new(a.x + abx * t, a.y + aby * t))
}

pub fn point_in_polygon(p: P, poly: &[P]) -> bool {
    let mut inside = false;
    let n = poly.len();
    if n < 3 {
        return false;
    }
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (poly[i], poly[j]);
        if (a.y > p.y) != (b.y > p.y) && p.x < (b.x - a.x) * (p.y - a.y) / (b.y - a.y) + a.x {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub width: f32,
    pub height: f32,
    pub template: PageTemplate,
    pub paper: Rgba,
    pub elements: Vec<Element>,
}

impl Page {
    pub fn new(size: PageSize, template: PageTemplate) -> Self {
        let (width, height) = size.dims();
        Page {
            width,
            height,
            template,
            paper: Rgba([255, 255, 255, 255]),
            elements: Vec::new(),
        }
    }

    /// Lines of the page background template: `(from, to)` pairs plus dots.
    pub fn template_geometry(&self) -> TemplateGeometry {
        let mut g = TemplateGeometry::default();
        let (w, h) = (self.width, self.height);
        match self.template {
            PageTemplate::Blank => {}
            PageTemplate::Lined => {
                let step = 24.0;
                let mut y = 90.0;
                while y < h - 30.0 {
                    g.lines.push((P::new(0.0, y), P::new(w, y)));
                    y += step;
                }
                g.margin = Some((P::new(70.0, 0.0), P::new(70.0, h)));
            }
            PageTemplate::Grid => {
                let step = 14.17; // 5 mm
                let mut x = step;
                while x < w {
                    g.lines.push((P::new(x, 0.0), P::new(x, h)));
                    x += step;
                }
                let mut y = step;
                while y < h {
                    g.lines.push((P::new(0.0, y), P::new(w, y)));
                    y += step;
                }
            }
            PageTemplate::Dotted => {
                let step = 14.17;
                let mut y = step;
                while y < h {
                    let mut x = step;
                    while x < w {
                        g.dots.push(P::new(x, y));
                        x += step;
                    }
                    y += step;
                }
            }
        }
        g
    }
}

#[derive(Default)]
pub struct TemplateGeometry {
    pub lines: Vec<(P, P)>,
    pub dots: Vec<P>,
    pub margin: Option<(P, P)>,
}

pub const TEMPLATE_LINE: Rgba = Rgba([170, 196, 222, 255]);
pub const TEMPLATE_MARGIN: Rgba = Rgba([232, 150, 150, 255]);
pub const TEMPLATE_LINE_WIDTH: f32 = 0.6;
pub const TEMPLATE_DOT_RADIUS: f32 = 0.9;

pub const COVER_COLORS: [Rgba; 10] = [
    Rgba([52, 120, 246, 255]),
    Rgba([88, 86, 214, 255]),
    Rgba([175, 82, 222, 255]),
    Rgba([255, 45, 85, 255]),
    Rgba([255, 59, 48, 255]),
    Rgba([255, 149, 0, 255]),
    Rgba([255, 204, 0, 255]),
    Rgba([52, 199, 89, 255]),
    Rgba([0, 166, 153, 255]),
    Rgba([99, 99, 102, 255]),
];

fn default_cover() -> Rgba {
    COVER_COLORS[0]
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Notebook {
    pub title: String,
    #[serde(default = "default_cover")]
    pub cover: Rgba,
    pub pages: Vec<Page>,
}

impl Notebook {
    pub fn new(title: &str, cover: Rgba, size: PageSize, template: PageTemplate) -> Self {
        Notebook {
            title: title.into(),
            cover,
            pages: vec![Page::new(size, template)],
        }
    }

    pub fn save(&self, path: &std::path::Path) -> Result<(), String> {
        let json = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        // write-then-rename so a crash never leaves a half-written notebook
        let tmp = path.with_extension("bnote.tmp");
        std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, path).map_err(|e| e.to_string())
    }

    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let mut nb: Notebook = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        // derived stroke data isn't stored in the file
        for page in &mut nb.pages {
            for el in &mut page.elements {
                el.rebuild();
            }
        }
        Ok(nb)
    }
}
