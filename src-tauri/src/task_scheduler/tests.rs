use super::*;
use crate::commands::tasks::{
    inner_complete_task, inner_create_task, inner_snooze_task_reminder, Task, TaskInput,
    TaskReminderInput,
};
use crate::recurrence::parse_timestamp;
use std::sync::Mutex;

#[derive(Default)]
struct FakeSink {
    focused: bool,
    notified: Mutex<Vec<NotificationText>>,
    changed: Mutex<usize>,
    badges: Mutex<Vec<i64>>,
}

impl TaskSink for FakeSink {
    fn main_window_focused(&self) -> bool {
        self.focused
    }
    fn notify(&self, n: &NotificationText) {
        self.notified.lock().unwrap().push(n.clone());
    }
    fn changed(&self) {
        *self.changed.lock().unwrap() += 1;
    }
    fn set_badge(&self, overdue: i64) {
        self.badges.lock().unwrap().push(overdue);
    }
}

fn at(s: &str) -> chrono::NaiveDateTime {
    parse_timestamp(s).unwrap()
}

/// A task without a due date reminding at `when` (custom).
async fn add(title: &str, when: &str, pool: &Pool<Sqlite>) -> Task {
    let input = TaskInput {
        title: title.to_string(),
        ..Default::default()
    };
    let reminder = TaskReminderInput {
        offset_minutes: None,
        at: Some(when.to_string()),
    };
    inner_create_task(&input, &[reminder], at("2026-11-08T12:00"), pool)
        .await
        .unwrap()
}

fn fired(reminder_id: i64, task_id: i64, title: &str) -> FiredReminder {
    FiredReminder {
        reminder_id,
        task_id,
        task_title: title.to_string(),
        note_title: None,
        fire_at: at("2026-11-09T09:00"),
    }
}

#[test]
fn single_notification_names_the_task_and_its_note() {
    assert_eq!(notification_for(&[]), None);
    let plain = notification_for(&[fired(1, 3, "Send invoice")]).unwrap();
    assert_eq!(
        plain,
        NotificationText {
            task_id: Some(3),
            title: "Reminder".to_string(),
            body: "Send invoice".to_string(),
        }
    );
    let mut note = fired(2, 5, "Read");
    note.note_title = Some("Weekly sync".to_string());
    assert_eq!(
        notification_for(&[note]).unwrap().body,
        "Read\nNote: Weekly sync"
    );
    // Two reminders of one task are one notification about it.
    let same = notification_for(&[fired(1, 3, "Call"), fired(2, 3, "Call")]).unwrap();
    assert_eq!((same.task_id, same.body.as_str()), (Some(3), "Call"));
}

#[test]
fn two_or_more_tasks_are_batched_into_one_notification() {
    let two = notification_for(&[fired(1, 1, "A"), fired(2, 2, "B")]).unwrap();
    assert_eq!(two.task_id, None);
    assert_eq!(two.title, "Reminders");
    assert_eq!(two.body, "2 reminders: A, B");
    let three = notification_for(&[
        fired(1, 1, "Send invoice"),
        fired(2, 2, "Call Anna"),
        fired(3, 3, "Water plants"),
    ])
    .unwrap();
    assert_eq!(three.body, "3 reminders: Send invoice, Call Anna, …");
}

#[tokio::test]
async fn tick_sends_once_batches_and_keeps_the_badge_at_the_overdue_count() {
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
    assert!(notified[1].task_id.is_some());
    assert_eq!(*sink.changed.lock().unwrap(), 2);
    assert_eq!(*sink.badges.lock().unwrap(), vec![2, 2, 3]);
}

#[tokio::test]
async fn tick_announces_a_reconciled_reminder_even_when_nothing_is_sent() {
    let pool = crate::test_db().await;
    let t = add("moved", "2026-11-09T09:00", &pool).await;
    // A raw write moves the reminder's time without reconciling it.
    sqlx::query("UPDATE task_reminders SET At = '2026-11-09T07:00:00.000' WHERE TaskId = ?")
        .bind(t.id)
        .execute(&pool)
        .await
        .unwrap();
    let sink = FakeSink::default();
    assert_eq!(tick(&pool, at("2026-11-09T08:00"), &sink).await, 0);
    assert!(sink.notified.lock().unwrap().is_empty());
    assert_eq!(*sink.changed.lock().unwrap(), 1);
    // Reconciled once: the next tick has nothing to announce.
    tick(&pool, at("2026-11-09T08:00:30"), &sink).await;
    assert_eq!(*sink.changed.lock().unwrap(), 1);
}

#[tokio::test]
async fn launch_catch_up_sends_one_batched_notification() {
    let pool = crate::test_db().await;
    for i in 0..5 {
        add(&format!("t{i}"), &format!("2026-11-09T0{i}:00"), &pool).await;
    }
    let sink = FakeSink::default();
    assert_eq!(tick(&pool, at("2026-11-10T08:00"), &sink).await, 5);
    let notified = sink.notified.lock().unwrap().clone();
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].body, "5 reminders: t0, t1, …");
}

#[tokio::test]
async fn the_badge_counts_time_only_overdue_tasks_as_the_clock_moves() {
    let pool = crate::test_db().await;
    let input = TaskInput {
        title: "Due at ten".to_string(),
        due_date: Some("2026-11-09".to_string()),
        due_time: Some("10:00".to_string()),
        ..Default::default()
    };
    inner_create_task(&input, &[], at("2026-11-08T12:00"), &pool)
        .await
        .unwrap();
    let sink = FakeSink::default();
    tick(&pool, at("2026-11-09T09:59"), &sink).await;
    tick(&pool, at("2026-11-09T10:00:10"), &sink).await;
    assert_eq!(
        *sink.badges.lock().unwrap(),
        vec![0, 1],
        "no reminder needed"
    );
    assert!(
        sink.notified.lock().unwrap().is_empty(),
        "due never notifies"
    );
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
async fn tick_does_not_send_a_reminder_snoozed_before_it_or_of_a_completed_task() {
    let pool = crate::test_db().await;
    let t = add("snoozed", "2026-11-09T09:00", &pool).await;
    inner_snooze_task_reminder(
        t.reminders[0].id,
        "2026-11-09T10:00",
        at("2026-11-09T08:59"),
        &pool,
    )
    .await
    .unwrap();
    let done = add("done", "2026-11-09T09:00", &pool).await;
    inner_complete_task(done.id, at("2026-11-09T08:59"), &pool)
        .await
        .unwrap();
    let sink = FakeSink::default();
    assert_eq!(tick(&pool, at("2026-11-09T09:00:20"), &sink).await, 0);
    assert!(sink.notified.lock().unwrap().is_empty());
    assert_eq!(*sink.changed.lock().unwrap(), 0);
    assert_eq!(tick(&pool, at("2026-11-09T10:00:20"), &sink).await, 1);
}

/// Sync on and healthy, with `t` paired and `alarms` pushed to its phone item.
async fn pair(t: &Task, alarms: &[&str], pool: &Pool<Sqlite>) {
    crate::app_settings::set_apple_sync_enabled(pool, true)
        .await
        .unwrap();
    let alarms: Vec<String> = alarms
        .iter()
        .map(|a| crate::recurrence::format_timestamp(at(a)))
        .collect();
    sqlx::query(
        "INSERT INTO apple_sync_items (BadgerId, EkId, BadgerSnap, PhoneSnap, PhoneAlarms, SyncedAt) \
         VALUES (?, ?, '{}', '{}', ?, 'x')",
    )
    .bind(t.id)
    .bind(format!("ek-{}", t.id))
    .bind(serde_json::to_string(&alarms).unwrap())
    .execute(pool)
    .await
    .unwrap();
}

#[tokio::test]
async fn tick_leaves_reminders_the_phone_alarms_for_and_still_notifies_the_rest() {
    let pool = crate::test_db().await;
    let synced = add("on phone", "2026-11-09T09:00", &pool).await;
    add("unpaired", "2026-11-09T09:00", &pool).await;
    let moved = add("moved", "2026-11-09T09:00", &pool).await;
    pair(&synced, &["2026-11-09T09:00"], &pool).await;
    pair(&moved, &["2026-11-09T08:00"], &pool).await;
    let sink = FakeSink::default();

    assert_eq!(tick(&pool, at("2026-11-09T09:00:20"), &sink).await, 3);
    let notified = sink.notified.lock().unwrap().clone();
    assert_eq!(notified.len(), 1);
    assert_eq!(notified[0].body, "2 reminders: unpaired, moved");
    assert_eq!(*sink.changed.lock().unwrap(), 1, "toasts still update");
    assert_eq!(
        *sink.badges.lock().unwrap(),
        vec![3],
        "the badge counts all"
    );
}

#[tokio::test]
async fn tick_notifies_paired_reminders_when_sync_is_not_healthy_or_the_task_is_mac_only() {
    for case in ["paused", "errored", "off", "mac_only"] {
        let pool = crate::test_db().await;
        let t = add(case, "2026-11-09T09:00", &pool).await;
        pair(&t, &["2026-11-09T09:00"], &pool).await;
        match case {
            "paused" => crate::app_settings::set_apple_sync_outcome(
                &pool,
                None,
                Some("x"),
                Some("access"),
                None,
            )
            .await
            .unwrap(),
            "errored" => {
                crate::app_settings::set_apple_sync_outcome(&pool, None, Some("boom"), None, None)
                    .await
                    .unwrap()
            }
            "off" => crate::app_settings::set_apple_sync_enabled(&pool, false)
                .await
                .unwrap(),
            _ => {
                sqlx::query("UPDATE tasks SET MacOnly = 1")
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        }
        let sink = FakeSink::default();
        tick(&pool, at("2026-11-09T09:00:20"), &sink).await;
        assert_eq!(sink.notified.lock().unwrap().len(), 1, "{case}");
    }
}

#[tokio::test]
async fn tick_survives_a_database_error_and_the_next_one_fires() {
    let broken = crate::test_db().await;
    broken.close().await;
    let sink = FakeSink::default();
    assert_eq!(tick(&broken, at("2026-11-09T09:00:20"), &sink).await, 0);
    assert!(sink.notified.lock().unwrap().is_empty());
    assert!(sink.badges.lock().unwrap().is_empty());

    let pool = crate::test_db().await;
    add("x", "2026-11-09T09:00", &pool).await;
    assert_eq!(tick(&pool, at("2026-11-09T09:00:50"), &sink).await, 1);
    assert_eq!(sink.notified.lock().unwrap().len(), 1);
}
