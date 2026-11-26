//! The floating command palette the global shortcut opens: a small frameless window over
//! whatever app is in front, so the main window is not brought up for a quick Log time or
//! New task. It is built hidden when a shortcut is registered and hidden (never closed) after use.
//!
//! On macOS it is a non-activating panel (`tauri-nspanel`), like Spotlight: it takes the
//! keyboard without activating Badger, so the menu bar stays on the app in front and focus
//! goes back there when it hides. Esc, a finished action, pressing the shortcut again or
//! clicking away hides it. Actions that need the main window (Go to, open a note) go through
//! [`palette_to_main`]: Rust hides the palette, shows the main window and re-emits the
//! action there as [`crate::tray::ACTION_EVENT`].
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow};

use crate::AppError;

pub const LABEL: &str = "palette";
/// Tells the palette window to reset and focus its input (each press after the first).
const OPEN_EVENT: &str = "palette://open";
const WIDTH: f64 = 640.0;
/// Initial height; the page resizes the window to its content.
const HEIGHT: f64 = 420.0;
/// Top edge as a fraction of the monitor height, like Spotlight.
const TOP_FRACTION: f64 = 0.2;

/// Global shortcut: open the palette, or hide it when it is showing.
pub fn toggle(app: &AppHandle) {
    on_main_thread(app, |app| match app.get_webview_window(LABEL) {
        Some(window) if window.is_visible().unwrap_or(false) => hide(app),
        Some(window) => {
            place(app, &window);
            show(app, &window);
            if let Err(e) = window.emit_to(LABEL, OPEN_EVENT, ()) {
                tracing::debug!("Palette: {} not sent: {}", OPEN_EVENT, e);
            }
        }
        // Not preloaded (yet): build it now. The page opens itself on its first load.
        None => match build(app) {
            Ok(window) => {
                place(app, &window);
                show(app, &window);
            }
            Err(e) => tracing::error!("Palette: window not created: {}", e),
        },
    });
}

/// Builds the hidden palette ahead of the first press (when a shortcut is registered), so a
/// press only shows it: creating a window can activate Badger.
pub fn preload(app: &AppHandle) {
    on_main_thread(app, |app| {
        if app.get_webview_window(LABEL).is_none() {
            if let Err(e) = build(app) {
                tracing::error!("Palette: window not created: {}", e);
            }
        }
    });
}

/// AppKit (the panel) must be driven from the main thread.
fn on_main_thread(app: &AppHandle, f: impl FnOnce(&AppHandle) + Send + 'static) {
    let handle = app.clone();
    if let Err(e) = app.run_on_main_thread(move || f(&handle)) {
        tracing::error!("Palette: not run on the main thread: {}", e);
    }
}

#[cfg(target_os = "macos")]
fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    mac::build(app)
}

#[cfg(not(target_os = "macos"))]
fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    use tauri::{WebviewUrl, WebviewWindowBuilder, WindowEvent};

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
    window.on_window_event({
        let window = window.clone();
        move |event| match event {
            // Clicking another app hides it, like Spotlight.
            WindowEvent::Focused(false) => step("hide", window.hide()),
            // Closing would recreate the webview on the next press.
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                step("hide", window.hide());
            }
            _ => {}
        }
    });
    Ok(window)
}

#[cfg(target_os = "macos")]
fn show(app: &AppHandle, _window: &WebviewWindow) {
    mac::show(app);
}

#[cfg(not(target_os = "macos"))]
fn show(_app: &AppHandle, window: &WebviewWindow) {
    step("show", window.show());
    step("focus", window.set_focus());
}

#[cfg(target_os = "macos")]
fn hide(app: &AppHandle) {
    mac::hide(app);
}

#[cfg(not(target_os = "macos"))]
fn hide(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(LABEL) {
        step("hide", window.hide());
    }
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

fn step(what: &str, result: tauri::Result<()>) {
    if let Err(e) = result {
        tracing::debug!("Palette: window {} failed: {}", what, e);
    }
}

/// The non-activating panel. `-> ()` in `panel_event!` and the generated items need the allows.
#[cfg(target_os = "macos")]
#[allow(dead_code, clippy::unused_unit)]
mod mac {
    use std::sync::Arc;

    use tauri::{AppHandle, LogicalSize, Manager, Size, WebviewUrl, WebviewWindow};
    use tauri_nspanel::{
        tauri_panel, CollectionBehavior, ManagerExt, PanelBuilder, PanelLevel, StyleMask,
    };

    use super::{HEIGHT, LABEL, WIDTH};

    tauri_panel! {
        panel!(PalettePanel {
            config: {
                can_become_key_window: true,
                is_floating_panel: true
            }
        })

        panel_event!(PalettePanelEvents {
            window_did_resign_key(notification: &NSNotification) -> (),
            window_should_close(window: &NSWindow) -> Bool
        })
    }

    pub fn build(app: &AppHandle) -> tauri::Result<WebviewWindow> {
        let panel = PanelBuilder::<_, PalettePanel>::new(app, LABEL)
            .url(WebviewUrl::App("palette".into()))
            .title("Badger")
            .size(Size::Logical(LogicalSize::new(WIDTH, HEIGHT)))
            .level(PanelLevel::Status) // above full-screen apps and the menu bar
            .floating(true)
            .has_shadow(false) // the card draws its own
            .transparent(true)
            .add_style_mask(StyleMask::empty().nonactivating_panel())
            .collection_behavior(
                CollectionBehavior::new()
                    .can_join_all_spaces()
                    .full_screen_auxiliary(),
            )
            .with_window(|w| {
                w.decorations(false)
                    .transparent(true)
                    .resizable(false)
                    .visible(false)
            })
            .build()?;

        // Replaces Tauri's window delegate, so these stand in for its focus and close events.
        let events = PalettePanelEvents::new();
        let weak = Arc::downgrade(&panel);
        events.window_did_resign_key(move |_| {
            // Clicking another app (or the main window taking over) hides it, like Spotlight.
            if let Some(panel) = weak.upgrade() {
                panel.hide();
            }
        });
        let weak = Arc::downgrade(&panel);
        events.window_should_close(move |_| {
            // ⌘W hides it; closing would recreate the webview on the next press.
            if let Some(panel) = weak.upgrade() {
                panel.hide();
            }
            Bool::NO
        });
        panel.set_event_handler(Some(events.as_ref()));

        // Not `panel.to_window()`: that turns the panel back into a plain window.
        app.get_webview_window(LABEL)
            .ok_or(tauri::Error::WindowNotFound)
    }

    pub fn show(app: &AppHandle) {
        match app.get_webview_panel(LABEL) {
            Ok(panel) => panel.show_and_make_key(),
            Err(e) => tracing::error!("Palette: no panel to show: {:?}", e),
        }
    }

    pub fn hide(app: &AppHandle) {
        if let Ok(panel) = app.get_webview_panel(LABEL) {
            panel.hide();
        }
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
    on_main_thread(&app, hide);
}

/// Hides the palette, shows the main window and asks it to run `action`.
#[tauri::command]
pub fn palette_to_main(
    action: String,
    href: Option<String>,
    app: AppHandle,
) -> Result<(), AppError> {
    validate_main_action(&action, href.as_deref())?;
    on_main_thread(&app, move |app| {
        hide(app);
        crate::tray::request_action(app, action, href);
    });
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
