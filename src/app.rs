//! Top-level app: switches between the library (home screen) and the editor.

use std::path::PathBuf;

use eframe::egui;

use crate::editor::{Editor, EditorAction};
use crate::library::{self, Library, LibraryAction};
use crate::settings::Settings;
use crate::{tablet, theme};

pub struct App {
    library: Library,
    editor: Option<Editor>,
    settings: Settings,
    title_shown: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        tablet::install();
        Self::with_dir(&cc.egui_ctx, library::default_dir())
    }

    pub fn with_dir(ctx: &egui::Context, dir: PathBuf) -> Self {
        theme::install(ctx);
        let library = Library::new(ctx, dir);
        let settings = Settings::load(&library.dir);
        App {
            library,
            editor: None,
            settings,
            title_shown: String::new(),
        }
    }

    pub fn open(&mut self, ctx: &egui::Context, path: PathBuf) {
        match Editor::open(path, self.library.dir.clone(), self.settings.clone()) {
            Ok(ed) => self.editor = Some(ed),
            Err(e) => {
                self.library.refresh(ctx);
                eprintln!("open failed: {e}");
            }
        }
    }

    fn close_editor(&mut self, ctx: &egui::Context) {
        if let Some(mut ed) = self.editor.take() {
            if let Err(e) = ed.save() {
                eprintln!("save failed: {e}");
            }
            self.settings = ed.settings.clone();
            self.settings.save(&self.library.dir);
        }
        self.library.refresh(ctx);
    }

    #[cfg(test)]
    pub fn editor_mut(&mut self) -> Option<&mut Editor> {
        self.editor.as_mut()
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if ctx.input(|i| i.viewport().close_requested()) {
            self.close_editor(&ctx);
            self.settings.save(&self.library.dir);
        }

        let title = match &self.editor {
            Some(ed) => format!("{} — BetterNotes", ed.nb.title),
            None => "BetterNotes".to_string(),
        };
        if title != self.title_shown {
            ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.title_shown = title;
        }

        if let Some(ed) = &mut self.editor {
            if let EditorAction::Close = ed.ui(ui) {
                self.close_editor(&ctx);
            }
        } else if let LibraryAction::Open(path) = self.library.ui(ui, &mut self.settings) {
            self.open(&ctx, path);
        }
    }
}
