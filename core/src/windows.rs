//! Window helpers for the desktop shell: the main window and the desktop buddy
//! (docs/ROADMAP.md → Desktop Buddy). Only compiled with the `tauri` feature.

use tauri::{AppHandle, Manager, Monitor, PhysicalPosition, Runtime, WebviewWindow};

use crate::settings::BuddySettings;

pub const MAIN: &str = "main";
pub const BUDDY: &str = "buddy";
/// Gap from the screen edge for the default position, in logical pixels.
const MARGIN: f64 = 24.0;

pub fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(MAIN) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Show (at its saved or default position) or hide the buddy. Never focuses it.
pub fn apply_buddy<R: Runtime>(app: &AppHandle<R>, buddy: &BuddySettings) {
    let Some(w) = app.get_webview_window(BUDDY) else {
        return;
    };
    if !buddy.enabled {
        let _ = w.hide();
        return;
    }
    if !w.is_visible().unwrap_or(false) {
        place(&w, buddy);
        let _ = w.show();
    }
}

fn on_some_monitor(monitors: &[Monitor], x: i32, y: i32) -> bool {
    monitors.iter().any(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x && y >= p.y && x < p.x + s.width as i32 && y < p.y + s.height as i32
    })
}

fn place<R: Runtime>(w: &WebviewWindow<R>, buddy: &BuddySettings) {
    let monitors = w.available_monitors().unwrap_or_default();
    if let (Some(x), Some(y)) = (buddy.x, buddy.y) {
        // Only reuse the saved spot if that display still exists.
        if on_some_monitor(&monitors, x + 20, y + 20) {
            let _ = w.set_position(PhysicalPosition::new(x, y));
            return;
        }
    }
    let Ok(Some(monitor)) = w.primary_monitor() else {
        return;
    };
    let Ok(size) = w.outer_size() else {
        return;
    };
    let area = monitor.work_area();
    let margin = (MARGIN * monitor.scale_factor()) as i32;
    let x = area.position.x + area.size.width as i32 - size.width as i32 - margin;
    let y = area.position.y + area.size.height as i32 - size.height as i32 - margin;
    let _ = w.set_position(PhysicalPosition::new(x, y));
}
