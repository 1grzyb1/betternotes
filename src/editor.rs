//! Notebook editor: canvas with continuous page scrolling, drawing tools and
//! the floating toolbars around it.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{
    self, Align2, Color32, CornerRadius, CursorIcon, Event, Key, Margin, Modifiers, PointerButton, Pos2, Rect,
    RichText, Sense, Stroke as EStroke, Vec2,
};

use crate::export::{self, RasterFormat};
use crate::ink::InkPoint;
use crate::library;
use crate::model::*;
use crate::recognize;
use crate::render::{self, PageView, TextureCache, c32};
use crate::settings::Settings;
use crate::tablet::{self, SampleKind, TabletSample};
use crate::theme::{self, icon};

const PAGE_GAP: f32 = 28.0; // page units between pages
const DOC_MARGIN: f32 = 72.0; // screen points above the first page (room for the options bar)
const MAX_UNDO: usize = 100;
const CLIP_MARKER: &str = "\u{2063}betternotes-selection\u{2063}";
const HOLD_SECONDS: f64 = 0.55;
const AUTOSAVE_SECONDS: f64 = 4.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tool {
    Pen,
    Highlighter,
    Eraser,
    Lasso,
    Shape,
    Text,
    Hand,
}

impl Tool {
    const ALL: [Tool; 7] = [
        Tool::Pen,
        Tool::Highlighter,
        Tool::Eraser,
        Tool::Lasso,
        Tool::Shape,
        Tool::Text,
        Tool::Hand,
    ];

    fn icon(self) -> &'static str {
        match self {
            Tool::Pen => icon::PEN_NIB,
            Tool::Highlighter => icon::HIGHLIGHTER,
            Tool::Eraser => icon::ERASER,
            Tool::Lasso => icon::LASSO,
            Tool::Shape => icon::SHAPES,
            Tool::Text => icon::TEXT_T,
            Tool::Hand => icon::HAND,
        }
    }

    fn tooltip(self) -> &'static str {
        match self {
            Tool::Pen => "Pen (P)",
            Tool::Highlighter => "Highlighter (H)",
            Tool::Eraser => "Eraser (E)",
            Tool::Lasso => "Lasso select (L)",
            Tool::Shape => "Shapes (S)",
            Tool::Text => "Text (T)",
            Tool::Hand => "Pan (hold Space)",
        }
    }
}

fn shape_icon(k: ShapeKind) -> &'static str {
    match k {
        ShapeKind::Line => icon::LINE_SEGMENT,
        ShapeKind::Arrow => icon::ARROW_UP_RIGHT,
        ShapeKind::Rect => icon::SQUARE,
        ShapeKind::Ellipse => icon::CIRCLE,
        ShapeKind::Triangle => icon::TRIANGLE,
        ShapeKind::Polygon => icon::POLYGON,
    }
}

enum Active {
    Ink {
        page: usize,
        points: Vec<InkPoint>,
        t0: f64,
        preview: Stroke,
        hold_pos: Pos2,
        hold_t: f64,
        snapped: Option<Shape>,
    },
    Shape {
        page: usize,
        a: P,
        b: P,
    },
    Lasso {
        page: usize,
        pts: Vec<P>,
    },
    Move {
        last: P,
        moved: bool,
    },
    Scale {
        anchor: P,
        start: P,
        orig: Vec<Element>,
    },
    Erase {
        page: usize,
        last: P,
        checkpointed: bool,
    },
    Pan {
        last: Pos2,
    },
}

struct Selection {
    page: usize,
    items: Vec<usize>,
}

struct TextEditing {
    page: usize,
    /// Element being edited; `None` for a new text box.
    index: Option<usize>,
    pos: P,
    text: String,
    size: f32,
    color: Rgba,
    width: f32,
    /// Frames to wait before grabbing keyboard focus. The click that created
    /// the box must finish first, or egui treats it as a click elsewhere.
    focus_in: u8,
}

pub enum EditorAction {
    None,
    Close,
}

pub struct Editor {
    pub nb: Notebook,
    pub path: PathBuf,
    lib_dir: PathBuf,
    pub settings: Settings,
    dirty: bool,
    last_change: f64,

    tool: Tool,
    shape_kind: ShapeKind,
    shape_color: Rgba,
    shape_width: f32,
    shape_fill: bool,
    text_color: Rgba,

    zoom: f32,
    scroll: Vec2,
    canvas_rect: Rect,
    fit_on_next_frame: bool,

    active: Option<Active>,
    selection: Option<Selection>,
    editing: Option<TextEditing>,
    undo: Vec<Vec<Page>>,
    redo: Vec<Vec<Page>>,
    clipboard: Vec<Element>,

    textures: TextureCache,
    toast: Option<(String, f64)>,
    renaming: Option<String>,

    tablet_q: VecDeque<TabletSample>,
    primary_down: bool,
    touch_force: Option<f32>,
    last_pressure: Option<f32>,
    last_input_time: f64,
}

impl Editor {
    pub fn open(path: PathBuf, lib_dir: PathBuf, settings: Settings) -> Result<Self, String> {
        let nb = Notebook::load(&path)?;
        Ok(Editor {
            nb,
            path,
            lib_dir,
            text_color: settings.pen_color,
            settings,
            dirty: false,
            last_change: 0.0,
            tool: Tool::Pen,
            shape_kind: ShapeKind::Rect,
            shape_color: Rgba::BLACK,
            shape_width: 1.5,
            shape_fill: false,
            zoom: 1.0,
            scroll: Vec2::ZERO,
            canvas_rect: Rect::NOTHING,
            fit_on_next_frame: true,
            active: None,
            selection: None,
            editing: None,
            undo: Vec::new(),
            redo: Vec::new(),
            clipboard: Vec::new(),
            textures: TextureCache::default(),
            toast: None,
            renaming: None,
            tablet_q: VecDeque::new(),
            primary_down: false,
            touch_force: None,
            last_pressure: None,
            last_input_time: 0.0,
        })
    }

    // ------------------------------------------------------------------
    // document helpers
    // ------------------------------------------------------------------

    fn checkpoint(&mut self) {
        self.undo.push(self.nb.pages.clone());
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    fn undo(&mut self) {
        self.commit_text();
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.nb.pages, prev));
            self.after_history_jump();
        }
    }

    fn redo(&mut self) {
        self.commit_text();
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.nb.pages, next));
            self.after_history_jump();
        }
    }

    fn after_history_jump(&mut self) {
        self.selection = None;
        self.active = None;
        self.dirty = true;
    }

    fn flash(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ctx.input(|i| i.time)));
    }

    pub fn save(&mut self) -> Result<(), String> {
        self.commit_text();
        library::save_notebook(&self.lib_dir, &self.path, &self.nb)?;
        self.dirty = false;
        Ok(())
    }

    fn set_tool(&mut self, t: Tool) {
        self.commit_text();
        self.tool = t;
        if t != Tool::Lasso {
            self.selection = None;
        }
    }

    fn page_tops(&self) -> Vec<f32> {
        let mut y = 0.0;
        self.nb
            .pages
            .iter()
            .map(|p| {
                let top = y;
                y += p.height + PAGE_GAP;
                top
            })
            .collect()
    }

    fn doc_height(&self) -> f32 {
        self.nb.pages.iter().map(|p| p.height + PAGE_GAP).sum::<f32>() - PAGE_GAP
    }

    fn doc_width(&self) -> f32 {
        self.nb.pages.iter().map(|p| p.width).fold(0.0, f32::max)
    }

    fn page_view(&self, i: usize) -> PageView {
        let top = self.page_tops()[i];
        self.page_view_with_top(i, top)
    }

    fn page_view_with_top(&self, i: usize, top: f32) -> PageView {
        let page = &self.nb.pages[i];
        let r = self.canvas_rect;
        PageView {
            origin: Pos2::new(
                r.center().x - page.width * self.zoom * 0.5 - self.scroll.x,
                r.top() + DOC_MARGIN + top * self.zoom - self.scroll.y,
            ),
            zoom: self.zoom,
        }
    }

    fn page_screen_rect(&self, i: usize) -> Rect {
        let v = self.page_view(i);
        let p = &self.nb.pages[i];
        Rect::from_min_size(v.origin, Vec2::new(p.width, p.height) * self.zoom)
    }

    fn page_at(&self, pos: Pos2) -> Option<usize> {
        let tops = self.page_tops();
        (0..self.nb.pages.len()).find(|&i| {
            let v = self.page_view_with_top(i, tops[i]);
            let p = v.to_page(pos);
            let page = &self.nb.pages[i];
            p.x >= 0.0 && p.y >= 0.0 && p.x <= page.width && p.y <= page.height
        })
    }

    fn current_page(&self) -> usize {
        if self.nb.pages.is_empty() || self.canvas_rect.width() <= 0.0 {
            return 0;
        }
        let mid = (self.canvas_rect.center().y - self.canvas_rect.top() - DOC_MARGIN + self.scroll.y) / self.zoom;
        let tops = self.page_tops();
        let mut best = 0;
        for (i, &t) in tops.iter().enumerate() {
            if t <= mid {
                best = i;
            }
        }
        best
    }

    fn scroll_to_page(&mut self, i: usize) {
        if let Some(t) = self.page_tops().get(i) {
            self.scroll.y = t * self.zoom;
        }
        self.clamp_scroll();
    }

    fn clamp_scroll(&mut self) {
        let r = self.canvas_rect;
        let content_h = self.doc_height() * self.zoom + DOC_MARGIN + 80.0;
        let max_y = (content_h - r.height()).max(0.0);
        self.scroll.y = self.scroll.y.clamp(0.0, max_y);
        let content_w = self.doc_width() * self.zoom + 48.0;
        let max_x = ((content_w - r.width()) * 0.5).max(0.0);
        self.scroll.x = self.scroll.x.clamp(-max_x, max_x);
    }

    fn set_zoom(&mut self, new_zoom: f32, anchor: Pos2) {
        let new_zoom = new_zoom.clamp(0.2, 8.0);
        let r = self.canvas_rect;
        let base = Pos2::new(r.center().x, r.top() + DOC_MARGIN);
        let doc = (anchor - base + self.scroll) / self.zoom;
        self.zoom = new_zoom;
        self.scroll = doc * self.zoom - (anchor - base);
        self.clamp_scroll();
    }

    fn fit_width(&mut self) {
        let w = self.doc_width();
        if w > 0.0 && self.canvas_rect.width() > 0.0 {
            let cur = self.current_page();
            self.zoom = ((self.canvas_rect.width() - 64.0) / w).clamp(0.2, 8.0);
            self.scroll.x = 0.0;
            self.scroll_to_page(cur);
        }
    }

    fn selection_bounds(&self) -> Option<(usize, Bounds)> {
        let sel = self.selection.as_ref()?;
        let page = self.nb.pages.get(sel.page)?;
        let mut b = Bounds::empty();
        for &i in &sel.items {
            b.union(page.elements.get(i)?.bounds());
        }
        (!b.is_empty()).then_some((sel.page, b))
    }

    fn delete_selection(&mut self) {
        if let Some(sel) = self.selection.take() {
            self.checkpoint();
            let mut items = sel.items;
            items.sort_unstable_by(|a, b| b.cmp(a));
            for i in items {
                self.nb.pages[sel.page].elements.remove(i);
            }
        }
    }

    fn copy_selection(&mut self) -> bool {
        let Some(sel) = &self.selection else { return false };
        let page = &self.nb.pages[sel.page];
        self.clipboard = sel.items.iter().map(|&i| page.elements[i].clone()).collect();
        if let Ok(mut cb) = arboard::Clipboard::new() {
            let _ = cb.set_text(CLIP_MARKER);
        }
        true
    }

    fn paste_elements(&mut self, elements: Vec<Element>, offset: P) {
        if elements.is_empty() {
            return;
        }
        let page = self.selection.as_ref().map(|s| s.page).unwrap_or_else(|| self.current_page());
        self.checkpoint();
        let start = self.nb.pages[page].elements.len();
        for mut e in elements {
            e.transform(P::new(0.0, 0.0), 1.0, offset);
            self.nb.pages[page].elements.push(e);
        }
        let end = self.nb.pages[page].elements.len();
        self.selection = Some(Selection {
            page,
            items: (start..end).collect(),
        });
        self.tool = Tool::Lasso;
    }

    fn duplicate_selection(&mut self) {
        if let Some(sel) = &self.selection {
            let els: Vec<Element> = sel
                .items
                .iter()
                .map(|&i| self.nb.pages[sel.page].elements[i].clone())
                .collect();
            self.paste_elements(els, P::new(14.0, 14.0));
        }
    }

    fn recolor_selection(&mut self, c: Rgba) {
        if let Some(sel) = &self.selection {
            let (page, items) = (sel.page, sel.items.clone());
            self.checkpoint();
            for i in items {
                self.nb.pages[page].elements[i].set_color(c);
            }
        }
    }

    fn insert_image(&mut self, ctx: &egui::Context, asset: Arc<ImageAsset>) {
        self.commit_text();
        let page_i = self.current_page();
        let view = self.page_view(page_i);
        let page = &self.nb.pages[page_i];
        // 1 px = 0.75 pt (96 dpi), limited to 70% of the page
        let mut w = asset.px_w as f32 * 0.75;
        let mut h = asset.px_h as f32 * 0.75;
        let k = (page.width * 0.7 / w).min(page.height * 0.7 / h).min(1.0);
        w *= k;
        h *= k;
        let page_rect = self.page_screen_rect(page_i);
        let vis = page_rect.intersect(self.canvas_rect);
        let c = view.to_page(vis.center());
        let pos = P::new(
            (c.x - w * 0.5).clamp(0.0, (page.width - w).max(0.0)),
            (c.y - h * 0.5).clamp(0.0, (page.height - h).max(0.0)),
        );
        self.checkpoint();
        let els = &mut self.nb.pages[page_i].elements;
        els.push(Element::Image(ImageElem { asset, pos, w, h }));
        self.selection = Some(Selection {
            page: page_i,
            items: vec![els.len() - 1],
        });
        self.tool = Tool::Lasso;
        self.flash(ctx, "Image inserted. Drag to move, use the corner handle to resize");
    }

    fn insert_image_file(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        match std::fs::read(path).map_err(|e| e.to_string()).and_then(ImageAsset::from_encoded) {
            Ok(a) => self.insert_image(ctx, a),
            Err(e) => self.flash(ctx, format!("Could not load image: {e}")),
        }
    }

    fn pick_image(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Images", &["png", "jpg", "jpeg", "gif", "bmp", "webp", "tif", "tiff"])
            .pick_file()
        {
            self.insert_image_file(ctx, &path);
        }
    }

    fn paste(&mut self, ctx: &egui::Context) {
        let mut cb = arboard::Clipboard::new().ok();
        let internal = cb
            .as_mut()
            .and_then(|c| c.get_text().ok())
            .is_some_and(|t| t == CLIP_MARKER);
        if !internal {
            if let Some(img) = cb.as_mut().and_then(|c| c.get_image().ok()) {
                match ImageAsset::from_rgba(img.width as u32, img.height as u32, img.bytes.into_owned()) {
                    Ok(a) => self.insert_image(ctx, a),
                    Err(e) => self.flash(ctx, format!("Clipboard image error: {e}")),
                }
                return;
            }
            if let Some(files) = cb.as_mut().and_then(|c| c.get().file_list().ok()) {
                if let Some(f) = files.first() {
                    let f = f.clone();
                    self.insert_image_file(ctx, &f);
                    return;
                }
            }
        }
        if !self.clipboard.is_empty() {
            let els = self.clipboard.clone();
            self.paste_elements(els, P::new(14.0, 14.0));
        } else {
            self.flash(ctx, "Clipboard has no image");
        }
    }

    fn export(&mut self, ctx: &egui::Context, kind: &str, only_current: bool) {
        self.commit_text();
        let (filter, ext): (&str, &str) = match kind {
            "pdf" => ("PDF", "pdf"),
            "png" => ("PNG", "png"),
            _ => ("JPEG", "jpg"),
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter(filter, &[ext])
            .set_file_name(format!("{}.{ext}", self.nb.title))
            .save_file()
        else {
            return;
        };
        let pages: Vec<&Page> = if only_current {
            vec![&self.nb.pages[self.current_page()]]
        } else {
            self.nb.pages.iter().collect()
        };
        let result = match kind {
            "pdf" => export::export_pdf(&pages, &path).map(|_| vec![path.clone()]),
            "png" => export::export_raster(&pages, &path, RasterFormat::Png, 3.0),
            _ => export::export_raster(&pages, &path, RasterFormat::Jpeg, 3.0),
        };
        match result {
            Ok(files) if files.len() == 1 => self.flash(ctx, format!("Exported: {}", files[0].display())),
            Ok(files) => self.flash(ctx, format!("Exported {} files", files.len())),
            Err(e) => self.flash(ctx, format!("Export failed: {e}")),
        }
    }

    // ------------------------------------------------------------------
    // pages
    // ------------------------------------------------------------------

    fn add_page_after(&mut self, i: usize) {
        self.commit_text();
        let at = (i + 1).min(self.nb.pages.len());
        let (size, template) = (self.settings.new_page_size, self.settings.new_page_template);
        self.checkpoint();
        self.nb.pages.insert(at, Page::new(size, template));
        self.selection = None;
        self.scroll_to_page(at);
    }

    fn duplicate_page(&mut self, i: usize) {
        self.commit_text();
        self.checkpoint();
        let p = self.nb.pages[i].clone();
        self.nb.pages.insert(i + 1, p);
        self.scroll_to_page(i + 1);
    }

    fn delete_page(&mut self, i: usize) {
        self.commit_text();
        self.checkpoint();
        if self.nb.pages.len() <= 1 {
            self.nb.pages[0].elements.clear();
        } else {
            self.nb.pages.remove(i);
        }
        self.selection = None;
        self.clamp_scroll();
    }

    fn move_page(&mut self, i: usize, up: bool) {
        let j = if up { i.wrapping_sub(1) } else { i + 1 };
        if j < self.nb.pages.len() {
            self.commit_text();
            self.checkpoint();
            self.nb.pages.swap(i, j);
            self.selection = None;
            self.scroll_to_page(j);
        }
    }

    fn set_template(&mut self, i: usize, t: PageTemplate) {
        if self.nb.pages[i].template != t {
            self.checkpoint();
            self.nb.pages[i].template = t;
        }
        self.settings.new_page_template = t;
    }

    // ------------------------------------------------------------------
    // text
    // ------------------------------------------------------------------

    fn begin_text(&mut self, page: usize, p: P) {
        self.commit_text();
        // edit an existing box if one was clicked
        let hit = self.nb.pages[page]
            .elements
            .iter()
            .enumerate()
            .rev()
            .find(|(_, e)| matches!(e, Element::Text(_)) && e.hit(p, 2.0))
            .map(|(i, _)| i);
        if let Some(i) = hit {
            if let Element::Text(t) = &self.nb.pages[page].elements[i] {
                self.editing = Some(TextEditing {
                    page,
                    index: Some(i),
                    pos: t.pos,
                    text: t.text.clone(),
                    size: t.size,
                    color: t.color,
                    width: t.width,
                    focus_in: 2,
                });
            }
            return;
        }
        let size = self.settings.text_size;
        let page_w = self.nb.pages[page].width;
        let pos = P::new(p.x, p.y - size * text_center_offset());
        self.editing = Some(TextEditing {
            page,
            index: None,
            pos,
            text: String::new(),
            size,
            color: self.text_color,
            width: (page_w - pos.x - 24.0).clamp(60.0, 420.0),
            focus_in: 2,
        });
    }

    fn commit_text(&mut self) {
        let Some(ed) = self.editing.take() else { return };
        let empty = ed.text.trim().is_empty();
        match ed.index {
            Some(i) => {
                let Some(Element::Text(old)) = self.nb.pages.get(ed.page).and_then(|p| p.elements.get(i)) else {
                    return;
                };
                if old.text == ed.text && old.color == ed.color && old.size == ed.size {
                    return;
                }
                self.checkpoint();
                if empty {
                    self.nb.pages[ed.page].elements.remove(i);
                } else if let Element::Text(t) = &mut self.nb.pages[ed.page].elements[i] {
                    t.text = ed.text;
                    t.color = ed.color;
                    t.size = ed.size;
                    t.rebuild();
                }
            }
            None if !empty => {
                self.checkpoint();
                let t = TextElem::new(ed.pos, ed.text, ed.size, ed.color, ed.width);
                self.nb.pages[ed.page].elements.push(Element::Text(t));
            }
            None => {}
        }
    }
}

/// Where the click lands relative to a text line (roughly mid x-height).
fn text_center_offset() -> f32 {
    crate::text::LINE_HEIGHT * 0.5
}

// ----------------------------------------------------------------------
// canvas input
// ----------------------------------------------------------------------

impl Editor {
    fn take_sample(&mut self, kind: SampleKind) -> Option<f32> {
        let sample = match kind {
            SampleKind::Drag => {
                if self.tablet_q.front().is_some_and(|s| s.kind == SampleKind::Drag) {
                    self.tablet_q.pop_front()
                } else {
                    None
                }
            }
            _ => {
                let mut found = None;
                while let Some(s) = self.tablet_q.pop_front() {
                    if s.kind == kind {
                        found = Some(s);
                        break;
                    }
                }
                found
            }
        };
        match sample {
            Some(s) if s.is_tablet => {
                self.last_pressure = Some(s.pressure);
                Some(s.pressure)
            }
            _ => self.touch_force,
        }
    }

    fn handle_canvas_input(&mut self, ui: &mut egui::Ui, resp: &egui::Response) {
        let ctx = ui.ctx().clone();
        let canvas_layer = ui.layer_id();

        let (zoom_delta, scroll_delta, hover) =
            ui.input(|i| (i.zoom_delta(), i.smooth_scroll_delta, i.pointer.hover_pos()));
        if resp.contains_pointer() {
            if zoom_delta != 1.0 {
                let anchor = hover.unwrap_or(self.canvas_rect.center());
                self.set_zoom(self.zoom * zoom_delta, anchor);
            }
            if scroll_delta != Vec2::ZERO {
                self.scroll -= scroll_delta;
            }
        }
        self.clamp_scroll();

        self.tablet_q.extend(tablet::drain());
        let (events, now, space_held) =
            ui.input(|i| (i.events.clone(), i.time, i.key_down(Key::Space) && !i.modifiers.any()));
        let space_held = space_held && self.editing.is_none();
        let pointer_events = events
            .iter()
            .filter(|e| matches!(e, Event::PointerMoved(_) | Event::PointerButton { .. }))
            .count()
            .max(1);
        // Events carry no timestamps; spread them evenly over the frame.
        let t_start = self.last_input_time.max(now - 0.05);
        let mut k = 0usize;
        let no_focus = ctx.memory(|m| m.focused().is_none());

        for ev in events {
            match ev {
                Event::Touch { force, phase, .. } => {
                    self.touch_force = match phase {
                        egui::TouchPhase::End | egui::TouchPhase::Cancel => None,
                        _ => force.filter(|f| *f > 0.0),
                    };
                }
                Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers,
                } => {
                    k += 1;
                    let t = t_start + (now - t_start) * k as f64 / pointer_events as f64;
                    match (button, pressed) {
                        (PointerButton::Primary, true) => {
                            let pr = self.take_sample(SampleKind::Down);
                            self.primary_down = true;
                            let on_canvas =
                                self.canvas_rect.contains(pos) && ctx.layer_id_at(pos) == Some(canvas_layer);
                            if on_canvas && self.active.is_none() {
                                self.on_press(pos, pr, t, space_held);
                            }
                        }
                        (PointerButton::Primary, false) => {
                            let pr = self.take_sample(SampleKind::Up);
                            self.primary_down = false;
                            self.on_move(pos, pr, t, modifiers);
                            self.on_release();
                        }
                        (PointerButton::Middle, true) if self.canvas_rect.contains(pos) => {
                            self.active = Some(Active::Pan { last: pos });
                        }
                        (PointerButton::Middle, false) => {
                            if matches!(self.active, Some(Active::Pan { .. })) {
                                self.active = None;
                            }
                        }
                        _ => {}
                    }
                }
                Event::PointerMoved(pos) => {
                    k += 1;
                    let t = t_start + (now - t_start) * k as f64 / pointer_events as f64;
                    let pr = if self.primary_down {
                        self.take_sample(SampleKind::Drag)
                    } else {
                        None
                    };
                    let mods = ui.input(|i| i.modifiers);
                    self.on_move(pos, pr, t, mods);
                }
                Event::Copy if no_focus => {
                    self.copy_selection();
                }
                Event::Cut if no_focus => {
                    if self.copy_selection() {
                        self.delete_selection();
                    }
                }
                #[cfg(not(target_os = "macos"))]
                Event::Paste(_) if no_focus => self.paste(&ctx),
                _ => {}
            }
        }
        self.last_input_time = now;

        if tablet::take_paste_request() && no_focus {
            self.paste(&ctx);
        }

        // draw-and-hold shape recognition
        if self.settings.shape_hold {
            if let Some(Active::Ink {
                page,
                preview,
                hold_t,
                snapped,
                ..
            }) = &mut self.active
            {
                if snapped.is_none() && preview.kind == InkKind::Pen && preview.samples.len() > 3 {
                    if now - *hold_t > HOLD_SECONDS {
                        let pts: Vec<P> = preview.samples.iter().map(|s| s.pos).collect();
                        if let Some(r) = recognize::recognize(&pts) {
                            let mut s = Shape::new(r.kind, r.a, r.b, preview.color, (preview.width * 0.75).max(0.6), None);
                            s.pts = r.pts;
                            *snapped = Some(s);
                        } else {
                            // don't retry on every frame while the pen keeps resting
                            *hold_t = f64::INFINITY;
                        }
                        let _ = page;
                    } else {
                        ctx.request_repaint_after(std::time::Duration::from_millis(60));
                    }
                }
            }
        }

        // safety net: pointer released outside the window
        if self.active.is_some()
            && !matches!(self.active, Some(Active::Pan { .. }))
            && !ui.input(|i| i.pointer.primary_down())
            && !self.primary_down
        {
            self.on_release();
        }
        if matches!(self.active, Some(Active::Pan { .. }))
            && !ui.input(|i| i.pointer.primary_down() || i.pointer.middle_down())
        {
            self.active = None;
        }

        let dropped: Vec<PathBuf> = ui.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for path in dropped {
            if !path.as_os_str().is_empty() {
                self.insert_image_file(&ctx, &path);
            }
        }
    }

    fn on_press(&mut self, pos: Pos2, pressure: Option<f32>, t: f64, space: bool) {
        if space || self.tool == Tool::Hand {
            self.active = Some(Active::Pan { last: pos });
            return;
        }

        if self.tool == Tool::Lasso {
            if let Some((page, b)) = self.selection_bounds() {
                let view = self.page_view(page);
                let r = view.rect(b).expand(6.0);
                let p = view.to_page(pos);
                if pos.distance(r.max) < 14.0 {
                    let orig = self
                        .selection
                        .as_ref()
                        .unwrap()
                        .items
                        .iter()
                        .map(|&i| self.nb.pages[page].elements[i].clone())
                        .collect();
                    self.checkpoint();
                    self.active = Some(Active::Scale {
                        anchor: b.min,
                        start: b.max,
                        orig,
                    });
                    return;
                }
                if r.contains(pos) {
                    self.checkpoint();
                    self.active = Some(Active::Move { last: p, moved: false });
                    return;
                }
            }
        }

        let Some(page) = self.page_at(pos) else {
            self.commit_text();
            if self.tool == Tool::Lasso {
                self.selection = None;
            }
            return;
        };
        let p = self.page_view(page).to_page(pos);
        if self.tool != Tool::Text {
            self.commit_text();
        }
        match self.tool {
            Tool::Pen | Tool::Highlighter => {
                self.selection = None;
                let s = &self.settings;
                let (kind, color, width) = if self.tool == Tool::Pen {
                    (InkKind::Pen, s.pen_color, s.pen_width)
                } else {
                    (InkKind::Highlighter, s.highlighter_color, s.highlighter_width)
                };
                let first = InkPoint {
                    x: p.x,
                    y: p.y,
                    p: pressure.unwrap_or(0.5),
                    t: 0.0,
                };
                let preview = Stroke::new(kind, color, width, pressure.is_some(), s.ink, vec![first]);
                self.active = Some(Active::Ink {
                    page,
                    points: vec![first],
                    t0: t,
                    preview,
                    hold_pos: pos,
                    hold_t: t,
                    snapped: None,
                });
            }
            Tool::Shape => {
                self.selection = None;
                self.active = Some(Active::Shape { page, a: p, b: p });
            }
            Tool::Lasso => {
                self.selection = None;
                self.active = Some(Active::Lasso { page, pts: vec![p] });
            }
            Tool::Eraser => {
                self.selection = None;
                self.active = Some(Active::Erase {
                    page,
                    last: p,
                    checkpointed: false,
                });
                self.erase_segment(p, p);
            }
            Tool::Text => {
                self.selection = None;
                self.begin_text(page, p);
            }
            Tool::Hand => {}
        }
    }

    fn on_move(&mut self, pos: Pos2, pressure: Option<f32>, t: f64, mods: Modifiers) {
        let zoom = self.zoom;
        let Some(active) = self.active.as_ref() else { return };
        let page = match active {
            Active::Ink { page, .. } | Active::Shape { page, .. } | Active::Lasso { page, .. } | Active::Erase { page, .. } => *page,
            Active::Move { .. } | Active::Scale { .. } => self.selection.as_ref().map(|s| s.page).unwrap_or(0),
            Active::Pan { .. } => 0,
        };
        if page >= self.nb.pages.len() {
            self.active = None;
            return;
        }
        let view = self.page_view(page);
        let p = view.to_page(pos);
        let constrained = match self.active.as_ref() {
            Some(Active::Shape { a, .. }) if mods.shift => Some(self.constrain_shape(*a, p)),
            _ => None,
        };

        match self.active.as_mut().unwrap() {
            Active::Ink {
                points,
                t0,
                preview,
                hold_pos,
                hold_t,
                snapped,
                ..
            } => {
                if snapped.is_some() {
                    return;
                }
                if pos.distance(*hold_pos) > 3.0 {
                    *hold_pos = pos;
                    *hold_t = t;
                }
                let last = *points.last().unwrap();
                if dist(P::new(last.x, last.y), p) < 0.25 / zoom {
                    return;
                }
                let pr = if preview.real_pressure { pressure.unwrap_or(last.p) } else { 0.5 };
                points.push(InkPoint {
                    x: p.x,
                    y: p.y,
                    p: pr,
                    t: ((t - *t0) * 1000.0) as f32,
                });
                preview.points.clone_from(points);
                preview.rebuild();
            }
            Active::Shape { b, .. } => {
                *b = constrained.unwrap_or(p);
            }
            Active::Lasso { pts, .. } => {
                if pts.last().is_none_or(|l| dist(*l, p) > 1.5 / zoom) {
                    pts.push(p);
                }
            }
            Active::Move { last, moved } => {
                let d = P::new(p.x - last.x, p.y - last.y);
                *last = p;
                *moved = true;
                if let Some(sel) = &self.selection {
                    for &i in &sel.items {
                        self.nb.pages[sel.page].elements[i].transform(P::new(0.0, 0.0), 1.0, d);
                    }
                }
            }
            Active::Scale { anchor, start, orig } => {
                let d0 = dist(*anchor, *start).max(0.001);
                let (dx, dy) = (start.x - anchor.x, start.y - anchor.y);
                let proj = ((p.x - anchor.x) * dx + (p.y - anchor.y) * dy) / d0;
                let s = (proj / d0).clamp(0.05, 20.0);
                let anchor = *anchor;
                if let Some(sel) = &self.selection {
                    for (k, &i) in sel.items.iter().enumerate() {
                        let mut e = orig[k].clone();
                        e.transform(anchor, s, P::new(0.0, 0.0));
                        self.nb.pages[sel.page].elements[i] = e;
                    }
                }
            }
            Active::Erase { last, .. } => {
                let from = *last;
                *last = p;
                self.erase_segment(from, p);
            }
            Active::Pan { last } => {
                let d = pos - *last;
                *last = pos;
                self.scroll -= d;
                self.clamp_scroll();
            }
        }
    }

    fn constrain_shape(&self, a: P, b: P) -> P {
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        match self.shape_kind {
            ShapeKind::Line | ShapeKind::Arrow => {
                let len = (dx * dx + dy * dy).sqrt();
                let step = std::f32::consts::PI / 12.0;
                let ang = (dy.atan2(dx) / step).round() * step;
                P::new(a.x + len * ang.cos(), a.y + len * ang.sin())
            }
            _ => {
                let s = dx.abs().max(dy.abs());
                P::new(a.x + s * dx.signum(), a.y + s * dy.signum())
            }
        }
    }

    fn erase_segment(&mut self, from: P, to: P) {
        let Some(Active::Erase { page, checkpointed, .. }) = &self.active else { return };
        let (page, already) = (*page, *checkpointed);
        let r = self.settings.eraser_radius / self.zoom;
        let whole = self.settings.eraser_whole;

        // compute the edit first, apply after the (lazy) undo checkpoint
        let mut edits: Vec<(usize, Option<Vec<Stroke>>)> = Vec::new();
        for (i, e) in self.nb.pages[page].elements.iter().enumerate() {
            match e {
                Element::Image(_) | Element::Text(_) => {}
                Element::Stroke(s) if !whole => {
                    if let Some(pieces) = s.erase_partial(from, to, r) {
                        edits.push((i, Some(pieces)));
                    }
                }
                _ => {
                    let len = dist(from, to);
                    let steps = ((len / (r * 0.5)).ceil() as usize).clamp(1, 200);
                    let hit = (0..=steps).any(|k| {
                        let t = k as f32 / steps as f32;
                        e.hit(P::new(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t), r)
                    });
                    if hit {
                        edits.push((i, None));
                    }
                }
            }
        }
        if edits.is_empty() {
            return;
        }
        if !already {
            self.checkpoint();
            if let Some(Active::Erase { checkpointed, .. }) = &mut self.active {
                *checkpointed = true;
            }
        }
        let els = &mut self.nb.pages[page].elements;
        for (i, pieces) in edits.into_iter().rev() {
            els.remove(i);
            if let Some(pieces) = pieces {
                for (k, s) in pieces.into_iter().enumerate() {
                    els.insert(i + k, Element::Stroke(s));
                }
            }
        }
        self.dirty = true;
    }

    fn on_release(&mut self) {
        let Some(active) = self.active.take() else { return };
        match active {
            Active::Ink {
                page,
                mut preview,
                snapped,
                ..
            } => {
                self.checkpoint();
                if let Some(shape) = snapped {
                    self.nb.pages[page].elements.push(Element::Shape(shape));
                } else {
                    preview.rebuild();
                    self.nb.pages[page].elements.push(Element::Stroke(preview));
                }
            }
            Active::Shape { page, a, b } => {
                if dist(a, b) * self.zoom > 3.0 {
                    let fill = self.shape_fill.then(|| self.shape_color.with_alpha(70));
                    self.checkpoint();
                    self.nb.pages[page].elements.push(Element::Shape(Shape::new(
                        self.shape_kind,
                        a,
                        b,
                        self.shape_color,
                        self.shape_width,
                        fill,
                    )));
                }
            }
            Active::Lasso { page, pts } => {
                let items: Vec<usize> = self.nb.pages[page]
                    .elements
                    .iter()
                    .enumerate()
                    .filter(|(_, e)| {
                        let probes = e.probe_points();
                        if probes.is_empty() {
                            return false;
                        }
                        let inside = probes.iter().filter(|p| point_in_polygon(**p, &pts)).count();
                        inside * 2 >= probes.len()
                    })
                    .map(|(i, _)| i)
                    .collect();
                self.selection = (!items.is_empty()).then_some(Selection { page, items });
            }
            Active::Move { moved, .. } => {
                if !moved {
                    self.undo.pop();
                }
            }
            Active::Scale { .. } | Active::Erase { .. } | Active::Pan { .. } => {}
        }
    }
}

// ----------------------------------------------------------------------
// UI
// ----------------------------------------------------------------------

const PEN_COLORS: [Rgba; 8] = [
    Rgba([20, 20, 24, 255]),
    Rgba([25, 55, 140, 255]),
    Rgba([30, 110, 235, 255]),
    Rgba([0, 150, 136, 255]),
    Rgba([46, 160, 67, 255]),
    Rgba([220, 40, 50, 255]),
    Rgba([245, 125, 20, 255]),
    Rgba([140, 60, 200, 255]),
];

const HL_COLORS: [Rgba; 6] = [
    Rgba([255, 214, 10, 110]),
    Rgba([120, 230, 90, 110]),
    Rgba([90, 200, 250, 110]),
    Rgba([255, 120, 190, 110]),
    Rgba([255, 160, 60, 110]),
    Rgba([180, 140, 255, 110]),
];

/// Rainbow swatch that opens a colour picker. Returns the picked colour.
fn custom_color(ui: &mut egui::Ui, current: Rgba) -> Option<Rgba> {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let p = ui.painter();
    let c = rect.center();
    let n = 24;
    for i in 0..n {
        let a0 = i as f32 / n as f32 * std::f32::consts::TAU;
        let a1 = (i + 1) as f32 / n as f32 * std::f32::consts::TAU;
        let col: Color32 = egui::ecolor::Hsva::new(i as f32 / n as f32, 0.75, 0.95, 1.0).into();
        let r = if resp.hovered() { 10.0 } else { 9.5 };
        p.add(egui::Shape::convex_polygon(
            vec![c, c + Vec2::angled(a0) * r, c + Vec2::angled(a1) * r],
            col,
            EStroke::NONE,
        ));
    }
    p.circle_filled(c, 4.0, Color32::WHITE);
    let mut out = None;
    egui::Popup::from_toggle_button_response(&resp)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            let mut col = c32(current.with_alpha(255));
            if egui::color_picker::color_picker_color32(ui, &mut col, egui::color_picker::Alpha::Opaque) {
                out = Some(Rgba([col.r(), col.g(), col.b(), 255]));
            }
        });
    resp.on_hover_text("Custom color");
    out
}

impl Editor {
    /// Draws the editor; returns `Close` when the user goes back to the library.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> EditorAction {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        let mut action = EditorAction::None;

        if self.dirty && self.last_change == 0.0 {
            self.last_change = now;
        }
        if !self.dirty {
            self.last_change = 0.0;
        }
        if self.dirty && self.active.is_none() && self.editing.is_none() && now - self.last_change > AUTOSAVE_SECONDS {
            if let Err(e) = self.save() {
                self.flash(&ctx, format!("Autosave failed: {e}"));
            }
        }
        if self.dirty {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(AUTOSAVE_SECONDS));
        }

        if self.shortcuts(&ctx) {
            action = EditorAction::Close;
        }

        egui::Panel::top("editor-top")
            .frame(
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .inner_margin(Margin::symmetric(12, 7))
                    .stroke(EStroke::new(1.0, theme::BORDER)),
            )
            .show(ui, |ui| {
                if self.top_bar(ui) {
                    action = EditorAction::Close;
                }
            });
        if self.settings.sidebar_open {
            egui::Panel::left("pages")
                .frame(
                    egui::Frame::new()
                        .fill(theme::SURFACE)
                        .inner_margin(Margin::symmetric(12, 12))
                        .stroke(EStroke::new(1.0, theme::BORDER)),
                )
                .resizable(true)
                .default_size(196.0)
                .min_size(140.0)
                .max_size(320.0)
                .show(ui, |ui| self.sidebar(ui));
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::CANVAS_BG))
            .show(ui, |ui| self.canvas(ui));

        self.options_bar(&ctx);
        self.zoom_bar(&ctx);
        self.text_editor_overlay(&ctx);

        if let Some((msg, t)) = &self.toast {
            if now - t < 4.0 {
                theme::toast(&ctx, Pos2::new(self.canvas_rect.center().x, self.canvas_rect.bottom() - 70.0), msg);
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else {
                self.toast = None;
            }
        }
        action
    }

    /// Returns true when the user asked to go back to the library.
    fn top_bar(&mut self, ui: &mut egui::Ui) -> bool {
        let ctx = ui.ctx().clone();
        let mut close = false;
        let full = ui.available_rect_before_wrap();
        let full = Rect::from_min_size(full.min, Vec2::new(full.width(), 36.0));
        ui.allocate_rect(full, Sense::hover());

        // left: back + title
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if ui
                    .add(
                        egui::Button::new((theme::ico(icon::CARET_LEFT, 16.0).color(theme::ACCENT), RichText::new("Library").font(theme::medium(14.0)).color(theme::ACCENT)))
                            .frame_when_inactive(false)
                            .min_size(Vec2::new(0.0, 32.0)),
                    )
                    .on_hover_text("Back to library (saves the notebook)")
                    .clicked()
                {
                    close = true;
                }
                theme::vsep(ui);
                match &mut self.renaming {
                    Some(name) => {
                        let r = ui.add(
                            egui::TextEdit::singleline(name)
                                .font(theme::semibold(15.0))
                                .desired_width(220.0)
                                .margin(Margin::symmetric(6, 4)),
                        );
                        r.request_focus();
                        if r.lost_focus() {
                            let n = name.trim().to_string();
                            if !n.is_empty() && n != self.nb.title {
                                self.nb.title = n;
                                self.dirty = true;
                            }
                            self.renaming = None;
                        }
                    }
                    None => {
                        let r = ui
                            .add(
                                egui::Label::new(RichText::new(&self.nb.title).font(theme::semibold(15.0)).color(theme::TEXT))
                                    .sense(Sense::click())
                                    .truncate(),
                            )
                            .on_hover_text("Click to rename");
                        if r.clicked() {
                            self.renaming = Some(self.nb.title.clone());
                        }
                        if r.hovered() {
                            ctx.set_cursor_icon(CursorIcon::Text);
                        }
                    }
                }
                let status = if self.dirty { "Unsaved changes…" } else { "Saved" };
                ui.label(RichText::new(status).small().color(theme::MUTED));
            },
        );

        // centre: tools
        let tools_w = Tool::ALL.len() as f32 * 42.0 + 8.0 + 52.0;
        let center = Rect::from_center_size(full.center(), Vec2::new(tools_w, 36.0));
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(center)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                egui::Frame::new()
                    .fill(Color32::from_rgb(241, 242, 245))
                    .corner_radius(CornerRadius::same(11))
                    .inner_margin(Margin::same(2))
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.horizontal(|ui| {
                            for t in Tool::ALL {
                                if theme::tool_button(ui, t.icon(), t.tooltip(), self.tool == t).clicked() {
                                    self.set_tool(t);
                                }
                            }
                            theme::vsep(ui);
                            if theme::tool_button(ui, icon::IMAGE, "Insert image (⌘V pastes from clipboard)", false).clicked() {
                                self.pick_image(&ctx);
                            }
                        });
                    });
            },
        );

        // right: history, export, view
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(full)
                .layout(egui::Layout::right_to_left(egui::Align::Center)),
            |ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let more = theme::icon_button(ui, icon::DOTS_THREE_OUTLINE_VERTICAL, "More");
                egui::Popup::menu(&more).show(|ui| self.more_menu(ui));
                let exp = theme::icon_button(ui, icon::EXPORT, "Export");
                egui::Popup::menu(&exp).show(|ui| self.export_menu(ui));
                if theme::tool_button(ui, icon::SIDEBAR_SIMPLE, "Pages panel", self.settings.sidebar_open).clicked() {
                    self.settings.sidebar_open = !self.settings.sidebar_open;
                }
                theme::vsep(ui);
                ui.add_enabled_ui(!self.redo.is_empty(), |ui| {
                    if theme::icon_button(ui, icon::ARROW_CLOCKWISE, "Redo (⇧⌘Z)").clicked() {
                        self.redo();
                    }
                });
                ui.add_enabled_ui(!self.undo.is_empty(), |ui| {
                    if theme::icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, "Undo (⌘Z)").clicked() {
                        self.undo();
                    }
                });
            },
        );
        close
    }

    fn export_menu(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.label(RichText::new("Whole notebook").small().color(theme::MUTED));
        if theme::menu_item(ui, icon::FILE_PDF, "PDF").clicked() {
            ui.close();
            self.export(&ctx, "pdf", false);
        }
        if theme::menu_item(ui, icon::FILE_PNG, "PNG (one file per page)").clicked() {
            ui.close();
            self.export(&ctx, "png", false);
        }
        if theme::menu_item(ui, icon::FILE_JPG, "JPG (one file per page)").clicked() {
            ui.close();
            self.export(&ctx, "jpg", false);
        }
        ui.separator();
        ui.label(RichText::new("Current page").small().color(theme::MUTED));
        if theme::menu_item(ui, icon::FILE_PDF, "PDF").clicked() {
            ui.close();
            self.export(&ctx, "pdf", true);
        }
        if theme::menu_item(ui, icon::FILE_PNG, "PNG").clicked() {
            ui.close();
            self.export(&ctx, "png", true);
        }
        if theme::menu_item(ui, icon::FILE_JPG, "JPG").clicked() {
            ui.close();
            self.export(&ctx, "jpg", true);
        }
    }

    fn more_menu(&mut self, ui: &mut egui::Ui) {
        let cur = self.current_page();
        ui.label(RichText::new("Current page paper").small().color(theme::MUTED));
        let mut t = self.nb.pages[cur].template;
        theme::segmented(ui, &mut t, &PageTemplate::ALL.map(|t| (t, t.label())));
        if t != self.nb.pages[cur].template {
            self.set_template(cur, t);
        }
        ui.add_space(4.0);
        ui.label(RichText::new("New page size").small().color(theme::MUTED));
        theme::segmented(ui, &mut self.settings.new_page_size, &PageSize::ALL.map(|s| (s, s.label())));
        ui.separator();
        if theme::menu_item(ui, icon::PLUS, "Add page").clicked() {
            self.add_page_after(cur);
            ui.close();
        }
        if theme::menu_item(ui, icon::CORNERS_OUT, "Fit to width").clicked() {
            self.fit_width();
            ui.close();
        }
        if theme::menu_item(ui, icon::FOLDER_OPEN, crate::library::REVEAL_LABEL).clicked() {
            library::reveal(&self.path);
            ui.close();
        }
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let current = self.current_page();
        ui.horizontal(|ui| {
            ui.label(RichText::new("Pages").font(theme::semibold(14.0)));
            ui.label(RichText::new(format!("{}", self.nb.pages.len())).color(theme::MUTED));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::icon_button(ui, icon::PLUS, "Add page after current").clicked() {
                    self.add_page_after(current);
                }
            });
        });
        ui.add_space(6.0);

        enum PageCmd {
            Go(usize),
            AddAfter(usize),
            Duplicate(usize),
            Up(usize),
            Down(usize),
            Template(usize, PageTemplate),
            Delete(usize),
        }
        let mut cmd = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            ui.add_space(6.0);
            let thumb_w = (ui.available_width() - 24.0).max(60.0);
            for i in 0..self.nb.pages.len() {
                let page = &self.nb.pages[i];
                let h = thumb_w * page.height / page.width;
                ui.vertical_centered(|ui| {
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(thumb_w, h), Sense::click());
                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter_at(rect.expand(16.0));
                        painter.add(theme::card_shadow(rect, 4, resp.hovered()));
                        let view = PageView {
                            origin: rect.min,
                            zoom: thumb_w / page.width,
                        };
                        render::draw_page(&painter.with_clip_rect(rect), page, view, &mut self.textures, true, &[]);
                        if i == current {
                            painter.rect_stroke(rect.expand(3.0), 6.0, EStroke::new(2.5, theme::ACCENT), egui::StrokeKind::Inside);
                        } else {
                            painter.rect_stroke(rect, 3.0, EStroke::new(1.0, theme::BORDER), egui::StrokeKind::Outside);
                        }
                    }
                    if resp.clicked() {
                        cmd = Some(PageCmd::Go(i));
                    }
                    if resp.hovered() {
                        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                    }
                    resp.context_menu(|ui| {
                        if theme::menu_item(ui, icon::PLUS, "Add page after").clicked() {
                            cmd = Some(PageCmd::AddAfter(i));
                        }
                        if theme::menu_item(ui, icon::COPY, "Duplicate page").clicked() {
                            cmd = Some(PageCmd::Duplicate(i));
                        }
                        if theme::menu_item(ui, icon::ARROW_UP, "Move up").clicked() {
                            cmd = Some(PageCmd::Up(i));
                        }
                        if theme::menu_item(ui, icon::ARROW_DOWN, "Move down").clicked() {
                            cmd = Some(PageCmd::Down(i));
                        }
                        ui.separator();
                        ui.label(RichText::new("Paper").small().color(theme::MUTED));
                        for t in PageTemplate::ALL {
                            let sel = self.nb.pages[i].template == t;
                            if ui.add(egui::Button::selectable(sel, t.label()).min_size(Vec2::new(200.0, 26.0))).clicked() {
                                cmd = Some(PageCmd::Template(i, t));
                            }
                        }
                        ui.separator();
                        if theme::menu_item_colored(ui, icon::TRASH, "Delete page", theme::DANGER).clicked()
                        {
                            cmd = Some(PageCmd::Delete(i));
                        }
                        if cmd.is_some() {
                            ui.close();
                        }
                    });
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(format!("{}", i + 1))
                            .small()
                            .color(if i == current { theme::ACCENT } else { theme::MUTED }),
                    );
                });
                ui.add_space(4.0);
            }
            ui.vertical_centered(|ui| {
                if theme::secondary_button(ui, Some(icon::PLUS), "Add page").clicked() {
                    cmd = Some(PageCmd::AddAfter(self.nb.pages.len() - 1));
                }
            });
            ui.add_space(12.0);
        });
        match cmd {
            Some(PageCmd::Go(i)) => self.scroll_to_page(i),
            Some(PageCmd::AddAfter(i)) => self.add_page_after(i),
            Some(PageCmd::Duplicate(i)) => self.duplicate_page(i),
            Some(PageCmd::Up(i)) => self.move_page(i, true),
            Some(PageCmd::Down(i)) => self.move_page(i, false),
            Some(PageCmd::Template(i, t)) => self.set_template(i, t),
            Some(PageCmd::Delete(i)) => self.delete_page(i),
            None => {}
        }
    }

    fn color_row(&mut self, ui: &mut egui::Ui, palette: &[Rgba], current: Rgba) -> Option<Rgba> {
        let mut picked = None;
        for &c in palette {
            if theme::color_swatch(ui, c, current.with_alpha(255) == c.with_alpha(255)).clicked() {
                picked = Some(c);
            }
        }
        let recent: Vec<Rgba> = self
            .settings
            .recent_colors
            .iter()
            .copied()
            .filter(|c| !palette.iter().any(|p| p.with_alpha(255) == *c))
            .collect();
        for c in recent {
            if theme::color_swatch(ui, c, current.with_alpha(255) == c).clicked() {
                picked = Some(c);
            }
        }
        if let Some(c) = custom_color(ui, current) {
            self.settings.remember_color(c);
            picked = Some(c);
        }
        picked
    }

    /// Contextual options for the active tool, floating over the canvas.
    fn options_bar(&mut self, ctx: &egui::Context) {
        let pos = Pos2::new(self.canvas_rect.center().x, self.canvas_rect.top() + 12.0);
        egui::Area::new(egui::Id::new("options-bar"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_TOP)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                theme::pill_frame().show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.horizontal(|ui| self.options_contents(ui));
                });
            });
    }

    fn options_contents(&mut self, ui: &mut egui::Ui) {
        match self.tool {
            Tool::Pen => {
                if let Some(c) = self.color_row(ui, &PEN_COLORS, self.settings.pen_color) {
                    self.settings.pen_color = c;
                }
                theme::vsep(ui);
                for w in [0.9, 1.4, 1.8, 2.6, 4.0] {
                    if theme::width_dot(ui, w * 1.2, (self.settings.pen_width - w).abs() < 0.01, self.settings.pen_color)
                        .on_hover_text(format!("{w} pt"))
                        .clicked()
                    {
                        self.settings.pen_width = w;
                    }
                }
                theme::vsep(ui);
                let r = theme::icon_button(ui, icon::SLIDERS_HORIZONTAL, "Pen settings");
                egui::Popup::from_toggle_button_response(&r)
                    .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                    .show(|ui| self.pen_settings(ui));
            }
            Tool::Highlighter => {
                if let Some(c) = self.color_row(ui, &HL_COLORS, self.settings.highlighter_color) {
                    self.settings.highlighter_color = c.with_alpha(110);
                }
                theme::vsep(ui);
                for w in [8.0, 12.0, 16.0, 22.0] {
                    if theme::width_dot(ui, w * 0.38, (self.settings.highlighter_width - w).abs() < 0.01, self.settings.highlighter_color)
                        .clicked()
                    {
                        self.settings.highlighter_width = w;
                    }
                }
            }
            Tool::Eraser => {
                let mut whole = self.settings.eraser_whole;
                theme::segmented(ui, &mut whole, &[(false, "Partial"), (true, "Whole strokes")]);
                self.settings.eraser_whole = whole;
                theme::vsep(ui);
                for (r, label) in [(5.0, "Small"), (10.0, "Medium"), (20.0, "Large")] {
                    if theme::width_dot(ui, r * 0.4 + 2.0, (self.settings.eraser_radius - r).abs() < 0.01, Rgba([120, 124, 134, 255]))
                        .on_hover_text(label)
                        .clicked()
                    {
                        self.settings.eraser_radius = r;
                    }
                }
            }
            Tool::Lasso => {
                if self.selection.is_some() {
                    if let Some(c) = self.color_row(ui, &PEN_COLORS, Rgba([0, 0, 0, 0])) {
                        self.recolor_selection(c);
                    }
                    theme::vsep(ui);
                    if theme::icon_button(ui, icon::COPY, "Duplicate (⌘D)").clicked() {
                        self.duplicate_selection();
                    }
                    if theme::icon_button(ui, icon::CLIPBOARD_TEXT, "Copy (⌘C)").clicked() {
                        self.copy_selection();
                    }
                    if ui
                        .add(
                            egui::Button::new(theme::ico(icon::TRASH, 18.0).color(theme::DANGER))
                                .frame_when_inactive(false)
                                .min_size(Vec2::new(34.0, 32.0)),
                        )
                        .on_hover_text("Delete (⌫)")
                        .clicked()
                    {
                        self.delete_selection();
                    }
                } else {
                    theme::hint(ui, icon::LASSO, "Circle items to select them");
                }
            }
            Tool::Shape => {
                for k in ShapeKind::TOOLS {
                    if theme::tool_button(ui, shape_icon(k), k.label(), self.shape_kind == k).clicked() {
                        self.shape_kind = k;
                    }
                }
                theme::vsep(ui);
                if let Some(c) = self.color_row(ui, &PEN_COLORS[..6], self.shape_color) {
                    self.shape_color = c;
                }
                theme::vsep(ui);
                for w in [1.0, 1.5, 2.5, 4.0] {
                    if theme::width_dot(ui, w * 1.2, (self.shape_width - w).abs() < 0.01, self.shape_color).clicked() {
                        self.shape_width = w;
                    }
                }
                theme::vsep(ui);
                if theme::tool_button(ui, icon::PAINT_BUCKET, "Fill", self.shape_fill).clicked() {
                    self.shape_fill = !self.shape_fill;
                }
            }
            Tool::Text => {
                let current = self.editing.as_ref().map(|e| e.color).unwrap_or(self.text_color);
                if let Some(c) = self.color_row(ui, &PEN_COLORS, current) {
                    self.text_color = c;
                    if let Some(e) = &mut self.editing {
                        e.color = c;
                        e.focus_in = 1;
                    }
                }
                theme::vsep(ui);
                let mut size = self.editing.as_ref().map(|e| e.size).unwrap_or(self.settings.text_size);
                if theme::segmented(ui, &mut size, &[(11.0, "S"), (14.0, "M"), (18.0, "L"), (26.0, "XL")]) {
                    self.settings.text_size = size;
                    if let Some(e) = &mut self.editing {
                        e.size = size;
                        e.focus_in = 1;
                    }
                }
            }
            Tool::Hand => {
                theme::hint(ui, icon::HAND, "Drag to scroll · ⌘ + wheel or pinch to zoom");
            }
        }
    }

    fn pen_settings(&mut self, ui: &mut egui::Ui) {
        ui.set_width(300.0);
        ui.label(RichText::new("Pen settings").font(theme::semibold(15.0)));
        ui.add_space(4.0);
        let s = &mut self.settings;
        egui::Grid::new("pen-settings").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label("Thickness");
            ui.add(egui::Slider::new(&mut s.pen_width, 0.3..=12.0).max_decimals(1).suffix(" pt"));
            ui.end_row();
            ui.label("Smoothing");
            ui.add(egui::Slider::new(&mut s.ink.smoothing, 0.0..=1.0).show_value(false));
            ui.end_row();
            ui.label("Thinning");
            ui.add(egui::Slider::new(&mut s.ink.thinning, 0.0..=0.95).show_value(false));
            ui.end_row();
            ui.label("Pressure sensitivity");
            // higher sensitivity = lower exponent (light touches get thicker)
            let mut sens = 2.0 - s.ink.pressure_gamma;
            if ui.add(egui::Slider::new(&mut sens, 0.2..=1.7).show_value(false)).changed() {
                s.ink.pressure_gamma = 2.0 - sens;
            }
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.checkbox(&mut s.shape_hold, "Draw and hold → perfect shape");
        ui.add_space(4.0);
        ui.horizontal(|ui| match self.last_pressure {
            Some(p) => theme::hint(ui, icon::PEN_NIB, &format!("Tablet detected · pressure {p:.2}")),
            None => theme::hint(ui, icon::CURSOR, "Mouse/trackpad · pressure simulated from speed"),
        });
        if ui.button("Restore defaults").clicked() {
            self.settings.ink = Default::default();
        }
    }

    fn zoom_bar(&mut self, ctx: &egui::Context) {
        let r = self.canvas_rect;
        egui::Area::new(egui::Id::new("zoom-bar"))
            .order(egui::Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(Pos2::new(r.right() - 16.0, r.bottom() - 16.0))
            .show(ctx, |ui| {
                theme::pill_frame().inner_margin(Margin::symmetric(4, 3)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal(|ui| {
                        if theme::icon_button(ui, icon::MINUS, "Zoom out (⌘−)").clicked() {
                            let c = self.canvas_rect.center();
                            self.set_zoom(self.zoom / 1.2, c);
                        }
                        let label = ui.add(
                            egui::Button::new(RichText::new(format!("{:.0}%", self.zoom * 100.0)).font(theme::medium(13.0)))
                                .frame_when_inactive(false)
                                .min_size(Vec2::new(54.0, 32.0)),
                        );
                        if label.on_hover_text("Fit to width (⌘0)").clicked() {
                            self.fit_width();
                        }
                        if theme::icon_button(ui, icon::PLUS, "Zoom in (⌘+)").clicked() {
                            let c = self.canvas_rect.center();
                            self.set_zoom(self.zoom * 1.2, c);
                        }
                    });
                });
            });

        let cur = self.current_page();
        egui::Area::new(egui::Id::new("page-bar"))
            .order(egui::Order::Foreground)
            .pivot(Align2::CENTER_BOTTOM)
            .fixed_pos(Pos2::new(r.center().x, r.bottom() - 16.0))
            .show(ctx, |ui| {
                theme::pill_frame().inner_margin(Margin::symmetric(4, 3)).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.horizontal(|ui| {
                        ui.add_enabled_ui(cur > 0, |ui| {
                            if theme::icon_button(ui, icon::CARET_UP, "Previous page").clicked() {
                                self.scroll_to_page(cur - 1);
                            }
                        });
                        ui.add_sized(
                            Vec2::new(86.0, 32.0),
                            egui::Label::new(
                                RichText::new(format!("{} / {}", cur + 1, self.nb.pages.len())).font(theme::medium(13.0)),
                            ),
                        );
                        ui.add_enabled_ui(cur + 1 < self.nb.pages.len(), |ui| {
                            if theme::icon_button(ui, icon::CARET_DOWN, "Next page").clicked() {
                                self.scroll_to_page(cur + 1);
                            }
                        });
                        theme::vsep(ui);
                        if theme::icon_button(ui, icon::FILE_PLUS, "Add page").clicked() {
                            self.add_page_after(cur);
                        }
                    });
                });
            });
    }

    fn text_editor_overlay(&mut self, ctx: &egui::Context) {
        let Some(page_i) = self.editing.as_ref().map(|e| e.page) else { return };
        if page_i >= self.nb.pages.len() {
            self.editing = None;
            return;
        }
        let view = self.page_view(page_i);
        let zoom = self.zoom;
        let ed = self.editing.as_mut().unwrap();
        let size = ed.size * zoom;
        let line_h = size * crate::text::LINE_HEIGHT;
        let color = c32(ed.color);
        let width = ed.width * zoom;
        let mut commit = false;
        let screen = view.to_screen(ed.pos);
        egui::Area::new(egui::Id::new("text-edit"))
            .order(egui::Order::Foreground)
            .fixed_pos(screen)
            .show(ctx, |ui| {
                let font = render::notes_font(size);
                let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
                    let mut job = egui::text::LayoutJob::single_section(
                        buf.as_str().to_string(),
                        egui::TextFormat {
                            font_id: font.clone(),
                            color,
                            line_height: Some(line_h),
                            ..Default::default()
                        },
                    );
                    job.wrap.max_width = wrap;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let out = egui::TextEdit::multiline(&mut ed.text)
                    .frame(egui::Frame::NONE)
                    .margin(Margin::ZERO)
                    .desired_width(width)
                    .desired_rows(1)
                    .layouter(&mut layouter)
                    .show(ui);
                let rect = out.response.rect.expand(4.0);
                ui.painter().extend(egui::Shape::dashed_line(
                    &[rect.left_top(), rect.right_top(), rect.right_bottom(), rect.left_bottom(), rect.left_top()],
                    EStroke::new(1.0, theme::ACCENT),
                    4.0,
                    3.0,
                ));
                if ed.focus_in > 0 {
                    ed.focus_in -= 1;
                    if ed.focus_in == 0 {
                        out.response.request_focus();
                    }
                    ctx.request_repaint();
                } else if out.response.lost_focus() || !out.response.has_focus() {
                    commit = true;
                }
            });
        if commit {
            self.commit_text();
        }
    }

    fn canvas(&mut self, ui: &mut egui::Ui) {
        let rect = ui.max_rect();
        self.canvas_rect = rect;
        if self.fit_on_next_frame && rect.width() > 100.0 {
            self.fit_on_next_frame = false;
            let w = self.doc_width();
            self.zoom = ((rect.width() - 64.0) / w).clamp(0.2, 1.5);
        }
        let resp = ui.allocate_rect(rect, Sense::click_and_drag());
        self.handle_canvas_input(ui, &resp);

        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 0.0, theme::CANVAS_BG);

        let tops = self.page_tops();
        let hidden: Vec<(usize, usize)> = self
            .editing
            .as_ref()
            .and_then(|e| e.index.map(|i| (e.page, i)))
            .into_iter()
            .collect();
        for i in 0..self.nb.pages.len() {
            let view = self.page_view_with_top(i, tops[i]);
            let page = &self.nb.pages[i];
            let pr = Rect::from_min_size(view.origin, Vec2::new(page.width, page.height) * self.zoom);
            if !pr.intersects(rect) {
                continue;
            }
            let hide: Vec<usize> = hidden.iter().filter(|(p, _)| *p == i).map(|(_, e)| *e).collect();
            render::draw_page(&painter, page, view, &mut self.textures, false, &hide);
        }

        match &self.active {
            Some(Active::Ink {
                page, preview, snapped, ..
            }) => {
                let view = self.page_view(*page);
                let p = painter.with_clip_rect(self.page_screen_rect(*page).intersect(rect));
                match snapped {
                    Some(s) => render::draw_shape(&p, s, view),
                    None => render::draw_ink(&p, &preview.samples, view, c32(preview.color)),
                }
            }
            Some(Active::Shape { page, a, b }) => {
                let view = self.page_view(*page);
                let s = Shape::new(
                    self.shape_kind,
                    *a,
                    *b,
                    self.shape_color,
                    self.shape_width,
                    self.shape_fill.then(|| self.shape_color.with_alpha(70)),
                );
                render::draw_shape(&painter, &s, view);
            }
            Some(Active::Lasso { page, pts }) => {
                let view = self.page_view(*page);
                let mut sp: Vec<Pos2> = pts.iter().map(|p| view.to_screen(*p)).collect();
                if let Some(f) = sp.first().copied() {
                    sp.push(f);
                }
                if sp.len() > 2 {
                    painter.extend(egui::Shape::dashed_line(&sp, EStroke::new(1.5, theme::ACCENT), 6.0, 4.0));
                }
            }
            _ => {}
        }

        if let Some((page, b)) = self.selection_bounds() {
            let view = self.page_view(page);
            let r = view.rect(b).expand(6.0);
            painter.rect_filled(r, 6.0, theme::ACCENT.gamma_multiply(0.06));
            let pts = [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom(), r.left_top()];
            painter.extend(egui::Shape::dashed_line(&pts, EStroke::new(1.2, theme::ACCENT), 5.0, 4.0));
            painter.circle_filled(r.max, 7.0, Color32::WHITE);
            painter.circle_stroke(r.max, 7.0, EStroke::new(2.0, theme::ACCENT));
        }

        if let Some(hover) = ui.input(|i| i.pointer.hover_pos()) {
            if rect.contains(hover) && ui.ctx().layer_id_at(hover) == Some(ui.layer_id()) {
                let space = ui.input(|i| i.key_down(Key::Space)) && self.editing.is_none();
                let icon = if space || self.tool == Tool::Hand {
                    if matches!(self.active, Some(Active::Pan { .. })) {
                        CursorIcon::Grabbing
                    } else {
                        CursorIcon::Grab
                    }
                } else {
                    match self.tool {
                        Tool::Pen | Tool::Highlighter | Tool::Shape => CursorIcon::Crosshair,
                        Tool::Text => CursorIcon::Text,
                        Tool::Eraser => {
                            painter.circle_filled(hover, self.settings.eraser_radius, Color32::from_white_alpha(90));
                            painter.circle_stroke(hover, self.settings.eraser_radius, EStroke::new(1.2, Color32::from_gray(90)));
                            CursorIcon::None
                        }
                        Tool::Lasso => {
                            let over_sel = self.selection_bounds().is_some_and(|(p, b)| {
                                let r = self.page_view(p).rect(b).expand(6.0);
                                r.contains(hover) || hover.distance(r.max) < 14.0
                            });
                            if over_sel { CursorIcon::Move } else { CursorIcon::Crosshair }
                        }
                        Tool::Hand => CursorIcon::Grab,
                    }
                };
                ui.ctx().set_cursor_icon(icon);
            }
        }

        // scrollbar indicator
        let content_h = self.doc_height() * self.zoom + DOC_MARGIN + 80.0;
        if content_h > rect.height() {
            let frac = rect.height() / content_h;
            let bar_h = (rect.height() * frac).max(30.0);
            let t = self.scroll.y / (content_h - rect.height());
            let y = rect.top() + 4.0 + (rect.height() - bar_h - 8.0) * t;
            let bar = Rect::from_min_size(Pos2::new(rect.right() - 9.0, y), Vec2::new(5.0, bar_h));
            painter.rect_filled(bar, 3.0, Color32::from_black_alpha(55));
        }
    }

    /// Keyboard shortcuts. Returns true for "back to library".
    fn shortcuts(&mut self, ctx: &egui::Context) -> bool {
        use egui::KeyboardShortcut as KS;
        let sc = |m: Modifiers, k: Key| ctx.input_mut(|i| i.consume_shortcut(&KS::new(m, k)));
        let cmd = Modifiers::COMMAND;
        let cmd_shift = Modifiers::COMMAND | Modifiers::SHIFT;

        if self.editing.is_some() {
            if sc(Modifiers::NONE, Key::Escape) {
                self.commit_text();
            }
            return false;
        }
        if ctx.memory(|m| m.focused().is_some()) {
            return false;
        }
        if sc(cmd_shift, Key::Z) || sc(cmd, Key::Y) {
            self.redo();
        }
        if sc(cmd, Key::Z) {
            self.undo();
        }
        if sc(cmd, Key::S) {
            match self.save() {
                Ok(()) => self.flash(ctx, "Saved"),
                Err(e) => self.flash(ctx, format!("Save failed: {e}")),
            }
        }
        if sc(cmd, Key::E) {
            self.export(ctx, "pdf", false);
        }
        if sc(cmd, Key::D) {
            self.duplicate_selection();
        }
        if sc(cmd, Key::Num0) {
            self.fit_width();
        }
        if sc(cmd, Key::Equals) || sc(cmd, Key::Plus) {
            let c = self.canvas_rect.center();
            self.set_zoom(self.zoom * 1.2, c);
        }
        if sc(cmd, Key::Minus) {
            let c = self.canvas_rect.center();
            self.set_zoom(self.zoom / 1.2, c);
        }
        if sc(cmd, Key::W) {
            return true;
        }
        if self.active.is_none() {
            let none = Modifiers::NONE;
            if sc(none, Key::Delete) || sc(none, Key::Backspace) {
                self.delete_selection();
            }
            if sc(none, Key::Escape) {
                self.selection = None;
            }
            for (k, t) in [
                (Key::P, Tool::Pen),
                (Key::H, Tool::Highlighter),
                (Key::E, Tool::Eraser),
                (Key::L, Tool::Lasso),
                (Key::S, Tool::Shape),
                (Key::T, Tool::Text),
            ] {
                if sc(none, k) {
                    self.set_tool(t);
                }
            }
            if sc(none, Key::PageDown) {
                let c = self.current_page();
                self.scroll_to_page((c + 1).min(self.nb.pages.len() - 1));
            }
            if sc(none, Key::PageUp) {
                let c = self.current_page();
                self.scroll_to_page(c.saturating_sub(1));
            }
        }
        false
    }
}
