//! Visual design: fonts (Inter + Phosphor icons), colours, egui style and
//! small custom widgets shared by the library and the editor.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Pos2, Rect, Response,
    RichText, Sense, Shadow, Stroke, TextStyle, Vec2,
};
pub use egui_phosphor::regular as icon;

use crate::model::Rgba;
use crate::render::{NOTES_FONT, c32};
use crate::text;

pub const ACCENT: Color32 = Color32::from_rgb(52, 120, 246);
pub const ACCENT_SOFT: Color32 = Color32::from_rgb(230, 239, 254);
pub const BG: Color32 = Color32::from_rgb(245, 246, 248);
pub const CANVAS_BG: Color32 = Color32::from_rgb(234, 235, 239);
pub const SURFACE: Color32 = Color32::WHITE;
pub const BORDER: Color32 = Color32::from_rgb(226, 228, 233);
pub const TEXT: Color32 = Color32::from_rgb(29, 30, 34);
pub const MUTED: Color32 = Color32::from_rgb(118, 122, 133);
pub const HOVER: Color32 = Color32::from_rgb(237, 238, 242);
pub const DANGER: Color32 = Color32::from_rgb(224, 49, 49);

pub const MEDIUM: &str = "medium";
pub const ICONS: &str = "icons";
pub const SEMIBOLD: &str = "semibold";

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(ICONS.into()))
}

/// An icon glyph as rich text (must use the dedicated icon family: Inter has
/// its own glyphs in the same private-use codepoints).
pub fn ico(glyph: &str, size: f32) -> RichText {
    RichText::new(glyph).font(icon_font(size))
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    let defaults = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for (name, bytes) in [
        ("inter", text::FONT_REGULAR),
        ("inter-medium", text::FONT_MEDIUM),
        ("inter-semibold", text::FONT_SEMIBOLD),
    ] {
        fonts.font_data.insert(name.into(), FontData::from_static(bytes).into());
    }
    fonts.font_data.insert(
        "phosphor".into(),
        FontData::from_static(egui_phosphor::Variant::Regular.font_bytes()).into(),
    );
    let family = |main: &str| {
        let mut v = vec![main.to_string()];
        v.extend(defaults.iter().cloned());
        v
    };
    fonts.families.insert(FontFamily::Name(ICONS.into()), vec!["phosphor".to_string()]);
    fonts.families.insert(FontFamily::Proportional, family("inter"));
    fonts.families.insert(FontFamily::Name(MEDIUM.into()), family("inter-medium"));
    fonts.families.insert(FontFamily::Name(SEMIBOLD.into()), family("inter-semibold"));
    fonts.families.insert(FontFamily::Name(NOTES_FONT.into()), family("inter"));
    ctx.set_fonts(fonts);

    ctx.set_theme(egui::Theme::Light);
    ctx.options_mut(|o| {
        o.zoom_with_keyboard = false;
        o.theme_preference = egui::ThemePreference::Light;
    });
    // same (light) look regardless of the system theme
    ctx.all_styles_mut(|s| {
        s.visuals = egui::Visuals::light();
        s.text_styles = [
            (TextStyle::Heading, semibold(22.0)),
            (TextStyle::Body, FontId::proportional(14.0)),
            (TextStyle::Button, medium(13.5)),
            (TextStyle::Small, FontId::proportional(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into();
        s.spacing.item_spacing = Vec2::new(8.0, 8.0);
        s.spacing.button_padding = Vec2::new(10.0, 6.0);
        s.spacing.interact_size.y = 30.0;
        s.spacing.menu_margin = Margin::same(8);
        s.spacing.slider_width = 140.0;
        s.spacing.combo_width = 120.0;

        let v = &mut s.visuals;
        v.panel_fill = SURFACE;
        v.window_fill = SURFACE;
        v.extreme_bg_color = Color32::from_rgb(243, 244, 247);
        v.faint_bg_color = BG;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(12);
        v.window_shadow = Shadow {
            offset: [0, 10],
            blur: 30,
            spread: 0,
            color: Color32::from_black_alpha(36),
        };
        v.popup_shadow = Shadow {
            offset: [0, 6],
            blur: 20,
            spread: 0,
            color: Color32::from_black_alpha(30),
        };
        v.selection.bg_fill = ACCENT_SOFT;
        v.selection.stroke = Stroke::new(1.5, ACCENT);
        v.hyperlink_color = ACCENT;
        v.override_text_color = None;
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;

        let r = CornerRadius::same(8);
        let w = &mut v.widgets;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
        w.noninteractive.corner_radius = r;
        w.inactive.bg_fill = Color32::from_rgb(238, 239, 243);
        w.inactive.weak_bg_fill = Color32::from_rgb(238, 239, 243);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
        w.inactive.corner_radius = r;
        w.hovered.bg_fill = Color32::from_rgb(228, 230, 236);
        w.hovered.weak_bg_fill = Color32::from_rgb(228, 230, 236);
        w.hovered.bg_stroke = Stroke::NONE;
        w.hovered.fg_stroke = Stroke::new(1.5, TEXT);
        w.hovered.corner_radius = r;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = Color32::from_rgb(218, 221, 229);
        w.active.weak_bg_fill = Color32::from_rgb(218, 221, 229);
        w.active.bg_stroke = Stroke::NONE;
        w.active.fg_stroke = Stroke::new(1.5, TEXT);
        w.active.corner_radius = r;
        w.active.expansion = 0.0;
        w.open.bg_fill = ACCENT_SOFT;
        w.open.weak_bg_fill = ACCENT_SOFT;
        w.open.bg_stroke = Stroke::NONE;
        w.open.fg_stroke = Stroke::new(1.5, ACCENT);
        w.open.corner_radius = r;
    });
}

/// Floating "pill" container used for toolbars over the canvas.
pub fn pill_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(14))
        .shadow(Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(22),
        })
        .inner_margin(Margin::symmetric(10, 6))
}

pub fn card_shadow(rect: Rect, radius: u8, strong: bool) -> egui::Shape {
    Shadow {
        offset: [0, if strong { 10 } else { 4 }],
        blur: if strong { 26 } else { 14 },
        spread: 0,
        color: Color32::from_black_alpha(if strong { 40 } else { 20 }),
    }
    .as_shape(rect, CornerRadius::same(radius))
    .into()
}

/// Frameless icon button that gets a soft background on hover.
pub fn icon_button(ui: &mut egui::Ui, glyph: &str, tooltip: &str) -> Response {
    ui.add(
        egui::Button::new(ico(glyph, 18.0))
            .frame_when_inactive(false)
            .min_size(Vec2::new(34.0, 32.0)),
    )
    .on_hover_text(tooltip)
}

/// Toggle-style icon button (tools), custom painted for a crisp selected state.
pub fn tool_button(ui: &mut egui::Ui, glyph: &str, tooltip: &str, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(38.0, 34.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 9.0, SURFACE);
        p.add(card_shadow(rect, 9, false));
        p.rect_filled(rect, 9.0, ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 9.0, HOVER);
    }
    p.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        icon_font(19.0),
        if selected { ACCENT } else { TEXT },
    );
    resp.on_hover_text(tooltip)
}

fn labelled<'a>(glyph: Option<&'a str>, text: &'a str, color: Color32) -> egui::Atoms<'a> {
    let t = RichText::new(text).font(medium(14.0)).color(color);
    match glyph {
        Some(g) => egui::IntoAtoms::into_atoms((ico(g, 16.0).color(color), t)),
        None => egui::IntoAtoms::into_atoms(t),
    }
}

pub fn primary_button(ui: &mut egui::Ui, glyph: Option<&str>, text: &str) -> Response {
    ui.add(
        egui::Button::new(labelled(glyph, text, Color32::WHITE))
            .fill(ACCENT)
            .corner_radius(CornerRadius::same(9))
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

pub fn secondary_button(ui: &mut egui::Ui, glyph: Option<&str>, text: &str) -> Response {
    ui.add(
        egui::Button::new(labelled(glyph, text, TEXT))
            .fill(Color32::from_rgb(234, 236, 240))
            .corner_radius(CornerRadius::same(9))
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

pub fn danger_button(ui: &mut egui::Ui, text: &str) -> Response {
    ui.add(
        egui::Button::new(labelled(None, text, Color32::WHITE))
            .fill(DANGER)
            .corner_radius(CornerRadius::same(9))
            .min_size(Vec2::new(0.0, 36.0)),
    )
}

/// Menu entry with a leading icon.
pub fn menu_item(ui: &mut egui::Ui, glyph: &str, text: &str) -> Response {
    menu_item_colored(ui, glyph, text, TEXT)
}

pub fn menu_item_colored(ui: &mut egui::Ui, glyph: &str, text: &str, color: Color32) -> Response {
    ui.add(
        egui::Button::new((
            ico(glyph, 17.0).color(if color == TEXT { MUTED } else { color }),
            RichText::new(text).font(FontId::proportional(14.0)).color(color),
        ))
        .frame_when_inactive(false)
        .min_size(Vec2::new(210.0, 30.0)),
    )
}

/// Icon followed by a muted hint text.
pub fn hint(ui: &mut egui::Ui, glyph: &str, text: &str) {
    ui.label(ico(glyph, 17.0).color(MUTED));
    ui.label(RichText::new(text).color(MUTED));
}

pub fn color_swatch(ui: &mut egui::Ui, c: Rgba, selected: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    let p = ui.painter();
    let center = rect.center();
    let col = c32(c.with_alpha(255));
    if selected {
        p.circle_stroke(center, 11.5, Stroke::new(2.0, col));
        p.circle_filled(center, 8.0, col);
    } else {
        p.circle_filled(center, if resp.hovered() { 10.0 } else { 9.5 }, col);
    }
    if c.0[0] > 235 && c.0[1] > 235 && c.0[2] > 235 {
        p.circle_stroke(center, 9.5, Stroke::new(1.0, BORDER));
    }
    resp
}

pub fn width_dot(ui: &mut egui::Ui, radius: f32, selected: bool, color: Rgba) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8.0, ACCENT_SOFT);
    } else if resp.hovered() {
        p.rect_filled(rect, 8.0, HOVER);
    }
    p.circle_filled(rect.center(), radius.clamp(1.2, 10.0), c32(color.with_alpha(255)));
    resp
}

pub fn vsep(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(9.0, 24.0), Sense::hover());
    ui.painter().line_segment(
        [Pos2::new(rect.center().x, rect.top()), Pos2::new(rect.center().x, rect.bottom())],
        Stroke::new(1.0, BORDER),
    );
}

/// Segmented control: a row of mutually exclusive options.
pub fn segmented<T: PartialEq + Copy>(ui: &mut egui::Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(Color32::from_rgb(238, 239, 243))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, label) in options {
                    let sel = *value == *v;
                    let mut b = egui::Button::new(RichText::new(*label).color(if sel { TEXT } else { MUTED }))
                        .corner_radius(CornerRadius::same(7))
                        .min_size(Vec2::new(0.0, 28.0));
                    b = if sel { b.fill(SURFACE) } else { b.frame_when_inactive(false) };
                    if ui.add(b).clicked() && !sel {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

pub fn toast(ctx: &egui::Context, anchor: Pos2, msg: &str) {
    egui::Area::new(egui::Id::new("toast"))
        .order(egui::Order::Tooltip)
        .pivot(egui::Align2::CENTER_BOTTOM)
        .fixed_pos(anchor)
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgb(40, 42, 48))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::symmetric(14, 9))
                .shadow(Shadow {
                    offset: [0, 4],
                    blur: 14,
                    spread: 0,
                    color: Color32::from_black_alpha(40),
                })
                .show(ui, |ui| {
                    ui.label(RichText::new(msg).color(Color32::WHITE));
                });
        });
}

/// Lays out `add` left-to-right but pushed against the right edge, using the
/// width measured on the previous frame (egui has no two-pass layout).
pub fn right_aligned<R>(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let id = egui::Id::new(id);
    let w: f32 = ui.data(|d| d.get_temp(id)).unwrap_or(0.0);
    let space = (ui.available_width() - w).max(0.0);
    ui.add_space(space);
    let start = ui.cursor().min.x;
    let r = add(ui);
    let used = ui.min_rect().right() - start;
    if (used - w).abs() > 0.5 {
        ui.data_mut(|d| d.insert_temp(id, used));
        ui.ctx().request_repaint();
    }
    r
}
