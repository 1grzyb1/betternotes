//! On-screen rendering with egui: pages, templates and anti-aliased ink meshes.

use std::collections::HashMap;

use eframe::egui::{
    self, Color32, Mesh, Painter, Pos2, Rect, Shape as EShape, Stroke as EStroke, TextureHandle, Vec2,
    epaint::{PathShape, PathStroke, Vertex, WHITE_UV},
};

use crate::ink::{self, Sample};
use crate::model::*;

pub fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c.0[0], c.0[1], c.0[2], c.0[3])
}

/// Maps page units to screen points for one page.
#[derive(Clone, Copy)]
pub struct PageView {
    pub origin: Pos2,
    pub zoom: f32,
}

impl PageView {
    pub fn to_screen(&self, p: P) -> Pos2 {
        Pos2::new(self.origin.x + p.x * self.zoom, self.origin.y + p.y * self.zoom)
    }

    pub fn to_page(&self, p: Pos2) -> P {
        P::new((p.x - self.origin.x) / self.zoom, (p.y - self.origin.y) / self.zoom)
    }

    pub fn rect(&self, b: Bounds) -> Rect {
        Rect::from_min_max(self.to_screen(b.min), self.to_screen(b.max))
    }
}

#[derive(Default)]
pub struct TextureCache {
    map: HashMap<u64, TextureHandle>,
}

impl TextureCache {
    pub fn get(&mut self, ctx: &egui::Context, asset: &ImageAsset) -> Option<&TextureHandle> {
        if !self.map.contains_key(&asset.id) {
            let img = asset.decode()?;
            // Keep GPU textures to a sane size; exports use the full image.
            let img = if img.width().max(img.height()) > 2048 {
                img.resize(2048, 2048, image::imageops::FilterType::Triangle)
            } else {
                img
            };
            let rgba = img.to_rgba8();
            let ci = egui::ColorImage::from_rgba_unmultiplied(
                [rgba.width() as usize, rgba.height() as usize],
                rgba.as_raw(),
            );
            let tex = ctx.load_texture(format!("img-{}", asset.id), ci, egui::TextureOptions::LINEAR);
            self.map.insert(asset.id, tex);
        }
        self.map.get(&asset.id)
    }
}

/// Draws a full page: paper, template and all elements.
/// `lod` renders strokes as simple polylines (for thumbnails).
pub fn draw_page(
    painter: &Painter,
    page: &Page,
    view: PageView,
    textures: &mut TextureCache,
    lod: bool,
    hidden: &[usize],
) {
    let page_rect = view.rect(Bounds {
        min: P::new(0.0, 0.0),
        max: P::new(page.width, page.height),
    });
    if !lod {
        painter.add(EShape::rect_filled(
            page_rect.translate(Vec2::new(0.0, 2.0)).expand(1.5),
            2.0,
            Color32::from_black_alpha(28),
        ));
    }
    painter.rect_filled(page_rect, 0.0, c32(page.paper));

    let clip = painter.clip_rect().intersect(page_rect);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    let painter = painter.with_clip_rect(clip);
    let visible = Bounds {
        min: view.to_page(clip.min),
        max: view.to_page(clip.max),
    };

    draw_template(&painter, page, view, visible, lod);

    let order = paint_order(page);
    for i in order {
        if hidden.contains(&i) {
            continue;
        }
        let el = &page.elements[i];
        if !el.bounds().intersects(&visible) {
            continue;
        }
        draw_element(&painter, el, view, textures, lod);
    }
}

/// Highlighter strokes are painted beneath everything else so they never
/// dull the ink they mark (same as on paper).
pub fn paint_order(page: &Page) -> Vec<usize> {
    let is_hl = |e: &Element| matches!(e, Element::Stroke(s) if s.kind == InkKind::Highlighter);
    let mut v: Vec<usize> = (0..page.elements.len()).filter(|&i| is_hl(&page.elements[i])).collect();
    v.extend((0..page.elements.len()).filter(|&i| !is_hl(&page.elements[i])));
    v
}

fn draw_template(painter: &Painter, page: &Page, view: PageView, visible: Bounds, lod: bool) {
    if lod && page.template == PageTemplate::Dotted {
        return;
    }
    let g = page.template_geometry();
    let w = (TEMPLATE_LINE_WIDTH * view.zoom).max(0.5);
    let stroke = EStroke::new(w, c32(TEMPLATE_LINE));
    for (a, b) in &g.lines {
        if a.y > visible.max.y || b.y < visible.min.y || a.x > visible.max.x || b.x < visible.min.x {
            continue;
        }
        painter.line_segment([view.to_screen(*a), view.to_screen(*b)], stroke);
    }
    if let Some((a, b)) = g.margin {
        painter.line_segment(
            [view.to_screen(a), view.to_screen(b)],
            EStroke::new(w, c32(TEMPLATE_MARGIN)),
        );
    }
    let r = (TEMPLATE_DOT_RADIUS * view.zoom).max(0.6);
    for d in &g.dots {
        if visible.expand(2.0).contains(*d) {
            painter.circle_filled(view.to_screen(*d), r, c32(TEMPLATE_LINE));
        }
    }
}

pub fn draw_element(painter: &Painter, el: &Element, view: PageView, textures: &mut TextureCache, lod: bool) {
    match el {
        Element::Stroke(s) => {
            if lod {
                let pts: Vec<Pos2> = s.samples.iter().step_by(4).map(|x| view.to_screen(x.pos)).collect();
                let avg_w = s.width * view.zoom * 0.8;
                if pts.len() >= 2 {
                    painter.add(EShape::line(pts, EStroke::new(avg_w.max(0.4), c32(s.color))));
                }
            } else {
                draw_ink(painter, &s.samples, view, c32(s.color));
            }
        }
        Element::Shape(s) => draw_shape(painter, s, view),
        Element::Text(t) => draw_text(painter, t, view),
        Element::Image(img) => {
            let rect = view.rect(img.bounds());
            if let Some(tex) = textures.get(painter.ctx(), &img.asset) {
                painter.image(
                    tex.id(),
                    rect,
                    Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
                    Color32::WHITE,
                );
            } else {
                painter.rect_stroke(rect, 0.0, EStroke::new(1.0, Color32::RED), egui::StrokeKind::Inside);
            }
        }
    }
}

pub fn draw_shape(painter: &Painter, s: &Shape, view: PageView) {
    let w = (s.width * view.zoom).max(0.5);
    let color = c32(s.color);
    if let (Some(fill), true) = (s.fill, s.fillable()) {
        let (pts, _) = &s.polylines()[0];
        let pts: Vec<Pos2> = pts.iter().map(|p| view.to_screen(*p)).collect();
        fill_polygon(painter, &pts, c32(fill));
    }
    for (pts, closed) in s.polylines() {
        let pts: Vec<Pos2> = pts.iter().map(|p| view.to_screen(*p)).collect();
        let path = if closed {
            PathShape::closed_line(pts, PathStroke::new(w, color))
        } else {
            PathShape::line(pts, PathStroke::new(w, color))
        };
        painter.add(path);
    }
    // round line ends, like the exported versions
    if matches!(s.kind, ShapeKind::Line | ShapeKind::Arrow) {
        for (pts, _) in s.polylines() {
            for p in [pts[0], pts[pts.len() - 1]] {
                painter.circle_filled(view.to_screen(p), w * 0.5, color);
            }
        }
    }
}

/// Builds an anti-aliased, variable-width ribbon mesh for the ink samples.
/// The edges get a one-pixel alpha feather, which gives smooth, crisp
/// strokes at any zoom without relying on MSAA.
pub fn draw_ink(painter: &Painter, samples: &[Sample], view: PageView, color: Color32) {
    if samples.is_empty() {
        return;
    }
    let mut mesh = Mesh::default();
    for piece in ink::pieces(samples) {
        ribbon(&mut mesh, piece, view, color);
    }
    painter.add(EShape::mesh(mesh));
}

const FEATHER: f32 = 0.5;

fn vtx(mesh: &mut Mesh, p: Pos2, c: Color32) -> u32 {
    let i = mesh.vertices.len() as u32;
    mesh.vertices.push(Vertex {
        pos: p,
        uv: WHITE_UV,
        color: c,
    });
    i
}

fn ribbon(mesh: &mut Mesh, piece: &[Sample], view: PageView, color: Color32) {
    if piece.is_empty() {
        return;
    }
    let zoom = view.zoom;
    // Hairlines thinner than a pixel are drawn one pixel wide but fainter.
    let max_w = piece.iter().map(|s| s.w).fold(0.0, f32::max) * zoom;
    let (color, min_half) = if max_w < 1.0 {
        (color.gamma_multiply(max_w.max(0.25)), 0.5)
    } else {
        (color, 0.0)
    };
    let transparent = Color32::TRANSPARENT;
    let half = |s: &Sample| (s.w * zoom * 0.5).max(min_half);

    let total: f32 = piece.windows(2).map(|w| dist(w[0].pos, w[1].pos)).sum::<f32>() * zoom;
    if piece.len() == 1 || total < 0.3 {
        let h = piece.iter().map(half).fold(0.0, f32::max);
        dot(mesh, view.to_screen(piece[0].pos), h, color);
        return;
    }

    let nrm = ink::normals(piece);
    let base = mesh.vertices.len() as u32;
    for (s, n) in piece.iter().zip(&nrm) {
        let c = view.to_screen(s.pos);
        let h = half(s);
        let n = Vec2::new(n.x, n.y);
        let hi = (h - FEATHER).max(0.0);
        let ho = h + FEATHER;
        vtx(mesh, c + n * ho, transparent); // 0 outer left
        vtx(mesh, c + n * hi, color); // 1 inner left
        vtx(mesh, c - n * hi, color); // 2 inner right
        vtx(mesh, c - n * ho, transparent); // 3 outer right
    }
    for i in 0..piece.len() as u32 - 1 {
        let a = base + i * 4;
        let b = a + 4;
        for k in 0..3 {
            mesh.add_triangle(a + k, a + k + 1, b + k);
            mesh.add_triangle(a + k + 1, b + k + 1, b + k);
        }
    }
    // round caps
    let first = piece[0];
    let last = piece[piece.len() - 1];
    let nf = nrm[0];
    let nl = nrm[nrm.len() - 1];
    cap(mesh, view.to_screen(last.pos), Vec2::new(nl.x, nl.y), half(&last), color);
    cap(mesh, view.to_screen(first.pos), Vec2::new(-nf.x, -nf.y), half(&first), color);
}

/// Semicircular feathered cap from `+n` to `-n`, bulging towards the stroke's
/// travel direction (see `ink::cap_arc`).
fn cap(mesh: &mut Mesh, c: Pos2, n: Vec2, h: f32, color: Color32) {
    let steps = ((h * 1.2) as usize).clamp(4, 24);
    let start = n.y.atan2(n.x);
    let hi = (h - FEATHER).max(0.0);
    let ho = h + FEATHER;
    let center = vtx(mesh, c, color);
    let mut prev: Option<(u32, u32)> = None;
    for i in 0..=steps {
        let a = start - std::f32::consts::PI * i as f32 / steps as f32;
        let d = Vec2::new(a.cos(), a.sin());
        let inner = vtx(mesh, c + d * hi, color);
        let outer = vtx(mesh, c + d * ho, Color32::TRANSPARENT);
        if let Some((pi, po)) = prev {
            mesh.add_triangle(center, pi, inner);
            mesh.add_triangle(pi, po, outer);
            mesh.add_triangle(pi, outer, inner);
        }
        prev = Some((inner, outer));
    }
}

fn dot(mesh: &mut Mesh, c: Pos2, h: f32, color: Color32) {
    let steps = ((h * 2.4) as usize).clamp(8, 48);
    let hi = (h - FEATHER).max(0.0);
    let ho = h + FEATHER;
    let center = vtx(mesh, c, color);
    let mut first: Option<(u32, u32)> = None;
    let mut prev: Option<(u32, u32)> = None;
    for i in 0..steps {
        let a = std::f32::consts::TAU * i as f32 / steps as f32;
        let d = Vec2::new(a.cos(), a.sin());
        let inner = vtx(mesh, c + d * hi, color);
        let outer = vtx(mesh, c + d * ho, Color32::TRANSPARENT);
        if let Some((pi, po)) = prev {
            mesh.add_triangle(center, pi, inner);
            mesh.add_triangle(pi, po, outer);
            mesh.add_triangle(pi, outer, inner);
        } else {
            first = Some((inner, outer));
        }
        prev = Some((inner, outer));
    }
    if let (Some((fi, fo)), Some((pi, po))) = (first, prev) {
        mesh.add_triangle(center, pi, fi);
        mesh.add_triangle(pi, po, fo);
        mesh.add_triangle(pi, fo, fi);
    }
}

pub const NOTES_FONT: &str = "notes";

pub fn notes_font(size: f32) -> egui::FontId {
    egui::FontId::new(size, egui::FontFamily::Name(NOTES_FONT.into()))
}

pub fn draw_text(painter: &Painter, t: &TextElem, view: PageView) {
    let size = t.size * view.zoom;
    if size < 1.0 {
        return;
    }
    for line in &t.layout.lines {
        if line.text.is_empty() {
            continue;
        }
        let p = view.to_screen(P::new(t.pos.x, t.pos.y + line.top + t.layout.pad));
        painter.text(p, egui::Align2::LEFT_TOP, &line.text, notes_font(size), c32(t.color));
    }
}

/// Fills any simple polygon (convex or not) using ear clipping.
pub fn fill_polygon(painter: &Painter, pts: &[Pos2], color: Color32) {
    if pts.len() < 3 {
        return;
    }
    let mut mesh = Mesh::default();
    for p in pts {
        mesh.colored_vertex(*p, color);
    }
    for [a, b, c] in triangulate(pts) {
        mesh.add_triangle(a, b, c);
    }
    painter.add(EShape::mesh(mesh));
}

fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let n = pts.len();
    let area: f32 = (0..n)
        .map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            a.x * b.y - b.x * a.y
        })
        .sum();
    let ccw = area > 0.0;
    let mut idx: Vec<usize> = (0..n).collect();
    let mut out = Vec::new();
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    let mut guard = 0;
    while idx.len() > 3 && guard < n * n {
        guard += 1;
        let m = idx.len();
        let mut clipped = false;
        for i in 0..m {
            let (ia, ib, ic) = (idx[(i + m - 1) % m], idx[i], idx[(i + 1) % m]);
            let (a, b, c) = (pts[ia], pts[ib], pts[ic]);
            let cr = cross(a, b, c);
            if (cr > 0.0) != ccw || cr.abs() < 1e-6 {
                continue;
            }
            let contains = idx.iter().any(|&j| {
                if j == ia || j == ib || j == ic {
                    return false;
                }
                let p = pts[j];
                let (d1, d2, d3) = (cross(a, b, p), cross(b, c, p), cross(c, a, p));
                if ccw { d1 > 0.0 && d2 > 0.0 && d3 > 0.0 } else { d1 < 0.0 && d2 < 0.0 && d3 < 0.0 }
            });
            if contains {
                continue;
            }
            out.push([ia as u32, ib as u32, ic as u32]);
            idx.remove(i);
            clipped = true;
            break;
        }
        if !clipped {
            break; // degenerate input: fall back to a fan for the rest
        }
    }
    for i in 1..idx.len().saturating_sub(1) {
        out.push([idx[0] as u32, idx[i] as u32, idx[i + 1] as u32]);
    }
    out
}
