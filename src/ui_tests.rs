//! Renders the real UI offscreen (wgpu) and drives it with synthetic input.
//! Screenshots go to `$BETTERNOTES_SHOTS` (or the temp dir) for inspection.

use std::path::PathBuf;

use eframe::egui::{self, Event, Modifiers, PointerButton, Pos2};
use egui_kittest::Harness;

use crate::app::App;
use crate::ink::InkPoint;
use crate::library;
use crate::model::*;

fn shots_dir() -> PathBuf {
    let d = std::env::var("BETTERNOTES_SHOTS")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("betternotes-shots"));
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn shot(h: &mut Harness<'_, App>, name: &str) {
    let img = h.render().expect("render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

/// Cursive-looking line of "handwriting" starting at (x, y).
fn handwriting(x: f32, y: f32, len: f32, seed: usize) -> Stroke {
    let n = (len * 1.6) as usize;
    let pts = (0..n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let a = t * len / 6.0 + seed as f32;
            let h = 9.0 + 4.0 * ((a * 0.37).sin() * (a * 0.13 + seed as f32).cos()).abs();
            InkPoint {
                x: x + t * len + a.cos() * 3.5,
                y: y - (a.sin() * 0.5 + 0.5) * h,
                p: 0.35 + 0.5 * (a * 0.7).sin().abs(),
                t: i as f32 * 8.0,
            }
        })
        .collect();
    Stroke::new(InkKind::Pen, Rgba([25, 55, 140, 255]), 1.6, true, Default::default(), pts)
}

fn sample_notebook(title: &str, cover: Rgba, template: PageTemplate, pages: usize) -> Notebook {
    let mut nb = Notebook::new(title, cover, PageSize::A4, template);
    for p in 1..pages {
        let _ = p;
        nb.pages.push(Page::new(PageSize::A4, template));
    }
    let page = &mut nb.pages[0];
    page.elements.push(Element::Text(TextElem::new(
        P::new(90.0, 52.0),
        title.to_string(),
        24.0,
        Rgba::BLACK,
        450.0,
    )));
    for row in 0..8 {
        let y = 136.0 + row as f32 * 24.0;
        let mut x = 90.0;
        for w in 0..4 {
            let len = 40.0 + ((row * 7 + w * 13) % 5) as f32 * 14.0;
            page.elements.push(Element::Stroke(handwriting(x, y, len, row * 4 + w)));
            x += len + 22.0;
        }
    }
    page.elements.push(Element::Stroke(Stroke::new(
        InkKind::Highlighter,
        Rgba([255, 214, 10, 110]),
        14.0,
        false,
        Default::default(),
        (0..40).map(|i| InkPoint { x: 88.0 + i as f32 * 6.0, y: 156.0, p: 1.0, t: i as f32 * 8.0 }).collect(),
    )));
    page.elements.push(Element::Shape(Shape::new(
        ShapeKind::Ellipse,
        P::new(360.0, 360.0),
        P::new(500.0, 440.0),
        Rgba([220, 40, 50, 255]),
        1.5,
        Some(Rgba([220, 40, 50, 50])),
    )));
    page.elements.push(Element::Shape(Shape::new(
        ShapeKind::Arrow,
        P::new(120.0, 400.0),
        P::new(340.0, 400.0),
        Rgba::BLACK,
        1.5,
        None,
    )));
    nb
}

fn setup_library(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("betternotes-ui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let books = [
        ("Calculus – lectures", COVER_COLORS[0], PageTemplate::Grid, 12),
        ("Physics", COVER_COLORS[3], PageTemplate::Lined, 4),
        ("Ideas & sketches", COVER_COLORS[7], PageTemplate::Dotted, 7),
        ("Journal", COVER_COLORS[5], PageTemplate::Lined, 1),
        ("Thesis project – meeting notes", COVER_COLORS[2], PageTemplate::Blank, 23),
    ];
    for (title, cover, tpl, pages) in books {
        let nb = sample_notebook(title, cover, tpl, pages);
        let path = dir.join(format!("{}.bnote", title.len()));
        library::save_notebook(&dir, &path, &nb).unwrap();
    }
    dir
}

fn harness(dir: PathBuf) -> Harness<'static, App> {
    Harness::builder()
        .with_size(egui::vec2(1440.0, 900.0))
        .with_pixels_per_point(1.0)
        .wgpu()
        .build_eframe(move |cc| App::with_dir(&cc.egui_ctx, dir))
}

fn press(h: &Harness<'_, App>, pos: Pos2, pressed: bool) {
    h.event(Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    });
}

#[test]
fn ui_screenshots() {
    let dir = setup_library("shots");
    let mut h = harness(dir.clone());
    h.run_steps(6);
    shot(&mut h, "01-library");

    // "new notebook" tile opens the creation dialog
    let tile = Pos2::new(120.0, 270.0);
    h.event(Event::PointerMoved(tile));
    press(&h, tile, true);
    h.step();
    press(&h, tile, false);
    h.run_steps(4);
    shot(&mut h, "01b-new-dialog");
    h.key_press(egui::Key::Escape);
    h.run_steps(3);

    // card "more" menu
    let card = Pos2::new(320.0, 260.0);
    h.event(Event::PointerMoved(card));
    h.run_steps(2);
    let more = Pos2::new(387.0, 166.0);
    h.event(Event::PointerMoved(more));
    press(&h, more, true);
    h.step();
    press(&h, more, false);
    h.run_steps(4);
    shot(&mut h, "01c-card-menu");
    h.key_press(egui::Key::Escape);
    h.event(Event::PointerMoved(Pos2::new(900.0, 700.0)));
    h.run_steps(3);

    // open the first notebook in the editor
    let path = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "bnote"))
        .min()
        .unwrap();
    let ctx = h.ctx.clone();
    h.state_mut().open(&ctx, path);
    h.run_steps(6);
    shot(&mut h, "02-editor");

    // draw a stroke with the pen through real input events
    let start = Pos2::new(560.0, 560.0);
    h.event(Event::PointerMoved(start));
    press(&h, start, true);
    h.step();
    for i in 1..60 {
        let t = i as f32 / 60.0;
        let p = Pos2::new(560.0 + t * 260.0, 560.0 - (t * 18.0).sin() * 22.0);
        h.event(Event::PointerMoved(p));
        if i % 3 == 0 {
            h.step();
        }
    }
    press(&h, Pos2::new(820.0, 560.0), false);
    h.run_steps(3);

    // text tool: click and type
    h.key_press(egui::Key::T);
    h.run_steps(2);
    let tp = Pos2::new(560.0, 640.0);
    h.event(Event::PointerMoved(tp));
    press(&h, tp, true);
    h.step();
    press(&h, tp, false);
    h.run_steps(3);
    h.event(Event::Text("Typed from the keyboard – café".into()));
    h.run_steps(3);
    shot(&mut h, "03-editor-text");

    // commit, switch to shapes
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    h.key_press(egui::Key::S);
    h.run_steps(3);
    shot(&mut h, "04-editor-shapes");

    // pen: draw a rough circle and hold still -> recognised ellipse
    h.key_press(egui::Key::P);
    h.run_steps(2);
    let c = Pos2::new(1000.0, 330.0);
    let at = |i: usize| {
        let a = i as f32 / 48.0 * std::f32::consts::TAU;
        let wobble = ((i * 7919) % 7) as f32 - 3.0;
        c + egui::vec2((60.0 + wobble) * a.cos(), (45.0 + wobble * 0.5) * a.sin())
    };
    h.event(Event::PointerMoved(at(0)));
    press(&h, at(0), true);
    h.step();
    for i in 1..=48 {
        h.event(Event::PointerMoved(at(i)));
        h.step();
    }
    for _ in 0..50 {
        h.step(); // hold
    }
    shot(&mut h, "05-shape-hold");
    press(&h, at(48), false);
    h.run_steps(3);
    {
        let ed = h.state_mut().editor_mut().unwrap();
        assert!(
            matches!(ed.nb.pages[0].elements.last(), Some(Element::Shape(s)) if s.kind == ShapeKind::Ellipse),
            "draw-and-hold should produce an ellipse"
        );
    }
    shot(&mut h, "06-editor-pen");
    let sliders = Pos2::new(1046.0, 91.0);
    h.event(Event::PointerMoved(sliders));
    press(&h, sliders, true);
    h.step();
    press(&h, sliders, false);
    h.run_steps(4);
    shot(&mut h, "07-pen-settings");

    let ed = h.state_mut().editor_mut().unwrap();
    let n = ed.nb.pages[0].elements.len();
    assert_eq!(n, 39, "stroke, text and recognised shape were added");
    assert!(ed.nb.pages[0].elements.iter().any(|e| matches!(e, Element::Text(t) if t.text.contains("café"))));

    let _ = std::fs::remove_dir_all(&dir);
}
