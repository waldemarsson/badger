//! The reminder scheduler: a 30 s loop that fires due reminders, sends one native
//! notification per tick (single or batched, skipped while the window is focused), keeps
//! the dock badge at the number of fired reminders, and turns a notification click into
//! a `reminders-open` event. Errors are logged and the loop continues.

use std::time::Duration;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, Emitter, Manager};

use crate::commands::reminders::{inner_count_fired, inner_fire_due, local_now, Reminder};

/// Emitted (no payload) whenever reminders may have changed: fired, edited, or removed by a
/// cascade or a restore. The frontend re-lists on it.
pub const CHANGED_EVENT: &str = "reminders-changed";

/// Emitted with [`OpenPayload`] when a notification is clicked.
pub const OPEN_EVENT: &str = "reminders-open";

/// Label of the app's only window.
const MAIN_WINDOW: &str = "main";

/// How often the scheduler looks for due reminders.
const TICK: Duration = Duration::from_secs(30);

/// How many texts a batch notification lists before "…".
const BATCH_LISTED: usize = 2;

/// `reminders-open` payload: the clicked reminder, or `None` for a batch.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpenPayload {
    pub id: Option<i64>,
}

/// A native notification to send. `id` is the reminder for a single one, `None` for a batch.
#[derive(Debug, Clone, PartialEq)]
pub struct NotificationText {
    pub id: Option<i64>,
    pub title: String,
    pub body: String,
}

/// One notification for the rows fired in a tick: a single row gets its own ("Reminder",
/// its text, plus the linked todo or note); two or more are batched.
pub fn notification_for(fired: &[Reminder]) -> Option<NotificationText> {
    match fired {
        [] => None,
        [r] => {
            let mut body = r.text.clone();
            if let Some(title) = &r.todo_title {
                body.push_str(&format!("\nTodo: {title}"));
            } else if let Some(title) = &r.note_title {
                body.push_str(&format!("\nNote: {title}"));
            }
            Some(NotificationText {
                id: Some(r.id),
                title: "Reminder".to_string(),
                body,
            })
        }
        rows => {
            let mut listed: Vec<&str> = rows
                .iter()
                .take(BATCH_LISTED)
                .map(|r| r.text.as_str())
                .collect();
            if rows.len() > BATCH_LISTED {
                listed.push("…");
            }
            Some(NotificationText {
                id: None,
                title: "Reminders".to_string(),
                body: format!("{} reminders: {}", rows.len(), listed.join(", ")),
            })
        }
    }
}

/// What a tick does to the outside world; the app implementation is [`AppSink`].
pub trait ReminderSink {
    fn main_window_focused(&self) -> bool;
    fn notify(&self, notification: &NotificationText);
    fn changed(&self);
    fn set_badge(&self, fired: i64);
}

/// One scheduler step: fire due reminders, notify (unless the window is focused, where the
/// toasts cover it), announce the change, and recompute the badge. Never fails: errors are
/// logged and the next tick retries. Returns how many rows fired.
pub async fn tick(
    pool: &Pool<Sqlite>,
    now: chrono::NaiveDateTime,
    sink: &impl ReminderSink,
) -> usize {
    let fired = match inner_fire_due(pool, now).await {
        Ok(rows) => rows,
        Err(e) => {
            tracing::error!("Reminders: firing due reminders failed: {}", e);
            Vec::new()
        }
    };
    if !fired.is_empty() {
        if !sink.main_window_focused() {
            if let Some(n) = notification_for(&fired) {
                sink.notify(&n);
            }
        }
        sink.changed();
    }
    match inner_count_fired(pool).await {
        Ok(count) => sink.set_badge(count),
        Err(e) => tracing::error!("Reminders: couldn't count fired reminders: {}", e),
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
    }
}

/// The app's [`ReminderSink`]: native notifications, events and the dock badge.
pub struct AppSink {
    pub app: AppHandle,
}

impl ReminderSink for AppSink {
    fn main_window_focused(&self) -> bool {
        self.app
            .get_webview_window(MAIN_WINDOW)
            .and_then(|w| w.is_focused().ok())
            .unwrap_or(false)
    }

    fn notify(&self, n: &NotificationText) {
        use tauri_plugin_notification::NotificationExt;
        // The notification id carries the reminder id back on click; 0 means a batch
        // (reminder ids start at 1).
        let id = n.id.and_then(|id| i32::try_from(id).ok()).unwrap_or(0);
        let shown = self
            .app
            .notification()
            .builder()
            .id(id)
            .title(&n.title)
            .body(&n.body)
            .show();
        // At-most-once: the row stays fired even when the notification couldn't be sent.
        if let Err(e) = shown {
            tracing::error!("Reminders: notification not sent: {}", e);
        }
    }

    fn changed(&self) {
        emit_changed(&self.app);
    }

    fn set_badge(&self, fired: i64) {
        set_badge(&self.app, fired);
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
        open_from_notification(&handle, id);
    });
    if let Err(e) = installed {
        tracing::error!("Reminders: notification click handler not installed: {}", e);
    }
}

/// Shows, unminimizes and focuses the window, then asks the frontend to open `/reminders`.
fn open_from_notification(app: &AppHandle, id: Option<i64>) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        for (step, result) in [
            ("show", window.show()),
            ("unminimize", window.unminimize()),
            ("focus", window.set_focus()),
        ] {
            if let Err(e) = result {
                tracing::debug!("Reminders: window {} failed: {}", step, e);
            }
        }
    }
    if let Err(e) = app.emit(OPEN_EVENT, OpenPayload { id }) {
        tracing::debug!("Reminders: open event not sent: {}", e);
    }
}

/// Tells the frontend that reminders changed and recomputes the dock badge. Called by every
/// wrapper that changes reminders. Never fails: the change has committed, so problems are
/// logged.
pub async fn reminders_changed(app: &AppHandle, pool: &Pool<Sqlite>) {
    emit_changed(app);
    match inner_count_fired(pool).await {
        Ok(count) => set_badge(app, count),
        Err(e) => tracing::error!("Reminders: couldn't count fired reminders: {}", e),
    }
}

fn emit_changed(app: &AppHandle) {
    if let Err(e) = app.emit(CHANGED_EVENT, ()) {
        tracing::debug!("Reminders: change event not sent: {}", e);
    }
}

/// Dock badge = number of fired reminders (none at zero).
fn set_badge(app: &AppHandle, fired: i64) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW) else {
        return;
    };
    if let Err(e) = window.set_badge_count((fired > 0).then_some(fired)) {
        tracing::debug!("Reminders: badge not set: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::reminders::{
        inner_create_reminder, inner_snooze_reminder, parse_timestamp, ReminderInput,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeSink {
        focused: bool,
        notified: Mutex<Vec<NotificationText>>,
        changed: Mutex<usize>,
        badges: Mutex<Vec<i64>>,
    }

    impl ReminderSink for FakeSink {
        fn main_window_focused(&self) -> bool {
            self.focused
        }
        fn notify(&self, n: &NotificationText) {
            self.notified.lock().unwrap().push(n.clone());
        }
        fn changed(&self) {
            *self.changed.lock().unwrap() += 1;
        }
        fn set_badge(&self, fired: i64) {
            self.badges.lock().unwrap().push(fired);
        }
    }

    fn at(s: &str) -> chrono::NaiveDateTime {
        parse_timestamp(s).unwrap()
    }

    async fn add(text: &str, remind_at: &str, pool: &Pool<Sqlite>) -> Reminder {
        let input = ReminderInput {
            text: text.to_string(),
            remind_at: remind_at.to_string(),
            repeat: "none".to_string(),
            ..Default::default()
        };
        inner_create_reminder(&input, at("2026-11-08T12:00"), pool)
            .await
            .unwrap()
    }

    fn reminder(id: i64, text: &str) -> Reminder {
        Reminder {
            id,
            text: text.to_string(),
            remind_at: "2026-11-09T09:00:00.000".to_string(),
            time_of_day: "09:00".to_string(),
            repeat: "none".to_string(),
            weekdays: vec![],
            month_day: None,
            state: "fired".to_string(),
            todo_id: None,
            note_id: None,
            todo_title: None,
            todo_status: None,
            note_title: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn single_notification_names_the_reminder_and_its_link() {
        assert_eq!(notification_for(&[]), None);
        let plain = notification_for(&[reminder(3, "Send invoice")]).unwrap();
        assert_eq!(
            plain,
            NotificationText {
                id: Some(3),
                title: "Reminder".to_string(),
                body: "Send invoice".to_string(),
            }
        );
        let mut todo = reminder(4, "Call");
        todo.todo_title = Some("Call Anna".to_string());
        assert_eq!(
            notification_for(&[todo]).unwrap().body,
            "Call\nTodo: Call Anna"
        );
        let mut note = reminder(5, "Read");
        note.note_title = Some("Weekly sync".to_string());
        assert_eq!(
            notification_for(&[note]).unwrap().body,
            "Read\nNote: Weekly sync"
        );
    }

    #[test]
    fn two_or_more_are_batched_into_one_notification() {
        let two = notification_for(&[reminder(1, "A"), reminder(2, "B")]).unwrap();
        assert_eq!(two.id, None);
        assert_eq!(two.title, "Reminders");
        assert_eq!(two.body, "2 reminders: A, B");
        let three = notification_for(&[
            reminder(1, "Send invoice"),
            reminder(2, "Call Anna"),
            reminder(3, "Water plants"),
        ])
        .unwrap();
        assert_eq!(three.body, "3 reminders: Send invoice, Call Anna, …");
    }

    #[tokio::test]
    async fn tick_notifies_once_per_tick_and_updates_the_badge() {
        let pool = crate::test_db().await;
        add("one", "2026-11-09T09:00", &pool).await;
        add("two", "2026-11-09T09:00", &pool).await;
        add("later", "2026-11-09T18:00", &pool).await;
        let sink = FakeSink::default();

        assert_eq!(tick(&pool, at("2026-11-09T09:00:20"), &sink).await, 2);
        assert_eq!(tick(&pool, at("2026-11-09T09:00:50"), &sink).await, 0);
        assert_eq!(tick(&pool, at("2026-11-09T18:00:10"), &sink).await, 1);

        let notified = sink.notified.lock().unwrap().clone();
        assert_eq!(notified.len(), 2);
        assert_eq!(notified[0].body, "2 reminders: one, two");
        assert_eq!(notified[1].body, "later");
        assert!(notified[1].id.is_some());
        assert_eq!(*sink.changed.lock().unwrap(), 2);
        assert_eq!(*sink.badges.lock().unwrap(), vec![2, 2, 3]);
    }

    #[tokio::test]
    async fn tick_skips_the_native_notification_while_focused() {
        let pool = crate::test_db().await;
        add("x", "2026-11-09T09:00", &pool).await;
        let sink = FakeSink {
            focused: true,
            ..Default::default()
        };
        assert_eq!(tick(&pool, at("2026-11-09T09:00:20"), &sink).await, 1);
        assert!(sink.notified.lock().unwrap().is_empty());
        assert_eq!(*sink.changed.lock().unwrap(), 1);
        assert_eq!(*sink.badges.lock().unwrap(), vec![1]);
    }

    #[tokio::test]
    async fn tick_does_not_fire_a_row_snoozed_before_it() {
        let pool = crate::test_db().await;
        let r = add("x", "2026-11-09T09:00", &pool).await;
        inner_snooze_reminder(r.id, "2026-11-09T10:00", at("2026-11-09T08:59"), &pool)
            .await
            .unwrap();
        let sink = FakeSink::default();
        assert_eq!(tick(&pool, at("2026-11-09T09:00:20"), &sink).await, 0);
        assert!(sink.notified.lock().unwrap().is_empty());
        assert_eq!(*sink.changed.lock().unwrap(), 0);
    }

    #[tokio::test]
    async fn tick_survives_a_database_error_and_the_next_one_fires() {
        let broken = crate::test_db().await;
        broken.close().await;
        let sink = FakeSink::default();
        assert_eq!(tick(&broken, at("2026-11-09T09:00:20"), &sink).await, 0);
        assert!(sink.notified.lock().unwrap().is_empty());
        assert!(sink.badges.lock().unwrap().is_empty());

        // The loop goes on: a later tick against a working database fires normally.
        let pool = crate::test_db().await;
        add("x", "2026-11-09T09:00", &pool).await;
        assert_eq!(tick(&pool, at("2026-11-09T09:00:50"), &sink).await, 1);
        assert_eq!(sink.notified.lock().unwrap().len(), 1);
    }
}
