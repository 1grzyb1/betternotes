//! User preferences, persisted as JSON in the library folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ink::InkParams;
use crate::model::{PageSize, PageTemplate, Rgba};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub ink: InkParams,
    pub pen_color: Rgba,
    pub pen_width: f32,
    pub highlighter_color: Rgba,
    pub highlighter_width: f32,
    /// Draw-and-hold turns rough shapes into clean ones.
    pub shape_hold: bool,
    /// Eraser removes whole strokes instead of only the touched parts.
    pub eraser_whole: bool,
    pub eraser_radius: f32,
    pub text_size: f32,
    pub sidebar_open: bool,
    pub new_page_size: PageSize,
    pub new_page_template: PageTemplate,
    pub recent_colors: Vec<Rgba>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            ink: InkParams::default(),
            pen_color: Rgba::BLACK,
            pen_width: 1.8,
            highlighter_color: Rgba([255, 214, 10, 110]),
            highlighter_width: 14.0,
            shape_hold: true,
            eraser_whole: false,
            eraser_radius: 10.0,
            text_size: 14.0,
            sidebar_open: true,
            new_page_size: PageSize::A4,
            new_page_template: PageTemplate::Lined,
            recent_colors: Vec::new(),
        }
    }
}

impl Settings {
    fn path(dir: &Path) -> PathBuf {
        dir.join("settings.json")
    }

    pub fn load(dir: &Path) -> Self {
        std::fs::read(Self::path(dir))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(json) = serde_json::to_vec_pretty(self) {
            let _ = std::fs::write(Self::path(dir), json);
        }
    }

    pub fn remember_color(&mut self, c: Rgba) {
        let c = c.with_alpha(255);
        self.recent_colors.retain(|x| *x != c);
        self.recent_colors.insert(0, c);
        self.recent_colors.truncate(4);
    }
}
