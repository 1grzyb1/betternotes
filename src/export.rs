//! Exporters: vector PDF (pdf-writer) and raster PNG/JPEG (tiny-skia).
//! Both use the same stroke outlines as the screen renderer.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use miniz_oxide::deflate::compress_to_vec_zlib;
use pdf_writer::{Content, Filter, Name, Pdf, Rect, Ref};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, PixmapPaint, Transform};

use crate::ink;
use crate::text::{self, PathCmd};
use crate::model::*;
use crate::render::paint_order;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RasterFormat {
    Png,
    Jpeg,
}

impl RasterFormat {
    pub fn ext(self) -> &'static str {
        match self {
            RasterFormat::Png => "png",
            RasterFormat::Jpeg => "jpg",
        }
    }
}

// ---------------------------------------------------------------------------
// Raster
// ---------------------------------------------------------------------------

pub fn render_page_pixmap(page: &Page, scale: f32) -> Result<Pixmap, String> {
    let w = (page.width * scale).round().max(1.0) as u32;
    let h = (page.height * scale).round().max(1.0) as u32;
    let mut pm = Pixmap::new(w, h).ok_or("could not create image")?;
    let paper = page.paper;
    pm.fill(tiny_skia::Color::from_rgba8(paper.0[0], paper.0[1], paper.0[2], 255));
    let ts = Transform::from_scale(scale, scale);

    // template
    let g = page.template_geometry();
    let line_stroke = tiny_skia::Stroke {
        width: TEMPLATE_LINE_WIDTH,
        ..Default::default()
    };
    if !g.lines.is_empty() {
        let mut pb = PathBuilder::new();
        for (a, b) in &g.lines {
            pb.move_to(a.x, a.y);
            pb.line_to(b.x, b.y);
        }
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &paint(TEMPLATE_LINE), &line_stroke, ts, None);
        }
    }
    if let Some((a, b)) = g.margin {
        let mut pb = PathBuilder::new();
        pb.move_to(a.x, a.y);
        pb.line_to(b.x, b.y);
        if let Some(path) = pb.finish() {
            pm.stroke_path(&path, &paint(TEMPLATE_MARGIN), &line_stroke, ts, None);
        }
    }
    if !g.dots.is_empty() {
        let mut pb = PathBuilder::new();
        for d in &g.dots {
            pb.push_circle(d.x, d.y, TEMPLATE_DOT_RADIUS);
        }
        if let Some(path) = pb.finish() {
            pm.fill_path(&path, &paint(TEMPLATE_LINE), FillRule::Winding, ts, None);
        }
    }

    for i in paint_order(page) {
        match &page.elements[i] {
            Element::Stroke(s) => {
                let mut pb = PathBuilder::new();
                for poly in ink::outline_polygons(&s.samples) {
                    push_poly(&mut pb, &poly);
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint(s.color), FillRule::Winding, ts, None);
                }
            }
            Element::Shape(s) => {
                if let (Some(fill), true) = (s.fill, s.fillable()) {
                    let mut pb = PathBuilder::new();
                    push_poly(&mut pb, &s.polylines()[0].0);
                    if let Some(path) = pb.finish() {
                        pm.fill_path(&path, &paint(fill), FillRule::Winding, ts, None);
                    }
                }
                let mut pb = PathBuilder::new();
                for (pts, closed) in s.polylines() {
                    pb.move_to(pts[0].x, pts[0].y);
                    for p in &pts[1..] {
                        pb.line_to(p.x, p.y);
                    }
                    if closed {
                        pb.close();
                    }
                }
                if let Some(path) = pb.finish() {
                    let st = tiny_skia::Stroke {
                        width: s.width,
                        line_cap: LineCap::Round,
                        line_join: LineJoin::Round,
                        ..Default::default()
                    };
                    pm.stroke_path(&path, &paint(s.color), &st, ts, None);
                }
            }
            Element::Image(img) => {
                let Some(src) = image_pixmap(&img.asset) else { continue };
                let sx = img.w / src.width() as f32;
                let sy = img.h / src.height() as f32;
                let t = ts.pre_concat(Transform::from_row(sx, 0.0, 0.0, sy, img.pos.x, img.pos.y));
                let pp = PixmapPaint {
                    quality: tiny_skia::FilterQuality::Bicubic,
                    ..Default::default()
                };
                pm.draw_pixmap(0, 0, src.as_ref(), &pp, t, None);
            }
            Element::Text(t) => {
                let mut pb = PathBuilder::new();
                for cmd in text::outlines(&t.layout, t.pos, t.size) {
                    match cmd {
                        PathCmd::Move(p) => pb.move_to(p.x, p.y),
                        PathCmd::Line(p) => pb.line_to(p.x, p.y),
                        PathCmd::Quad(a, p) => pb.quad_to(a.x, a.y, p.x, p.y),
                        PathCmd::Cubic(a, b, p) => pb.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y),
                        PathCmd::Close => pb.close(),
                    }
                }
                if let Some(path) = pb.finish() {
                    pm.fill_path(&path, &paint(t.color), FillRule::Winding, ts, None);
                }
            }
        }
    }
    Ok(pm)
}

fn paint(c: Rgba) -> Paint<'static> {
    let mut p = Paint::default();
    p.set_color_rgba8(c.0[0], c.0[1], c.0[2], c.0[3]);
    p.anti_alias = true;
    p
}

fn push_poly(pb: &mut PathBuilder, poly: &[P]) {
    if poly.len() < 2 {
        return;
    }
    pb.move_to(poly[0].x, poly[0].y);
    for p in &poly[1..] {
        pb.line_to(p.x, p.y);
    }
    pb.close();
}

fn image_pixmap(asset: &ImageAsset) -> Option<Pixmap> {
    let img = asset.decode()?.to_rgba8();
    let mut pm = Pixmap::new(img.width(), img.height())?;
    for (dst, src) in pm.pixels_mut().iter_mut().zip(img.pixels()) {
        let [r, g, b, a] = src.0;
        *dst = tiny_skia::ColorU8::from_rgba(r, g, b, a).premultiply();
    }
    Some(pm)
}

/// Exports the given pages; one file per page (`name-1.png`, ...) when
/// there's more than one. Returns the written paths.
pub fn export_raster(
    pages: &[&Page],
    path: &Path,
    format: RasterFormat,
    scale: f32,
) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "notebook".into());
    let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
    for (i, page) in pages.iter().enumerate() {
        let out = if pages.len() == 1 {
            path.with_extension(format.ext())
        } else {
            dir.join(format!("{stem}-{}.{}", i + 1, format.ext()))
        };
        let pm = render_page_pixmap(page, scale)?;
        match format {
            RasterFormat::Png => pm.save_png(&out).map_err(|e| e.to_string())?,
            RasterFormat::Jpeg => {
                // page backgrounds are opaque, so un-premultiplying is lossless
                let mut rgb = image::RgbImage::new(pm.width(), pm.height());
                for (dst, src) in rgb.pixels_mut().zip(pm.pixels()) {
                    let c = src.demultiply();
                    *dst = image::Rgb([c.red(), c.green(), c.blue()]);
                }
                let file = std::fs::File::create(&out).map_err(|e| e.to_string())?;
                let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(std::io::BufWriter::new(file), 92);
                enc.encode_image(&rgb).map_err(|e| e.to_string())?;
            }
        }
        written.push(out);
    }
    Ok(written)
}

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

struct PdfImage {
    id: Ref,
    name: String,
}

pub fn export_pdf(pages: &[&Page], path: &Path) -> Result<(), String> {
    let mut pdf = Pdf::new();
    let mut next = 1;
    let mut alloc = || {
        let r = Ref::new(next);
        next += 1;
        r
    };
    let catalog_id = alloc();
    let tree_id = alloc();
    let page_ids: Vec<Ref> = pages.iter().map(|_| alloc()).collect();
    let content_ids: Vec<Ref> = pages.iter().map(|_| alloc()).collect();

    pdf.catalog(catalog_id).pages(tree_id);
    pdf.pages(tree_id).kids(page_ids.iter().copied()).count(pages.len() as i32);

    // Images are written once and shared by every page that uses them.
    let mut images: HashMap<u64, PdfImage> = HashMap::new();
    for page in pages {
        for el in &page.elements {
            if let Element::Image(img) = el {
                if images.contains_key(&img.asset.id) {
                    continue;
                }
                let id = alloc();
                let mask_id = alloc();
                write_pdf_image(&mut pdf, &img.asset, id, mask_id)?;
                let name = format!("Im{}", images.len() + 1);
                images.insert(img.asset.id, PdfImage { id, name });
            }
        }
    }

    // One ExtGState per distinct opacity.
    let mut alphas: HashMap<u8, (Ref, String)> = HashMap::new();
    for page in pages {
        for el in &page.elements {
            let mut want = |a: u8| {
                if a < 255 && !alphas.contains_key(&a) {
                    let r = alloc();
                    alphas.insert(a, (r, format!("Ga{a}")));
                }
            };
            match el {
                Element::Stroke(s) => want(s.color.0[3]),
                Element::Shape(s) => {
                    want(s.color.0[3]);
                    if let Some(f) = s.fill {
                        want(f.0[3]);
                    }
                }
                Element::Text(t) => want(t.color.0[3]),
                Element::Image(_) => {}
            }
        }
    }
    for (a, (r, _)) in &alphas {
        let v = *a as f32 / 255.0;
        pdf.ext_graphics(*r).non_stroking_alpha(v).stroking_alpha(v);
    }

    for (pi, page) in pages.iter().enumerate() {
        {
            let mut p = pdf.page(page_ids[pi]);
            p.media_box(Rect::new(0.0, 0.0, page.width, page.height));
            p.parent(tree_id);
            p.contents(content_ids[pi]);
            let mut res = p.resources();
            {
                let mut xo = res.x_objects();
                for img in images.values() {
                    xo.pair(Name(img.name.as_bytes()), img.id);
                }
            }
            {
                let mut gs = res.ext_g_states();
                for (r, name) in alphas.values() {
                    gs.pair(Name(name.as_bytes()), *r);
                }
            }
        }

        let mut c = Content::new();
        // Flip to a top-left origin so page units map 1:1.
        c.transform([1.0, 0.0, 0.0, -1.0, 0.0, page.height]);

        let (r, g, b) = page.paper.rgb_f32();
        c.set_fill_rgb(r, g, b);
        c.rect(0.0, 0.0, page.width, page.height);
        c.fill_nonzero();

        let tg = page.template_geometry();
        c.set_line_width(TEMPLATE_LINE_WIDTH);
        if !tg.lines.is_empty() {
            let (r, g, b) = TEMPLATE_LINE.rgb_f32();
            c.set_stroke_rgb(r, g, b);
            for (a, b) in &tg.lines {
                c.move_to(a.x, a.y);
                c.line_to(b.x, b.y);
            }
            c.stroke();
        }
        if let Some((a, b)) = tg.margin {
            let (r, g, bl) = TEMPLATE_MARGIN.rgb_f32();
            c.set_stroke_rgb(r, g, bl);
            c.move_to(a.x, a.y);
            c.line_to(b.x, b.y);
            c.stroke();
        }
        if !tg.dots.is_empty() {
            let (r, g, b) = TEMPLATE_LINE.rgb_f32();
            c.set_fill_rgb(r, g, b);
            for d in &tg.dots {
                pdf_poly(&mut c, &ink::circle(*d, TEMPLATE_DOT_RADIUS, 8));
            }
            c.fill_nonzero();
        }

        let set_alpha = |c: &mut Content, a: u8| {
            if let Some((_, name)) = alphas.get(&a) {
                c.set_parameters(Name(name.as_bytes()));
            }
        };

        for i in paint_order(page) {
            match &page.elements[i] {
                Element::Stroke(s) => {
                    c.save_state();
                    set_alpha(&mut c, s.color.0[3]);
                    let (r, g, b) = s.color.rgb_f32();
                    c.set_fill_rgb(r, g, b);
                    for poly in ink::outline_polygons(&s.samples) {
                        pdf_poly(&mut c, &poly);
                    }
                    c.fill_nonzero();
                    c.restore_state();
                }
                Element::Shape(s) => {
                    c.save_state();
                    if let (Some(fill), true) = (s.fill, s.fillable()) {
                        set_alpha(&mut c, fill.0[3]);
                        let (r, g, b) = fill.rgb_f32();
                        c.set_fill_rgb(r, g, b);
                        pdf_poly(&mut c, &s.polylines()[0].0);
                        c.fill_nonzero();
                        c.restore_state();
                        c.save_state();
                    }
                    set_alpha(&mut c, s.color.0[3]);
                    let (r, g, b) = s.color.rgb_f32();
                    c.set_stroke_rgb(r, g, b);
                    c.set_line_width(s.width);
                    c.set_line_cap(pdf_writer::types::LineCapStyle::RoundCap);
                    c.set_line_join(pdf_writer::types::LineJoinStyle::RoundJoin);
                    for (pts, closed) in s.polylines() {
                        c.move_to(pts[0].x, pts[0].y);
                        for p in &pts[1..] {
                            c.line_to(p.x, p.y);
                        }
                        if closed {
                            c.close_path();
                        }
                    }
                    c.stroke();
                    c.restore_state();
                }
                Element::Text(t) => {
                    c.save_state();
                    set_alpha(&mut c, t.color.0[3]);
                    let (r, g, b) = t.color.rgb_f32();
                    c.set_fill_rgb(r, g, b);
                    let mut cur = P::new(0.0, 0.0);
                    for cmd in text::outlines(&t.layout, t.pos, t.size) {
                        match cmd {
                            PathCmd::Move(p) => {
                                c.move_to(p.x, p.y);
                                cur = p;
                            }
                            PathCmd::Line(p) => {
                                c.line_to(p.x, p.y);
                                cur = p;
                            }
                            PathCmd::Quad(q, p) => {
                                // PDF has no quadratic curves: elevate to cubic
                                let c1 = P::new(cur.x + (q.x - cur.x) * 2.0 / 3.0, cur.y + (q.y - cur.y) * 2.0 / 3.0);
                                let c2 = P::new(p.x + (q.x - p.x) * 2.0 / 3.0, p.y + (q.y - p.y) * 2.0 / 3.0);
                                c.cubic_to(c1.x, c1.y, c2.x, c2.y, p.x, p.y);
                                cur = p;
                            }
                            PathCmd::Cubic(a, b, p) => {
                                c.cubic_to(a.x, a.y, b.x, b.y, p.x, p.y);
                                cur = p;
                            }
                            PathCmd::Close => {
                                c.close_path();
                            }
                        }
                    }
                    c.fill_nonzero();
                    c.restore_state();
                }
                Element::Image(img) => {
                    let Some(pi) = images.get(&img.asset.id) else { continue };
                    c.save_state();
                    c.transform([img.w, 0.0, 0.0, -img.h, img.pos.x, img.pos.y + img.h]);
                    c.x_object(Name(pi.name.as_bytes()));
                    c.restore_state();
                }
            }
        }
        pdf.stream(content_ids[pi], &c.finish());
    }

    std::fs::write(path, pdf.finish()).map_err(|e| e.to_string())
}

fn pdf_poly(c: &mut Content, poly: &[P]) {
    if poly.len() < 2 {
        return;
    }
    c.move_to(poly[0].x, poly[0].y);
    for p in &poly[1..] {
        c.line_to(p.x, p.y);
    }
    c.close_path();
}

fn write_pdf_image(pdf: &mut Pdf, asset: &ImageAsset, id: Ref, mask_id: Ref) -> Result<(), String> {
    let dynimg = asset.decode().ok_or("could not decode image")?;
    let (w, h) = (dynimg.width() as i32, dynimg.height() as i32);
    let is_jpeg = matches!(image::guess_format(&asset.bytes), Ok(image::ImageFormat::Jpeg));
    let level = 6;

    if is_jpeg && dynimg.color() == image::ColorType::Rgb8 {
        // JPEG data is valid DCT-encoded PDF image data as-is.
        let mut x = pdf.image_xobject(id, &asset.bytes);
        x.filter(Filter::DctDecode);
        x.width(w);
        x.height(h);
        x.color_space().device_rgb();
        x.bits_per_component(8);
        return Ok(());
    }

    let rgba = dynimg.to_rgba8();
    let has_alpha = rgba.pixels().any(|p| p.0[3] < 255);
    let rgb: Vec<u8> = rgba.pixels().flat_map(|p| [p.0[0], p.0[1], p.0[2]]).collect();
    let encoded = compress_to_vec_zlib(&rgb, level);
    {
        let mut x = pdf.image_xobject(id, &encoded);
        x.filter(Filter::FlateDecode);
        x.width(w);
        x.height(h);
        x.color_space().device_rgb();
        x.bits_per_component(8);
        x.interpolate(true);
        if has_alpha {
            x.s_mask(mask_id);
        }
    }
    if has_alpha {
        let alpha: Vec<u8> = rgba.pixels().map(|p| p.0[3]).collect();
        let enc = compress_to_vec_zlib(&alpha, level);
        let mut m = pdf.image_xobject(mask_id, &enc);
        m.filter(Filter::FlateDecode);
        m.width(w);
        m.height(h);
        m.color_space().device_gray();
        m.bits_per_component(8);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ink::InkPoint;

    /// Synthetic cursive handwriting: a looping signature-like curve with
    /// varying speed and pressure, plus some quick hatches.
    fn sample_page() -> Page {
        let mut page = Page::new(PageSize::A4, PageTemplate::Lined);
        let mut add = |pts: Vec<InkPoint>, real: bool, width: f32, kind: InkKind, color: Rgba| {
            let s = Stroke::new(kind, color, width, real, Default::default(), pts);
            page.elements.push(Element::Stroke(s));
        };
        for (row, real) in [(0, true), (1, false)] {
            let y0 = 120.0 + row as f32 * 72.0;
            let mut pts = Vec::new();
            let n = 420;
            for i in 0..n {
                let t = i as f32 / n as f32;
                let a = t * 34.0;
                // loops like "llllll" with a jittery hand (sensor noise at ~0.3pt)
                let noise = ((i * 7919) % 13) as f32 / 13.0 - 0.5;
                let x = 80.0 + t * 380.0 + a.cos() * 9.0 + noise * 0.3;
                let y = y0 - a.sin().abs() * 26.0 + noise * 0.3;
                let p = (0.25 + 0.7 * (t * std::f32::consts::PI).sin()) * (0.85 + 0.15 * (a * 0.5).sin());
                pts.push(InkPoint { x, y, p, t: i as f32 * 7.5 });
            }
            add(pts, real, 2.2, InkKind::Pen, Rgba::BLACK);
        }
        // zig-zag with sharp cusps ("vvvv")
        let mut pts = Vec::new();
        for i in 0..=16 {
            let x = 80.0 + i as f32 * 14.0;
            let y = if i % 2 == 0 { 250.0 } else { 280.0 };
            pts.push(InkPoint { x, y, p: 0.6, t: i as f32 * 40.0 });
        }
        add(pts, true, 2.0, InkKind::Pen, Rgba([30, 110, 235, 255]));
        // highlighter over text
        let pts = (0..60)
            .map(|i| InkPoint { x: 70.0 + i as f32 * 7.0, y: 112.0, p: 1.0, t: i as f32 * 8.0 })
            .collect();
        add(pts, false, 16.0, InkKind::Highlighter, Rgba([255, 214, 10, 110]));
        // a dot
        add(vec![InkPoint { x: 330.0, y: 265.0, p: 0.7, t: 0.0 }], true, 2.4, InkKind::Pen, Rgba::BLACK);

        page.elements.push(Element::Shape(Shape::new(
            ShapeKind::Ellipse,
            P::new(380.0, 230.0),
            P::new(520.0, 300.0),
            Rgba([220, 40, 50, 255]),
            1.5,
            Some(Rgba([220, 40, 50, 60])),
        )));
        page.elements.push(Element::Shape(Shape::new(
            ShapeKind::Arrow,
            P::new(80.0, 340.0),
            P::new(300.0, 360.0),
            Rgba::BLACK,
            1.5,
            None,
        )));
        page.elements.push(Element::Shape(Shape::new(
            ShapeKind::Triangle,
            P::new(320.0, 330.0),
            P::new(380.0, 380.0),
            Rgba([46, 160, 67, 255]),
            1.5,
            None,
        )));
        page.elements.push(Element::Text(TextElem::new(
            P::new(280.0, 420.0),
            "Lecture notes: zażółć gęślą jaźń. Text wraps automatically.".into(),
            14.0,
            Rgba([25, 55, 140, 255]),
            220.0,
        )));
        // partial erase through the middle of the first handwriting row
        if let Element::Stroke(s) = &page.elements[0] {
            let pieces = s.erase_partial(P::new(270.0, 80.0), P::new(270.0, 140.0), 6.0).unwrap();
            assert!(pieces.len() >= 2);
            page.elements.remove(0);
            for p in pieces {
                page.elements.insert(0, Element::Stroke(p));
            }
        }
        let rgba: Vec<u8> = (0..64 * 48)
            .flat_map(|i| {
                let (x, y) = (i % 64, i / 64);
                [(x * 4) as u8, (y * 5) as u8, 180, if x < 8 { 120 } else { 255 }]
            })
            .collect();
        let asset = ImageAsset::from_rgba(64, 48, rgba).unwrap();
        page.elements.push(Element::Image(ImageElem { asset, pos: P::new(80.0, 400.0), w: 160.0, h: 120.0 }));
        page
    }

    #[test]
    fn exports_all_formats() {
        let dir = std::env::temp_dir().join("betternotes-test");
        std::fs::create_dir_all(&dir).unwrap();
        let page = sample_page();
        let pages = vec![&page, &page];
        export_pdf(&pages, &dir.join("test.pdf")).unwrap();
        let files = export_raster(&pages, &dir.join("test.png"), RasterFormat::Png, 3.0).unwrap();
        assert_eq!(files.len(), 2);
        export_raster(&pages[..1], &dir.join("test.jpg"), RasterFormat::Jpeg, 2.0).unwrap();
        let pdf = std::fs::read(dir.join("test.pdf")).unwrap();
        assert!(pdf.starts_with(b"%PDF"));
        // round-trip through the notebook format
        let nb = Notebook { title: "t".into(), cover: COVER_COLORS[0], pages: vec![page] };
        nb.save(&dir.join("t.bnote")).unwrap();
        let back = Notebook::load(&dir.join("t.bnote")).unwrap();
        assert_eq!(back.pages[0].elements.len(), nb.pages[0].elements.len());
        println!("written to {}", dir.display());
    }
}
