mod app;
mod editor;
mod export;
mod ink;
mod library;
mod model;
mod recognize;
mod render;
mod settings;
mod tablet;
mod text;
mod theme;

#[cfg(test)]
mod ui_tests;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("BetterNotes")
            .with_inner_size([1440.0, 920.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true)
            .with_icon(
                eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))
                    .expect("valid app icon"),
            ),
        ..Default::default()
    };
    eframe::run_native("BetterNotes", options, Box::new(|cc| Ok(Box::new(app::App::new(cc)))))
}
