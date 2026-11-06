//! Quit/close flush guard.
//!
//! On window close or app exit, Rust holds the request, emits [`FLUSH_REQUEST_EVENT`] so the
//! frontend can save unsaved notes, and waits for the `flush_done` command or
//! [`FLUSH_TIMEOUT`], whichever comes first. Then it closes the window or exits for real.
//! Once a flush has finished, further close/exit requests pass straight through, so the
//! re-entrant close/exit we trigger ourselves does not loop.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, RunEvent, Runtime, State, WindowEvent};
use tokio::sync::oneshot;

/// Event the frontend layout listens for.
pub const FLUSH_REQUEST_EVENT: &str = "app://flush-request";
/// Upper bound on how long a close or quit waits for the frontend.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(2);
/// Custom menu id for the macOS app-menu Quit item (see [`install_quit_menu`]).
pub const QUIT_MENU_ID: &str = "app-quit";

/// What to do with an incoming close or exit request.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// A flush already finished; let the request through.
    Allow,
    /// Hold the request. `start_flush` is false when a flush is already running.
    Prevent { start_flush: bool },
}

/// How a flush ended.
#[derive(Debug, PartialEq, Eq)]
pub enum FlushOutcome {
    /// The frontend called `flush_done`.
    Done,
    /// No reply within the timeout.
    TimedOut,
    /// The event could not be emitted; nothing to wait for.
    EmitFailed,
}

#[derive(Default)]
pub struct FlushGuard {
    flushed: AtomicBool,
    in_progress: AtomicBool,
    pending: Mutex<Option<oneshot::Sender<()>>>,
}

impl FlushGuard {
    /// Never panics on a poisoned lock: the slot holds no invariant worth protecting.
    fn slot(&self) -> MutexGuard<'_, Option<oneshot::Sender<()>>> {
        self.pending.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn decide(&self) -> Decision {
        if self.flushed.load(Ordering::SeqCst) {
            return Decision::Allow;
        }
        let start_flush = self
            .in_progress
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok();
        Decision::Prevent { start_flush }
    }

    /// Registers a waiter, calls `emit`, and waits for [`FlushGuard::done`] or `timeout`.
    /// Afterwards every request is allowed through.
    pub async fn flush(&self, emit: impl FnOnce() -> bool, timeout: Duration) -> FlushOutcome {
        let (tx, rx) = oneshot::channel();
        *self.slot() = Some(tx);
        let outcome = if !emit() {
            FlushOutcome::EmitFailed
        } else {
            match tokio::time::timeout(timeout, rx).await {
                Ok(Ok(())) => FlushOutcome::Done,
                _ => FlushOutcome::TimedOut,
            }
        };
        self.slot().take();
        self.flushed.store(true, Ordering::SeqCst);
        self.in_progress.store(false, Ordering::SeqCst);
        outcome
    }

    /// Wakes the pending flush. Returns false when nothing was waiting.
    pub fn done(&self) -> bool {
        match self.slot().take() {
            Some(tx) => tx.send(()).is_ok(),
            None => false,
        }
    }
}

/// Called by the frontend once it has flushed (or had nothing to flush).
#[tauri::command]
pub fn flush_done(guard: State<'_, FlushGuard>) {
    if !guard.done() {
        tracing::debug!("flush_done with no pending flush request");
    }
}

async fn flush_frontend<R: Runtime>(app: &AppHandle<R>) {
    let Some(guard) = app.try_state::<FlushGuard>() else {
        return;
    };
    let outcome = guard
        .flush(
            || match app.emit(FLUSH_REQUEST_EVENT, ()) {
                Ok(()) => true,
                Err(e) => {
                    tracing::error!("Flush guard: failed to emit flush request: {}", e);
                    false
                }
            },
            FLUSH_TIMEOUT,
        )
        .await;
    tracing::info!("Flush guard: {:?}", outcome);
}

/// Run-event hook: holds window close and app exit until the frontend has flushed.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: &RunEvent) {
    match event {
        RunEvent::ExitRequested { code, api, .. } => {
            let Some(guard) = app.try_state::<FlushGuard>() else {
                return;
            };
            if let Decision::Prevent { start_flush } = guard.decide() {
                api.prevent_exit();
                if start_flush {
                    let app = app.clone();
                    let code = code.unwrap_or(0);
                    tauri::async_runtime::spawn(async move {
                        flush_frontend(&app).await;
                        app.exit(code);
                    });
                }
            }
        }
        RunEvent::WindowEvent {
            label,
            event: WindowEvent::CloseRequested { api, .. },
            ..
        } => {
            let Some(guard) = app.try_state::<FlushGuard>() else {
                return;
            };
            if let Decision::Prevent { start_flush } = guard.decide() {
                api.prevent_close();
                if start_flush {
                    let app = app.clone();
                    let label = label.clone();
                    tauri::async_runtime::spawn(async move {
                        flush_frontend(&app).await;
                        match app.get_webview_window(&label) {
                            Some(window) => {
                                if let Err(e) = window.destroy() {
                                    tracing::error!("Flush guard: failed to close window: {}", e);
                                }
                            }
                            None => app.exit(0),
                        }
                    });
                }
            }
        }
        _ => {}
    }
}

/// Replaces the native app-menu Quit item with a custom one.
///
/// The native item sends `terminate:`, which tao turns into an unpreventable exit, so ⌘Q
/// would bypass `RunEvent::ExitRequested`. The custom item calls `AppHandle::exit`, which
/// goes through `ExitRequested` and therefore through the flush guard. Only meaningful
/// with the macOS default menu (app submenu first, Quit last); anything else is left as is.
pub fn install_quit_menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, MenuItemKind};

    let menu = Menu::default(app)?;
    let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() else {
        tracing::warn!("Flush guard: no app submenu; keeping the native Quit item");
        return Ok(());
    };
    let items = app_menu.items()?;
    let quit_index = items.len().checked_sub(1);
    let is_native_quit = matches!(
        items.last(),
        Some(MenuItemKind::Predefined(item))
            if item.text().is_ok_and(|t| t.starts_with("Quit"))
    );
    let (Some(index), true) = (quit_index, is_native_quit) else {
        tracing::warn!("Flush guard: app menu has no trailing Quit item; keeping the menu");
        return Ok(());
    };
    app_menu.remove_at(index)?;
    let quit = MenuItem::with_id(
        app,
        QUIT_MENU_ID,
        format!("Quit {}", app.package_info().name),
        true,
        Some("CmdOrCtrl+Q"),
    )?;
    app_menu.append(&quit)?;
    app.set_menu(menu)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn first_request_starts_flush_and_later_ones_wait() {
        let guard = FlushGuard::default();
        assert_eq!(guard.decide(), Decision::Prevent { start_flush: true });
        assert_eq!(guard.decide(), Decision::Prevent { start_flush: false });
    }

    #[tokio::test]
    async fn flush_completes_on_done_and_then_allows_reentry() {
        let guard = Arc::new(FlushGuard::default());
        assert_eq!(guard.decide(), Decision::Prevent { start_flush: true });

        let g = guard.clone();
        let outcome = guard
            .flush(
                move || {
                    // Frontend replies immediately (no handler registered).
                    tokio::spawn(async move { g.done() });
                    true
                },
                Duration::from_secs(5),
            )
            .await;
        assert_eq!(outcome, FlushOutcome::Done);
        // The close/exit we trigger next must pass straight through.
        assert_eq!(guard.decide(), Decision::Allow);
        assert_eq!(guard.decide(), Decision::Allow);
    }

    #[tokio::test]
    async fn flush_times_out_without_reply() {
        let guard = FlushGuard::default();
        let started = std::time::Instant::now();
        let outcome = guard.flush(|| true, Duration::from_millis(50)).await;
        assert_eq!(outcome, FlushOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(guard.decide(), Decision::Allow);
    }

    #[tokio::test]
    async fn emit_failure_skips_the_wait() {
        let guard = FlushGuard::default();
        let started = std::time::Instant::now();
        let outcome = guard.flush(|| false, Duration::from_secs(5)).await;
        assert_eq!(outcome, FlushOutcome::EmitFailed);
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(guard.decide(), Decision::Allow);
    }

    #[tokio::test]
    async fn stray_done_before_a_request_is_ignored() {
        let guard = FlushGuard::default();
        assert!(!guard.done(), "nothing pending");
        // A stale reply must not satisfy the next flush.
        let outcome = guard.flush(|| true, Duration::from_millis(50)).await;
        assert_eq!(outcome, FlushOutcome::TimedOut);
        assert!(!guard.done(), "waiter is cleared after the flush");
    }
}
