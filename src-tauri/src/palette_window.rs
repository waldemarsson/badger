//! The floating command palette the global shortcut opens: a small frameless window over
//! whatever app is in front, so the main window is not brought up for a quick Log time or
//! New task. It is created on the first press and hidden (never closed) afterwards.
//!
//! Esc, a finished action or clicking away hides it. When Badger was not the active app
//! before the press, dismissing also hides the app (macOS), which hands focus back to the
//! app the user was in. Actions that need the main window (Go to, open a note) go through
//! [`palette_to_main`]: Rust hides the palette, shows the main window and re-emits the
//! action there as [`crate::tray::ACTION_EVENT`].
use std::sync::atomic::{AtomicBool, Ordering};

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewUrl, WebviewWindow, WebviewWindowBuilder,
    WindowEvent,
};

use crate::AppError;

pub const LABEL: &str = "palette";
/// Tells the palette window to reset and focus its input (each press after the first).
const OPEN_EVENT: &str = "palette://open";
const WIDTH: f64 = 640.0;
/// Initial height; the page resizes the window to its content.
const HEIGHT: f64 = 420.0;
/// Top edge as a fraction of the monitor height, like Spotlight.
const TOP_FRACTION: f64 = 0.2;

/// Whether another app was in front when the palette opened (dismiss hands focus back).
static CAME_FROM_OTHER_APP: AtomicBool = AtomicBool::new(false);

/// Global shortcut: open the palette, or dismiss it when it already has focus.
pub fn toggle(app: &AppHandle) {
    let existing = app.get_webview_window(LABEL);
    if let Some(window) = &existing {
        if window.is_visible().unwrap_or(false) && window.is_focused().unwrap_or(false) {
            dismiss(app);
            return;
        }
    }
    let badger_active = app
        .webview_windows()
        .values()
        .any(|w| w.label() != LABEL && w.is_focused().unwrap_or(false));
    CAME_FROM_OTHER_APP.store(!badger_active, Ordering::SeqCst);
    match existing {
        Some(window) => {
            place(app, &window);
            step("show", window.show());
            step("focus", window.set_focus());
            if let Err(e) = window.emit_to(LABEL, OPEN_EVENT, ()) {
                tracing::debug!("Palette: {} not sent: {}", OPEN_EVENT, e);
            }
        }
        // The page opens itself on its first load.
        None => {
            if let Err(e) = create(app) {
                tracing::error!("Palette: window not created: {}", e);
            }
        }
    }
}

fn create(app: &AppHandle) -> tauri::Result<()> {
    let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("palette".into()))
        .title("Badger")
        .inner_size(WIDTH, HEIGHT)
        .decorations(false)
        .transparent(true)
        .shadow(false) // the card draws its own; a native one would outline the transparent area
        .resizable(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .visible_on_all_workspaces(true)
        .visible(false)
        .build()?;
    place(app, &window);
    window.on_window_event({
        let window = window.clone();
        move |event| match event {
            // Clicking another app hides it, like Spotlight. That app is already in front.
            WindowEvent::Focused(false) => step("hide", window.hide()),
            // ⌘W: hide; closing would recreate the webview on the next press.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                step("hide", window.hide());
            }
            _ => {}
        }
    });
    window.show()?;
    window.set_focus()
}

/// Centres the window horizontally on the monitor under the cursor, near the top.
fn place(app: &AppHandle, window: &WebviewWindow) {
    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(monitor) = monitor else { return };
    let (origin, size) = (monitor.position(), monitor.size());
    let width = (WIDTH * monitor.scale_factor()).round() as i32;
    let x = origin.x + (size.width as i32 - width) / 2;
    let y = origin.y + (f64::from(size.height) * TOP_FRACTION).round() as i32;
    step("position", window.set_position(PhysicalPosition::new(x, y)));
}

/// Hides the palette and, when another app was in front before, hands focus back to it.
fn dismiss(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        step("hide", window.hide());
    }
    #[cfg(target_os = "macos")]
    if CAME_FROM_OTHER_APP.swap(false, Ordering::SeqCst) {
        step("hide app", app.hide());
    }
}

fn step(what: &str, result: tauri::Result<()>) {
    if let Err(e) = result {
        tracing::debug!("Palette: window {} failed: {}", what, e);
    }
}

/// Actions the palette can hand to the main window (handled by the root layout).
const MAIN_ACTIONS: [&str; 3] = ["goto", "scratchpad", "handover"];

/// Checks a hand-off; `goto` needs an app path (`/notes?id=3`).
pub fn validate_main_action(action: &str, href: Option<&str>) -> Result<(), AppError> {
    if !MAIN_ACTIONS.contains(&action) {
        return Err(AppError::Custom(format!("Unknown action: {action}")));
    }
    if action == "goto" && !href.is_some_and(|h| h.starts_with('/') && !h.starts_with("//")) {
        return Err(AppError::Custom("Go to needs a page".to_string()));
    }
    Ok(())
}

/// Esc or a finished action in the palette window.
#[tauri::command]
pub fn dismiss_palette(app: AppHandle) {
    dismiss(&app);
}

/// Hides the palette, shows the main window and asks it to run `action`.
#[tauri::command]
pub fn palette_to_main(
    action: String,
    href: Option<String>,
    app: AppHandle,
) -> Result<(), AppError> {
    validate_main_action(&action, href.as_deref())?;
    CAME_FROM_OTHER_APP.store(false, Ordering::SeqCst); // Badger stays in front
    dismiss(&app);
    crate::tray::request_action(&app, action, href);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_main_actions() {
        assert!(validate_main_action("goto", Some("/notes?id=3")).is_ok());
        assert!(validate_main_action("scratchpad", None).is_ok());
        assert!(validate_main_action("handover", None).is_ok());
        assert!(validate_main_action("goto", None).is_err());
        assert!(validate_main_action("goto", Some("https://example.com")).is_err());
        assert!(validate_main_action("goto", Some("//example.com")).is_err());
        assert!(validate_main_action("quit", None).is_err());
    }
}
