//! The task reminder scheduler: a 30 s loop that sends due task reminders, shows one native
//! notification per tick (single or batched, skipped while the window is focused and for
//! reminders the phone alarms for), keeps the dock badge at the overdue count (D10, which
//! moves with the clock, so it's recomputed every tick), and turns a notification click into
//! a `tasks-open` event. Errors are logged and the loop continues.

use std::time::Duration;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::tasks::{
    inner_count_overdue, inner_fire_due, DueRun, FiredReminder, UNDO_WINDOW_SECS,
};
use crate::recurrence::local_now;

/// Emitted (no payload) whenever tasks may have changed: edited, completed, synced, removed
/// by a restore, or a reminder sent. Pages that list tasks re-read on it.
pub const TASKS_CHANGED_EVENT: &str = "tasks-changed";
/// Emitted (no payload) together with [`TASKS_CHANGED_EVENT`]; the reminder toasts re-read
/// on it.
pub const REMINDERS_CHANGED_EVENT: &str = "reminders-changed";
/// Emitted with [`OpenPayload`] when a notification is clicked.
pub const OPEN_EVENT: &str = "tasks-open";

/// Label of the app's only window.
const MAIN_WINDOW: &str = "main";

/// How often the scheduler looks for due reminders.
const TICK: Duration = Duration::from_secs(30);

/// How many task titles a batch notification lists before "…".
const BATCH_LISTED: usize = 2;

/// `tasks-open` payload: the clicked notification's task, or `None` for a batch.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpenPayload {
    pub task_id: Option<i64>,
}

/// A native notification to send. `task_id` is the task for a single one, `None` for a batch.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationText {
    pub task_id: Option<i64>,
    pub title: String,
    pub body: String,
}

/// One notification for the reminders sent in a tick. Reminders of one task make a single
/// notification ("Reminder", the task title, plus its note); two or more tasks are batched.
pub fn notification_for(fired: &[FiredReminder]) -> Option<NotificationText> {
    let mut tasks: Vec<&FiredReminder> = Vec::new();
    for f in fired {
        if !tasks.iter().any(|t| t.task_id == f.task_id) {
            tasks.push(f);
        }
    }
    match tasks.as_slice() {
        [] => None,
        [f] => {
            let mut body = f.task_title.clone();
            if let Some(title) = &f.note_title {
                body.push_str(&format!("\nNote: {title}"));
            }
            Some(NotificationText {
                task_id: Some(f.task_id),
                title: "Reminder".to_string(),
                body,
            })
        }
        many => {
            let mut listed: Vec<&str> = many
                .iter()
                .take(BATCH_LISTED)
                .map(|f| f.task_title.as_str())
                .collect();
            if many.len() > BATCH_LISTED {
                listed.push("…");
            }
            Some(NotificationText {
                task_id: None,
                title: "Reminders".to_string(),
                body: format!("{} reminders: {}", many.len(), listed.join(", ")),
            })
        }
    }
}

/// What a tick does to the outside world; the app implementation is [`AppSink`].
pub trait TaskSink {
    fn main_window_focused(&self) -> bool;
    fn notify(&self, notification: &NotificationText);
    fn changed(&self);
    fn set_badge(&self, overdue: i64);
}

/// One scheduler step: send due reminders, notify (unless the window is focused, where the
/// toasts cover it, and leaving out reminders the phone alarms for through Apple Reminders
/// sync), announce the change (sent or reconciled reminders), and recompute the badge. Never fails: errors are logged and
/// the next tick retries. Returns how many reminders were sent.
pub async fn tick(pool: &Pool<Sqlite>, now: chrono::NaiveDateTime, sink: &impl TaskSink) -> usize {
    let DueRun { fired, reconciled } = match inner_fire_due(pool, now).await {
        Ok(run) => run,
        Err(e) => {
            tracing::error!("Tasks: sending due reminders failed: {}", e);
            DueRun::default()
        }
    };
    if !fired.is_empty() && !sink.main_window_focused() {
        let on_phone = crate::apple_sync::phone_alarms_cover(pool, &fired).await;
        let notify: Vec<FiredReminder> = fired
            .iter()
            .filter(|f| !on_phone.contains(&f.reminder_id))
            .cloned()
            .collect();
        if let Some(n) = notification_for(&notify) {
            sink.notify(&n);
        }
    }
    if !fired.is_empty() || !reconciled.is_empty() {
        sink.changed();
    }
    match inner_count_overdue(pool, now).await {
        Ok(count) => sink.set_badge(count),
        Err(e) => tracing::error!("Tasks: couldn't count overdue tasks: {}", e),
    }
    fired.len()
}

/// Runs the scheduler for the app's lifetime. The first tick runs at once (launch catch-up).
pub async fn run(app: AppHandle, pool: Pool<Sqlite>) {
    let sink = AppSink { app };
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        interval.tick().await;
        tick(&pool, local_now(), &sink).await;
        // Also rolls the menu bar over to a new day.
        crate::tray::nudge();
    }
}

/// The app's [`TaskSink`]: native notifications, events and the dock badge.
pub struct AppSink {
    pub app: AppHandle,
}

impl TaskSink for AppSink {
    fn main_window_focused(&self) -> bool {
        self.app
            .get_webview_window(MAIN_WINDOW)
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false)
    }

    fn notify(&self, n: &NotificationText) {
        use tauri_plugin_notification::NotificationExt;
        // The notification id carries the task id back on click; 0 means a batch (task ids
        // start at 1).
        let id = n.task_id.and_then(|id| i32::try_from(id).ok()).unwrap_or(0);
        let shown = self
            .app
            .notification()
            .builder()
            .id(id)
            .title(&n.title)
            .body(&n.body)
            .show();
        // At-most-once: the reminder stays sent even when the notification couldn't be shown.
        if let Err(e) = shown {
            tracing::error!("Tasks: notification not sent: {}", e);
        }
    }

    fn changed(&self) {
        emit_changed(&self.app);
    }

    fn set_badge(&self, overdue: i64) {
        set_badge(&self.app, overdue);
    }
}

/// Registers the notification click handler. Must run before the first notification is
/// shown: the plugin only reports clicks on notifications shown while a handler exists.
pub fn install_click_handler(app: &AppHandle) {
    use tauri_plugin_notification::NotificationExt;
    let handle = app.clone();
    let installed = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        let id = performed
            .notification()
            .map(|n| i64::from(n.id()))
            .filter(|id| *id > 0);
        open_tasks(&handle, id);
    });
    if let Err(e) = installed {
        tracing::error!("Tasks: notification click handler not installed: {}", e);
    }
}

/// Shows, unminimizes and focuses the window, then asks the frontend to open the task
/// (`/tasks?id=…`) or, for a batch, the tasks page. Notification clicks and the menu bar.
pub fn open_tasks(app: &AppHandle, task_id: Option<i64>) {
    crate::tray::show_main_window(app);
    if let Err(e) = app.emit(OPEN_EVENT, OpenPayload { task_id }) {
        tracing::debug!("Tasks: open event not sent: {}", e);
    }
}

/// Tells the frontend that tasks changed, recomputes the dock badge and nudges Apple
/// Reminders sync. Called by every wrapper that changes tasks or their reminders (and by
/// cascades: note delete, restore). Never fails: the change has committed, so problems are
/// logged.
pub async fn tasks_changed(app: &AppHandle, pool: &Pool<Sqlite>) {
    tasks_refreshed(app, pool).await;
    crate::apple_sync::runner::nudge(app);
}

/// [`tasks_changed`] for a completion: the sync nudge comes once the Undo window has closed
/// (sync also holds back completions younger than it), so an undone completion never
/// reaches the phone.
pub async fn tasks_changed_later(app: &AppHandle, pool: &Pool<Sqlite>) {
    tasks_refreshed(app, pool).await;
    let delay = Duration::from_secs(u64::try_from(UNDO_WINDOW_SECS).unwrap_or(5) + 1);
    crate::apple_sync::runner::nudge_after(app, delay);
}

/// [`tasks_changed`] without the sync nudge, for sync's own writes.
pub async fn tasks_refreshed(app: &AppHandle, pool: &Pool<Sqlite>) {
    emit_changed(app);
    crate::tray::nudge();
    match inner_count_overdue(pool, local_now()).await {
        Ok(count) => set_badge(app, count),
        Err(e) => tracing::error!("Tasks: couldn't count overdue tasks: {}", e),
    }
}

fn emit_changed(app: &AppHandle) {
    for event in [TASKS_CHANGED_EVENT, REMINDERS_CHANGED_EVENT] {
        if let Err(e) = app.emit(event, ()) {
            tracing::debug!("Tasks: {} not sent: {}", event, e);
        }
    }
}

/// Dock badge = overdue tasks (none at zero).
fn set_badge(app: &AppHandle, overdue: i64) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    if let Err(e) = window.set_badge_count((overdue > 0).then_some(overdue)) {
        tracing::debug!("Tasks: badge not set: {}", e);
    }
}

#[cfg(test)]
mod tests;
