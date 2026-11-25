//! The background sync task: one pass at a time, on a nudge (Badger writes, EventKit's
//! store-changed notification, restore, enabling) after a short debounce, or every 60 s.

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::{Mutex, Notify};

use super::engine::{run_pass, Outcome, PassMode, PassReport};
use super::{platform_store, PlatformStore, STATUS_EVENT};
use crate::recurrence::local_now;

/// How long a nudge waits for more before the pass starts.
pub const DEBOUNCE: Duration = Duration::from_millis(1500);
/// Fallback pass interval (iCloud changes don't always announce themselves).
pub const TICK: Duration = Duration::from_secs(60);
/// After a pass wrote to Reminders, EventKit's change notifications are its own echo.
const QUIET_AFTER_WRITES: Duration = Duration::from_secs(5);

/// Shared sync state, managed by Tauri. `lock` serializes sync passes and restore.
#[derive(Clone)]
pub struct SyncHandle {
    pub lock: Arc<Mutex<()>>,
    pub store: Arc<PlatformStore>,
    wake: Arc<Notify>,
    quiet_until: Arc<std::sync::Mutex<Option<Instant>>>,
}

impl SyncHandle {
    pub fn new() -> Self {
        let wake = Arc::new(Notify::new());
        let quiet_until: Arc<std::sync::Mutex<Option<Instant>>> = Arc::default();
        let on_change = {
            let (wake, quiet_until) = (wake.clone(), quiet_until.clone());
            Arc::new(move || {
                if !is_quiet(&quiet_until) {
                    wake.notify_one();
                }
            })
        };
        SyncHandle {
            lock: Arc::default(),
            store: Arc::new(platform_store(on_change)),
            wake,
            quiet_until,
        }
    }

    /// Asks for a pass soon. Bursts coalesce into one.
    pub fn nudge(&self) {
        self.wake.notify_one();
    }

    fn quiet_for(&self, d: Duration) {
        if let Ok(mut until) = self.quiet_until.lock() {
            *until = Some(Instant::now() + d);
        }
    }
}

impl Default for SyncHandle {
    fn default() -> Self {
        Self::new()
    }
}

fn is_quiet(quiet_until: &std::sync::Mutex<Option<Instant>>) -> bool {
    quiet_until
        .lock()
        .ok()
        .and_then(|until| *until)
        .is_some_and(|until| Instant::now() < until)
}

/// Nudges sync after a Badger write. A no-op before setup.
pub fn nudge(app: &AppHandle) {
    if let Some(handle) = app.try_state::<SyncHandle>() {
        handle.nudge();
    }
}

/// Nudges sync once `delay` has passed (a completion waits out its Undo window).
pub fn nudge_after(app: &AppHandle, delay: Duration) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(delay).await;
        nudge(&app);
    });
}

/// Runs for the app's lifetime. The first pass runs at once (catch-up after a restart).
pub async fn run(app: AppHandle, pool: Pool<Sqlite>, handle: SyncHandle) {
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            _ = handle.wake.notified() => tokio::time::sleep(DEBOUNCE).await,
        }
        pass_now(&app, &pool, &handle, PassMode::Normal).await;
    }
}

/// One pass under the lock, then tells the frontend what changed.
pub async fn pass_now(
    app: &AppHandle,
    pool: &Pool<Sqlite>,
    handle: &SyncHandle,
    mode: PassMode,
) -> PassReport {
    let report = {
        let _pass = handle.lock.lock().await;
        run_pass(pool, handle.store.as_ref(), local_now(), mode).await
    };
    if report.phone_writes > 0 {
        handle.quiet_for(QUIET_AFTER_WRITES);
    }
    if report.tasks_changed {
        // Sync changed tasks behind the UI's back; no nudge, or it would loop.
        crate::task_scheduler::tasks_refreshed(app, pool).await;
    }
    if report.outcome != Outcome::Disabled {
        emit(app, STATUS_EVENT);
    }
    report
}

pub fn emit(app: &AppHandle, event: &str) {
    if let Err(e) = app.emit(event, ()) {
        tracing::debug!("Apple Reminders sync: {} not sent: {}", event, e);
    }
}
