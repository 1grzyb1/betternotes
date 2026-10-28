//! Pen-pressure input.
//!
//! winit (and therefore egui) doesn't forward graphics-tablet pressure on
//! macOS, so we install a local `NSEvent` monitor that observes mouse events
//! before winit handles them and records the pressure of tablet events.
//! The app pairs these readings with egui's pointer events, in order.
//!
//! On other platforms pen input arrives as egui `Touch` events with `force`,
//! which the app handles directly.

use std::sync::Mutex;

#[derive(Clone, Copy, Debug)]
pub struct TabletSample {
    pub pressure: f32,
    /// True when the event came from a tablet/stylus (not a mouse/trackpad).
    pub is_tablet: bool,
    pub kind: SampleKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SampleKind {
    Down,
    Drag,
    Up,
}

static SAMPLES: Mutex<Vec<TabletSample>> = Mutex::new(Vec::new());
static PASTE_REQUESTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Takes all samples recorded since the last call.
pub fn drain() -> Vec<TabletSample> {
    let mut g = SAMPLES.lock().unwrap();
    std::mem::take(&mut *g)
}

/// Cmd+V was pressed (egui only reports text pastes, we also want images).
pub fn take_paste_request() -> bool {
    PASTE_REQUESTED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

#[cfg(target_os = "macos")]
pub fn install() {
    use block2::RcBlock;
    use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSEventSubtype, NSEventType};
    use std::ptr::NonNull;

    let mask = NSEventMask::LeftMouseDown
        | NSEventMask::LeftMouseDragged
        | NSEventMask::LeftMouseUp
        | NSEventMask::KeyDown;

    let block = RcBlock::new(|event: NonNull<NSEvent>| -> *mut NSEvent {
        // SAFETY: AppKit hands us a valid event for the duration of the call.
        let ev = unsafe { event.as_ref() };
        let ty = ev.r#type();
        if ty == NSEventType::KeyDown {
            let cmd = ev.modifierFlags().contains(NSEventModifierFlags::Command);
            if cmd {
                if let Some(chars) = ev.charactersIgnoringModifiers() {
                    if chars.to_string().eq_ignore_ascii_case("v") {
                        PASTE_REQUESTED.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                }
            }
            return event.as_ptr();
        }
        let kind = if ty == NSEventType::LeftMouseDown {
            SampleKind::Down
        } else if ty == NSEventType::LeftMouseUp {
            SampleKind::Up
        } else {
            SampleKind::Drag
        };
        let is_tablet = ev.subtype() == NSEventSubtype::TabletPoint;
        let pressure = ev.pressure();
        if let Ok(mut g) = SAMPLES.lock() {
            // Bound the queue in case the app stops draining (e.g. minimised).
            if g.len() > 4096 {
                g.clear();
            }
            g.push(TabletSample {
                pressure,
                is_tablet,
                kind,
            });
        }
        event.as_ptr()
    });

    // SAFETY: the handler returns the event it was given, which is valid.
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) };
    // The monitor lives for the whole program.
    std::mem::forget(monitor);
    std::mem::forget(block);
}

#[cfg(not(target_os = "macos"))]
pub fn install() {}
