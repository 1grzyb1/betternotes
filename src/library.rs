//! Home screen: browse, create, rename, recolour, duplicate, import, export
//! and delete notebooks stored in the library folder.
//!
//! Each `<name>.bnote` file has a sidecar in `.thumbs/` with cached metadata
//! (`<name>.json`) and a first-page thumbnail (`<name>.png`), so the library
//! opens instantly without parsing every notebook.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{SystemTime, UNIX_EPOCH};

use eframe::egui::{
    self, Color32, CornerRadius, Margin, Pos2, Rect, RichText, Sense, Stroke, TextureHandle, Vec2,
};
use serde::{Deserialize, Serialize};

use crate::export;
use crate::model::*;
use crate::render::{self, PageView, TextureCache, c32};
use crate::settings::Settings;
use crate::theme::{self, icon};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Meta {
    pub title: String,
    pub cover: Rgba,
    pub pages: usize,
    pub page_w: f32,
    pub page_h: f32,
    /// File modification time (unix seconds) this metadata was made from.
    pub modified: u64,
}

struct Entry {
    path: PathBuf,
    meta: Option<Meta>,
    thumb: Option<TextureHandle>,
    thumb_tried: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Sort {
    Recent,
    Name,
}

enum Dialog {
    New {
        title: String,
        cover: Rgba,
        template: PageTemplate,
        size: PageSize,
        focus: bool,
    },
    Rename {
        path: PathBuf,
        title: String,
        focus: bool,
    },
    Delete {
        path: PathBuf,
        title: String,
    },
}

pub enum LibraryAction {
    None,
    Open(PathBuf),
}

pub struct Library {
    pub dir: PathBuf,
    entries: Vec<Entry>,
    tx: Sender<(PathBuf, Result<Meta, String>)>,
    rx: Receiver<(PathBuf, Result<Meta, String>)>,
    search: String,
    sort: Sort,
    dialog: Option<Dialog>,
    toast: Option<(String, f64)>,
    preview_textures: TextureCache,
}

pub fn default_dir() -> PathBuf {
    if let Ok(d) = std::env::var("BETTERNOTES_DIR") {
        return PathBuf::from(d);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join("Documents").join("BetterNotes")
}

fn thumbs_dir(dir: &Path) -> PathBuf {
    dir.join(".thumbs")
}

fn sidecar(dir: &Path, path: &Path, ext: &str) -> PathBuf {
    let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    thumbs_dir(dir).join(format!("{stem}.{ext}"))
}

fn mtime(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Writes the metadata + thumbnail sidecars for a notebook that was just saved.
pub fn write_meta(dir: &Path, path: &Path, nb: &Notebook) -> Result<Meta, String> {
    std::fs::create_dir_all(thumbs_dir(dir)).map_err(|e| e.to_string())?;
    let first = nb.pages.first();
    let meta = Meta {
        title: nb.title.clone(),
        cover: nb.cover,
        pages: nb.pages.len(),
        page_w: first.map(|p| p.width).unwrap_or(595.0),
        page_h: first.map(|p| p.height).unwrap_or(842.0),
        modified: mtime(path),
    };
    if let Some(page) = first {
        let pm = export::render_page_pixmap(page, 400.0 / page.width)?;
        pm.save_png(sidecar(dir, path, "png")).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_vec(&meta).map_err(|e| e.to_string())?;
    std::fs::write(sidecar(dir, path, "json"), json).map_err(|e| e.to_string())?;
    Ok(meta)
}

/// Saves a notebook into the library and refreshes its sidecars.
pub fn save_notebook(dir: &Path, path: &Path, nb: &Notebook) -> Result<(), String> {
    nb.save(path)?;
    write_meta(dir, path, nb).map(|_| ())
}

fn slug(title: &str) -> String {
    let map = |c: char| match c {
        'ą' => 'a',
        'ć' => 'c',
        'ę' => 'e',
        'ł' => 'l',
        'ń' => 'n',
        'ó' => 'o',
        'ś' => 's',
        'ź' | 'ż' => 'z',
        c if c.is_ascii_alphanumeric() => c,
        _ => '-',
    };
    let s: String = title.to_lowercase().chars().map(map).collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    if s.is_empty() { "notebook".into() } else { s.chars().take(40).collect() }
}

fn unique_path(dir: &Path, title: &str) -> PathBuf {
    let base = slug(title);
    let mut p = dir.join(format!("{base}.bnote"));
    let mut i = 2;
    while p.exists() {
        p = dir.join(format!("{base}-{i}.bnote"));
        i += 1;
    }
    p
}

/// Relative time, e.g. "5 min ago".
pub fn relative_time(secs: u64) -> String {
    let now = now_secs();
    let d = now.saturating_sub(secs);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        86_400..=172_799 => "yesterday".into(),
        172_800..=604_799 => format!("{} days ago", d / 86_400),
        _ => {
            // civil-from-days (Howard Hinnant)
            let z = (secs / 86_400) as i64 + 719_468;
            let era = z.div_euclid(146_097);
            let doe = z.rem_euclid(146_097);
            let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
            let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
            let mp = (5 * doy + 2) / 153;
            let day = doy - (153 * mp + 2) / 5 + 1;
            let month = if mp < 10 { mp + 3 } else { mp - 9 };
            let year = yoe + era * 400 + if month <= 2 { 1 } else { 0 };
            format!("{year}-{month:02}-{day:02}")
        }
    }
}

fn pages_label(n: usize) -> String {
    let word = if n == 1 { "page" } else { "pages" };
    format!("{n} {word}")
}

impl Library {
    pub fn new(ctx: &egui::Context, dir: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let (tx, rx) = channel();
        let mut lib = Library {
            dir,
            entries: Vec::new(),
            tx,
            rx,
            search: String::new(),
            sort: Sort::Recent,
            dialog: None,
            toast: None,
            preview_textures: TextureCache::default(),
        };
        lib.scan(ctx);
        lib
    }

    /// Re-reads the folder. Notebooks with stale or missing sidecars are
    /// parsed on a background thread.
    pub fn scan(&mut self, ctx: &egui::Context) {
        let mut entries = Vec::new();
        let mut stale = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path.extension().is_none_or(|x| x != "bnote") {
                    continue;
                }
                let meta: Option<Meta> = std::fs::read(sidecar(&self.dir, &path, "json"))
                    .ok()
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .filter(|m: &Meta| m.modified == mtime(&path));
                if meta.is_none() {
                    stale.push(path.clone());
                }
                // keep already-loaded thumbnails if nothing changed
                let old = self
                    .entries
                    .iter_mut()
                    .find(|o| o.path == path && o.meta.as_ref().map(|m| m.modified) == meta.as_ref().map(|m| m.modified));
                let thumb = old.and_then(|o| o.thumb.take());
                entries.push(Entry {
                    path,
                    meta,
                    thumb_tried: thumb.is_some(),
                    thumb,
                });
            }
        }
        self.entries = entries;
        if !stale.is_empty() {
            let (tx, dir, ctx) = (self.tx.clone(), self.dir.clone(), ctx.clone());
            std::thread::spawn(move || {
                for path in stale {
                    let r = Notebook::load(&path).and_then(|nb| write_meta(&dir, &path, &nb));
                    let _ = tx.send((path, r));
                    ctx.request_repaint();
                }
            });
        }
    }

    pub fn create(&mut self, ctx: &egui::Context, title: &str, cover: Rgba, size: PageSize, template: PageTemplate) -> Option<PathBuf> {
        let title = if title.trim().is_empty() { "Untitled" } else { title.trim() };
        let nb = Notebook::new(title, cover, size, template);
        let path = unique_path(&self.dir, title);
        match save_notebook(&self.dir, &path, &nb) {
            Ok(()) => {
                self.scan(ctx);
                Some(path)
            }
            Err(e) => {
                self.flash(ctx, format!("Could not create notebook: {e}"));
                None
            }
        }
    }

    fn flash(&mut self, ctx: &egui::Context, msg: impl Into<String>) {
        self.toast = Some((msg.into(), ctx.input(|i| i.time)));
    }

    fn modify(&mut self, ctx: &egui::Context, path: &Path, f: impl FnOnce(&mut Notebook)) {
        match Notebook::load(path) {
            Ok(mut nb) => {
                f(&mut nb);
                if let Err(e) = save_notebook(&self.dir, path, &nb) {
                    self.flash(ctx, format!("Save failed: {e}"));
                }
                self.scan(ctx);
            }
            Err(e) => self.flash(ctx, format!("Could not open: {e}")),
        }
    }

    fn duplicate(&mut self, ctx: &egui::Context, path: &Path) {
        match Notebook::load(path) {
            Ok(mut nb) => {
                nb.title = format!("{} (copy)", nb.title);
                let new = unique_path(&self.dir, &nb.title);
                if let Err(e) = save_notebook(&self.dir, &new, &nb) {
                    self.flash(ctx, format!("Error: {e}"));
                }
                self.scan(ctx);
            }
            Err(e) => self.flash(ctx, format!("Could not open: {e}")),
        }
    }

    fn delete(&mut self, ctx: &egui::Context, path: &Path) {
        // move to a trash folder instead of deleting outright
        let trash = self.dir.join(".trash");
        let _ = std::fs::create_dir_all(&trash);
        let name = path.file_name().map(|n| n.to_os_string()).unwrap_or_default();
        let mut target = trash.join(&name);
        if target.exists() {
            target = trash.join(format!("{}-{}", now_secs(), name.to_string_lossy()));
        }
        match std::fs::rename(path, &target) {
            Ok(()) => {
                let _ = std::fs::remove_file(sidecar(&self.dir, path, "json"));
                let _ = std::fs::remove_file(sidecar(&self.dir, path, "png"));
                self.flash(ctx, "Notebook moved to trash (.trash folder)");
            }
            Err(e) => self.flash(ctx, format!("Could not delete: {e}")),
        }
        self.scan(ctx);
    }

    pub fn import_files(&mut self, ctx: &egui::Context, files: &[PathBuf]) -> usize {
        let mut n = 0;
        for f in files {
            match Notebook::load(f) {
                Ok(nb) => {
                    let path = unique_path(&self.dir, &nb.title);
                    if save_notebook(&self.dir, &path, &nb).is_ok() {
                        n += 1;
                    }
                }
                Err(e) => self.flash(ctx, format!("{}: {e}", f.display())),
            }
        }
        if n > 0 {
            self.flash(ctx, format!("Imported notebooks: {n}"));
        }
        self.scan(ctx);
        n
    }

    fn export_pdf(&mut self, ctx: &egui::Context, path: &Path, title: &str) {
        let Some(out) = rfd::FileDialog::new()
            .add_filter("PDF", &["pdf"])
            .set_file_name(format!("{title}.pdf"))
            .save_file()
        else {
            return;
        };
        let r = Notebook::load(path).and_then(|nb| {
            let pages: Vec<&Page> = nb.pages.iter().collect();
            export::export_pdf(&pages, &out)
        });
        match r {
            Ok(()) => self.flash(ctx, format!("Exported: {}", out.display())),
            Err(e) => self.flash(ctx, format!("Export failed: {e}")),
        }
    }

    fn load_thumb(&mut self, ctx: &egui::Context, i: usize) {
        let e = &mut self.entries[i];
        if e.thumb_tried {
            return;
        }
        e.thumb_tried = true;
        let Some(img) = image::open(sidecar(&self.dir, &e.path, "png")).ok() else { return };
        let rgba = img.to_rgba8();
        let ci = egui::ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
        e.thumb = Some(ctx.load_texture(format!("thumb-{}", e.path.display()), ci, egui::TextureOptions::LINEAR));
    }

    // ------------------------------------------------------------------
    // UI
    // ------------------------------------------------------------------

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut Settings) -> LibraryAction {
        let ctx = ui.ctx().clone();
        let mut action = LibraryAction::None;

        while let Ok((path, r)) = self.rx.try_recv() {
            match r {
                Ok(meta) => {
                    if let Some(e) = self.entries.iter_mut().find(|e| e.path == path) {
                        e.meta = Some(meta);
                        e.thumb = None;
                        e.thumb_tried = false;
                    }
                }
                Err(err) => {
                    self.entries.retain(|e| e.path != path);
                    self.flash(&ctx, format!("Skipped damaged file {}: {err}", path.display()));
                }
            }
        }

        // dropped notebooks get imported
        let dropped: Vec<PathBuf> = ui.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| p.extension().is_some_and(|e| e == "bnote"))
                .collect()
        });
        if !dropped.is_empty() {
            self.import_files(&ctx, &dropped);
        }

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin::symmetric(36, 26)))
            .show(ui, |ui| {
                self.header(ui, settings);
                ui.add_space(22.0);
                if let Some(p) = self.grid(ui) {
                    action = LibraryAction::Open(p);
                }
            });

        if let Some(p) = self.dialogs(&ctx) {
            action = LibraryAction::Open(p);
        }
        if ctx.memory(|m| m.focused().is_none()) && ctx.input_mut(|i| i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N))) {
            self.open_new_dialog(settings);
        }

        if let Some((msg, t)) = &self.toast {
            let now = ctx.input(|i| i.time);
            if now - t < 4.0 {
                let r = ctx.content_rect();
                theme::toast(&ctx, Pos2::new(r.center().x, r.bottom() - 28.0), msg);
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            } else {
                self.toast = None;
            }
        }
        action
    }

    fn open_new_dialog(&mut self, settings: &Settings) {
        let used = self.entries.len();
        self.dialog = Some(Dialog::New {
            title: String::new(),
            cover: COVER_COLORS[used % COVER_COLORS.len()],
            template: settings.new_page_template,
            size: settings.new_page_size,
            focus: true,
        });
    }

    fn header(&mut self, ui: &mut egui::Ui, settings: &mut Settings) {
        let ctx = ui.ctx().clone();
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(Vec2::splat(40.0), Sense::hover());
            ui.painter().rect_filled(rect, 11.0, theme::ACCENT);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::NOTEBOOK,
                theme::icon_font(22.0),
                Color32::WHITE,
            );
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("BetterNotes").font(theme::semibold(22.0)).color(theme::TEXT));
                ui.label(RichText::new("Your notebooks").color(theme::MUTED));
            });

            theme::right_aligned(ui, "library-header-right", |ui| {
                egui::Frame::new()
                    .fill(theme::SURFACE)
                    .stroke(Stroke::new(1.0, theme::BORDER))
                    .corner_radius(CornerRadius::same(9))
                    .inner_margin(Margin::symmetric(10, 5))
                    .show(ui, |ui| {
                        ui.label(theme::ico(icon::MAGNIFYING_GLASS, 16.0).color(theme::MUTED));
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Search notebooks")
                                .frame(egui::Frame::NONE)
                                .desired_width(200.0),
                        );
                    });
                ui.add_space(4.0);
                theme::segmented(ui, &mut self.sort, &[(Sort::Recent, "Recent"), (Sort::Name, "Name")]);
                ui.add_space(8.0);
                if theme::secondary_button(ui, Some(icon::DOWNLOAD_SIMPLE), "Import")
                    .on_hover_text("Add .bnote files to the library (or drag them here)")
                    .clicked()
                {
                    if let Some(files) = rfd::FileDialog::new().add_filter("BetterNotes notebook", &["bnote"]).pick_files() {
                        self.import_files(&ctx, &files);
                    }
                }
                if theme::primary_button(ui, Some(icon::PLUS), "New notebook").clicked() {
                    self.open_new_dialog(settings);
                }
            });
        });
    }

    fn grid(&mut self, ui: &mut egui::Ui) -> Option<PathBuf> {
        let ctx = ui.ctx().clone();
        let q = self.search.to_lowercase();
        let mut order: Vec<usize> = (0..self.entries.len())
            .filter(|&i| {
                q.is_empty()
                    || self.entries[i]
                        .meta
                        .as_ref()
                        .is_some_and(|m| m.title.to_lowercase().contains(&q))
            })
            .collect();
        match self.sort {
            Sort::Recent => order.sort_by_key(|&i| std::cmp::Reverse(self.entries[i].meta.as_ref().map(|m| m.modified).unwrap_or(u64::MAX))),
            Sort::Name => order.sort_by_key(|&i| self.entries[i].meta.as_ref().map(|m| m.title.to_lowercase()).unwrap_or_default()),
        }

        ui.horizontal(|ui| {
            ui.label(RichText::new("All notebooks").font(theme::semibold(17.0)).color(theme::TEXT));
            ui.label(RichText::new(format!("{}", order.len())).color(theme::MUTED));
        });
        ui.add_space(10.0);

        if self.entries.is_empty() {
            ui.add_space(60.0);
            ui.vertical_centered(|ui| {
                ui.label(theme::ico(icon::BOOKS, 64.0).color(Color32::from_rgb(190, 195, 205)));
                ui.add_space(8.0);
                ui.label(RichText::new("No notebooks yet").font(theme::semibold(18.0)));
                ui.label(RichText::new("Create your first notebook or drop a .bnote file here").color(theme::MUTED));
                ui.add_space(12.0);
                if theme::primary_button(ui, Some(icon::PLUS), "New notebook").clicked() {
                    self.dialog = Some(Dialog::New {
                        title: String::new(),
                        cover: COVER_COLORS[0],
                        template: PageTemplate::Lined,
                        size: PageSize::A4,
                        focus: true,
                    });
                }
            });
            return None;
        }

        let mut open = None;
        let card_w = 172.0;
        let gap = 28.0;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            let avail = ui.available_width();
            let cols = (((avail + gap) / (card_w + gap)).floor() as usize).max(1);
            // index 0 of the grid is the "new notebook" tile
            let total = order.len() + 1;
            for row in 0..total.div_ceil(cols) {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = gap;
                    for col in 0..cols {
                        let k = row * cols + col;
                        if k >= total {
                            break;
                        }
                        if k == 0 {
                            if self.new_tile(ui, card_w) {
                                self.dialog = Some(Dialog::New {
                                    title: String::new(),
                                    cover: COVER_COLORS[self.entries.len() % COVER_COLORS.len()],
                                    template: PageTemplate::Lined,
                                    size: PageSize::A4,
                                    focus: true,
                                });
                            }
                        } else if let Some(p) = self.card(ui, &ctx, order[k - 1], card_w) {
                            open = Some(p);
                        }
                    }
                });
                ui.add_space(18.0);
            }
        });
        open
    }

    fn new_tile(&mut self, ui: &mut egui::Ui, w: f32) -> bool {
        let cover_h = w * 842.0 / 595.0;
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, cover_h + 46.0), Sense::click());
        let cover = Rect::from_min_size(rect.min, Vec2::new(w, cover_h));
        let p = ui.painter();
        let hovered = resp.hovered();
        p.rect_filled(cover, 12.0, if hovered { theme::ACCENT_SOFT } else { Color32::from_rgb(236, 238, 242) });
        let pts = [cover.left_top(), cover.right_top(), cover.right_bottom(), cover.left_bottom(), cover.left_top()];
        let col = if hovered { theme::ACCENT } else { Color32::from_rgb(190, 195, 205) };
        p.extend(egui::Shape::dashed_line(&pts.map(|q| q + (cover.center() - q) * 0.03), Stroke::new(1.5, col), 7.0, 5.0));
        p.circle_filled(cover.center(), 26.0, if hovered { theme::ACCENT } else { theme::SURFACE });
        p.text(
            cover.center(),
            egui::Align2::CENTER_CENTER,
            icon::PLUS,
            theme::icon_font(24.0),
            if hovered { Color32::WHITE } else { theme::ACCENT },
        );
        p.text(
            Pos2::new(rect.left() + 2.0, cover.bottom() + 12.0),
            egui::Align2::LEFT_TOP,
            "New notebook",
            theme::semibold(14.0),
            theme::TEXT,
        );
        if hovered {
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        resp.clicked()
    }

    fn card(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, i: usize, w: f32) -> Option<PathBuf> {
        let (ratio, title, sub, cover_col) = match &self.entries[i].meta {
            Some(m) => (
                m.page_h / m.page_w.max(1.0),
                m.title.clone(),
                format!("{} · {}", pages_label(m.pages), relative_time(m.modified)),
                c32(m.cover),
            ),
            None => (842.0 / 595.0, "Loading…".to_string(), String::new(), Color32::GRAY),
        };
        let cover_h = (w * ratio).min(w * 1.6);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, cover_h + 46.0), Sense::click());
        if !ui.is_rect_visible(rect) {
            return None;
        }
        self.load_thumb(ctx, i);
        let cover = Rect::from_min_size(rect.min, Vec2::new(w, cover_h));
        let hovered = resp.hovered() || resp.context_menu_opened();
        let lift = if hovered { -3.0 } else { 0.0 };
        let cover = cover.translate(Vec2::new(0.0, lift));
        let p = ui.painter();
        p.add(theme::card_shadow(cover, 12, hovered));
        p.rect_filled(cover, 12.0, theme::SURFACE);
        if let Some(tex) = &self.entries[i].thumb {
            egui::Image::new(tex).corner_radius(12).paint_at(ui, cover);
        }
        // notebook spine
        let spine = Rect::from_min_size(cover.min, Vec2::new(14.0, cover.height()));
        p.rect_filled(
            spine,
            CornerRadius { nw: 12, sw: 12, ne: 0, se: 0 },
            cover_col,
        );
        p.line_segment(
            [Pos2::new(spine.right(), spine.top()), Pos2::new(spine.right(), spine.bottom())],
            Stroke::new(1.0, Color32::from_black_alpha(30)),
        );
        p.rect_stroke(cover, 12.0, Stroke::new(1.0, Color32::from_black_alpha(18)), egui::StrokeKind::Inside);

        let galley = p.layout(title.clone(), theme::semibold(14.0), theme::TEXT, w - 4.0);
        let title_pos = Pos2::new(rect.left() + 2.0, cover.bottom() - lift + 10.0);
        let galley = if galley.rows.len() > 1 {
            // elide to one line
            let mut t = title.clone();
            while t.chars().count() > 1 && p.layout_no_wrap(format!("{t}…"), theme::semibold(14.0), theme::TEXT).size().x > w - 4.0 {
                t.pop();
            }
            p.layout_no_wrap(format!("{t}…"), theme::semibold(14.0), theme::TEXT)
        } else {
            galley
        };
        p.galley(title_pos, galley, theme::TEXT);
        p.text(
            title_pos + Vec2::new(0.0, 20.0),
            egui::Align2::LEFT_TOP,
            sub,
            egui::FontId::proportional(12.0),
            theme::MUTED,
        );

        let path = self.entries[i].path.clone();
        let mut open = None;

        // "more" button on hover
        let more_rect = Rect::from_center_size(cover.right_top() + Vec2::new(-20.0, 20.0), Vec2::splat(28.0));
        let more = ui.interact(more_rect, ui.id().with(("more", i)), Sense::click());
        let popup_open = egui::Popup::is_id_open(ctx, egui::Popup::default_response_id(&more));
        if hovered || more.hovered() || popup_open {
            let p = ui.painter();
            p.circle_filled(more_rect.center(), 14.0, Color32::from_white_alpha(235));
            p.circle_stroke(more_rect.center(), 14.0, Stroke::new(1.0, theme::BORDER));
            p.text(
                more_rect.center(),
                egui::Align2::CENTER_CENTER,
                icon::DOTS_THREE,
                theme::icon_font(18.0),
                theme::TEXT,
            );
        }
        let mut menu_action = None;
        egui::Popup::menu(&more).show(|ui| menu_action = self.card_menu(ui));
        resp.context_menu(|ui| {
            if let Some(a) = self.card_menu(ui) {
                menu_action = Some(a);
            }
        });

        if resp.hovered() || more.hovered() {
            ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if resp.clicked() && !more.clicked() && self.entries[i].meta.is_some() {
            open = Some(path.clone());
        }

        if let Some(a) = menu_action {
            let title = self.entries[i].meta.as_ref().map(|m| m.title.clone()).unwrap_or_default();
            match a {
                CardAction::Open => open = Some(path),
                CardAction::Rename => self.dialog = Some(Dialog::Rename { path, title, focus: true }),
                CardAction::Cover(c) => self.modify(ctx, &path, |nb| nb.cover = c),
                CardAction::Duplicate => self.duplicate(ctx, &path),
                CardAction::Export => self.export_pdf(ctx, &path, &title),
                CardAction::Reveal => reveal(&path),
                CardAction::Delete => self.dialog = Some(Dialog::Delete { path, title }),
            }
        }
        open
    }

    fn card_menu(&self, ui: &mut egui::Ui) -> Option<CardAction> {
        let mut a = None;
        if theme::menu_item(ui, icon::BOOK_OPEN, "Open").clicked() {
            a = Some(CardAction::Open);
        }
        if theme::menu_item(ui, icon::PENCIL_SIMPLE, "Rename").clicked() {
            a = Some(CardAction::Rename);
        }
        if theme::menu_item(ui, icon::COPY, "Duplicate").clicked() {
            a = Some(CardAction::Duplicate);
        }
        if theme::menu_item(ui, icon::FILE_PDF, "Export to PDF…").clicked() {
            a = Some(CardAction::Export);
        }
        if theme::menu_item(ui, icon::FOLDER_OPEN, REVEAL_LABEL).clicked() {
            a = Some(CardAction::Reveal);
        }
        ui.separator();
        ui.label(RichText::new("Cover color").small().color(theme::MUTED));
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(200.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            for c in COVER_COLORS {
                if theme::color_swatch(ui, c, false).clicked() {
                    a = Some(CardAction::Cover(c));
                }
            }
        });
        ui.separator();
        if theme::menu_item_colored(ui, icon::TRASH, "Delete", theme::DANGER).clicked()
        {
            a = Some(CardAction::Delete);
        }
        if a.is_some() {
            ui.close();
        }
        a
    }

    fn dialogs(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        let mut close = false;
        let mut result = None;
        let mut deferred: Option<Box<dyn FnOnce(&mut Library, &egui::Context) -> Option<PathBuf>>> = None;
        match &mut self.dialog {
            None => return None,
            Some(Dialog::New {
                title,
                cover,
                template,
                size,
                focus,
            }) => {
                let m = egui::Modal::new(egui::Id::new("new-notebook"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(460.0);
                        ui.label(RichText::new("New notebook").font(theme::semibold(20.0)));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Title").color(theme::MUTED));
                        let r = ui.add(
                            egui::TextEdit::singleline(title)
                                .hint_text("e.g. Calculus lectures")
                                .desired_width(f32::INFINITY)
                                .margin(Margin::symmetric(10, 8)),
                        );
                        if *focus {
                            r.request_focus();
                            *focus = false;
                        }
                        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(6.0);
                        ui.label(RichText::new("Cover color").color(theme::MUTED));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            for c in COVER_COLORS {
                                if theme::color_swatch(ui, c, *cover == c).clicked() {
                                    *cover = c;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(RichText::new("Paper").color(theme::MUTED));
                        ui.horizontal(|ui| {
                            for t in PageTemplate::ALL {
                                if template_tile(ui, &mut self.preview_textures, t, *template == t).clicked() {
                                    *template = t;
                                }
                            }
                        });
                        ui.add_space(6.0);
                        ui.label(RichText::new("Page size").color(theme::MUTED));
                        theme::segmented(
                            ui,
                            size,
                            &PageSize::ALL.map(|s| (s, s.label())),
                        );
                        ui.add_space(14.0);
                        let mut create = enter;
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if theme::primary_button(ui, None, "Create").clicked() {
                                    create = true;
                                }
                                if theme::secondary_button(ui, None, "Cancel").clicked() {
                                    close = true;
                                }
                            });
                        });
                        create
                    });
                if m.should_close() {
                    close = true;
                }
                if m.inner {
                    let (t, c, s, tp) = (title.clone(), *cover, *size, *template);
                    deferred = Some(Box::new(move |lib, ctx| lib.create(ctx, &t, c, s, tp)));
                    close = true;
                }
            }
            Some(Dialog::Rename { path, title, focus }) => {
                let m = egui::Modal::new(egui::Id::new("rename"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(380.0);
                        ui.label(RichText::new("Rename").font(theme::semibold(20.0)));
                        ui.add_space(6.0);
                        let r = ui.add(
                            egui::TextEdit::singleline(title)
                                .desired_width(f32::INFINITY)
                                .margin(Margin::symmetric(10, 8)),
                        );
                        if *focus {
                            r.request_focus();
                            *focus = false;
                        }
                        let mut ok = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        ui.add_space(12.0);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::primary_button(ui, None, "Save").clicked() {
                                ok = true;
                            }
                            if theme::secondary_button(ui, None, "Cancel").clicked() {
                                close = true;
                            }
                        });
                        ok
                    });
                if m.should_close() {
                    close = true;
                }
                if m.inner && !title.trim().is_empty() {
                    let (p, t) = (path.clone(), title.trim().to_string());
                    deferred = Some(Box::new(move |lib, ctx| {
                        lib.modify(ctx, &p, |nb| nb.title = t);
                        None
                    }));
                    close = true;
                }
            }
            Some(Dialog::Delete { path, title }) => {
                let m = egui::Modal::new(egui::Id::new("delete"))
                    .frame(modal_frame())
                    .show(ctx, |ui| {
                        ui.set_width(380.0);
                        ui.label(RichText::new("Delete notebook?").font(theme::semibold(20.0)));
                        ui.add_space(4.0);
                        ui.label(format!("“{title}” will be moved to the trash folder in your library."));
                        ui.add_space(12.0);
                        let mut ok = false;
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if theme::danger_button(ui, "Delete").clicked() {
                                ok = true;
                            }
                            if theme::secondary_button(ui, None, "Cancel").clicked() {
                                close = true;
                            }
                        });
                        ok
                    });
                if m.should_close() {
                    close = true;
                }
                if m.inner {
                    let p = path.clone();
                    deferred = Some(Box::new(move |lib, ctx| {
                        lib.delete(ctx, &p);
                        None
                    }));
                    close = true;
                }
            }
        }
        if close {
            self.dialog = None;
        }
        if let Some(f) = deferred {
            result = f(self, ctx);
        }
        result
    }

    /// Called after the editor saved a notebook, so its card is up to date.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        self.scan(ctx);
    }
}

enum CardAction {
    Open,
    Rename,
    Cover(Rgba),
    Duplicate,
    Export,
    Reveal,
    Delete,
}

fn modal_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(theme::SURFACE)
        .corner_radius(CornerRadius::same(16))
        .inner_margin(Margin::same(24))
        .shadow(egui::Shadow {
            offset: [0, 16],
            blur: 40,
            spread: 0,
            color: Color32::from_black_alpha(50),
        })
}

fn template_tile(ui: &mut egui::Ui, textures: &mut TextureCache, t: PageTemplate, selected: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(92.0, 124.0), Sense::click());
    let page_rect = Rect::from_min_size(rect.min + Vec2::new(8.0, 4.0), Vec2::new(76.0, 92.0));
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 10.0, theme::ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 10.0, theme::HOVER);
    }
    let mut page = Page::new(PageSize::A4, t);
    page.height = page.width * 92.0 / 76.0;
    let zoom = 76.0 / page.width;
    render::draw_page(&p.with_clip_rect(page_rect.expand(3.0)), &page, PageView { origin: page_rect.min, zoom }, textures, false, &[]);
    p.rect_stroke(
        page_rect,
        2.0,
        if selected { Stroke::new(2.0, theme::ACCENT) } else { Stroke::new(1.0, theme::BORDER) },
        egui::StrokeKind::Outside,
    );
    p.text(
        Pos2::new(rect.center().x, rect.bottom() - 14.0),
        egui::Align2::CENTER_CENTER,
        t.label(),
        theme::medium(12.5),
        if selected { theme::ACCENT } else { theme::TEXT },
    );
    resp
}

pub const REVEAL_LABEL: &str = if cfg!(target_os = "macos") {
    "Show in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in folder"
};

pub fn reveal(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("explorer").arg("/select,").arg(path).spawn();
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open").arg(path.parent().unwrap_or(path)).spawn();
}
