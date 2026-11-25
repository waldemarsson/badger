use super::*;
use crate::commands::customers::inner_create_customer;
use crate::commands::issues::inner_resolve_issue;
use crate::commands::projects::inner_create_project;

// 2026-11-19 is a Thursday.
const NOW: &str = "2026-11-19T12:00";

fn at(s: &str) -> NaiveDateTime {
    parse_timestamp(s).unwrap()
}

fn now() -> NaiveDateTime {
    at(NOW)
}

fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
    r.unwrap_err().to_string()
}

fn input(title: &str) -> TaskInput {
    TaskInput {
        title: title.to_string(),
        ..Default::default()
    }
}

fn due(title: &str, date: &str, time: Option<&str>) -> TaskInput {
    TaskInput {
        due_date: Some(date.to_string()),
        due_time: time.map(str::to_string),
        ..input(title)
    }
}

fn offset(minutes: i64) -> TaskReminderInput {
    TaskReminderInput {
        offset_minutes: Some(minutes),
        at: None,
    }
}

fn custom_at(s: &str) -> TaskReminderInput {
    TaskReminderInput {
        offset_minutes: None,
        at: Some(s.to_string()),
    }
}

async fn create(i: TaskInput, reminders: &[TaskReminderInput], pool: &Pool<Sqlite>) -> Task {
    inner_create_task(&i, reminders, now(), pool).await.unwrap()
}

/// (offset, at, state, fire_at) per reminder, in the task's order.
type ReminderView = (Option<i64>, Option<String>, String, Option<String>);

fn view(t: &Task) -> Vec<ReminderView> {
    t.reminders
        .iter()
        .map(|r| {
            (
                r.offset_minutes,
                r.at.clone(),
                r.state.clone(),
                r.fire_at.clone(),
            )
        })
        .collect()
}

fn ts(s: &str) -> Option<String> {
    Some(format_timestamp(at(s)))
}

async fn fire(now: &str, pool: &Pool<Sqlite>) -> Vec<FiredReminder> {
    inner_fire_due(pool, at(now)).await.unwrap().fired
}

async fn overdue(now: &str, pool: &Pool<Sqlite>) -> i64 {
    inner_count_overdue(pool, at(now)).await.unwrap()
}

async fn edit(t: &Task, f: impl FnOnce(&mut TaskInput), when: &str, pool: &Pool<Sqlite>) -> Task {
    let mut i = TaskInput {
        title: t.title.clone(),
        notes: t.notes.clone(),
        due_date: t.due_date.clone(),
        due_time: t.due_time.clone(),
        repeat: t.repeat.clone(),
        weekdays: t.weekdays.iter().map(|d| i64::from(*d)).collect(),
        month_day: t.month_day,
        issue_id: t.issue_id,
        note_id: t.note_id,
        mac_only: None,
    };
    f(&mut i);
    inner_update_task(t.id, &i, at(when), pool).await.unwrap()
}

// ── AC2: CRUD and validation ──────────────────────────────────────────────────

#[tokio::test]
async fn create_trims_and_nulls_blank_fields() {
    let pool = crate::test_db().await;
    let t = create(
        TaskInput {
            title: "  Buy milk 🥛 ü  ".into(),
            notes: Some("   ".into()),
            due_date: Some("".into()),
            due_time: Some(" ".into()),
            ..Default::default()
        },
        &[],
        &pool,
    )
    .await;
    assert_eq!(t.title, "Buy milk 🥛 ü");
    assert_eq!((t.notes, t.due_date, t.due_time), (None, None, None));
    assert!(!t.done && !t.mac_only);
    assert_eq!(t.repeat, "none");
    assert!(t.reminders.is_empty());
    assert_eq!(
        inner_get_task(t.id, &pool).await.unwrap().title,
        "Buy milk 🥛 ü"
    );
}

#[tokio::test]
async fn titles_are_one_to_500_characters() {
    let pool = crate::test_db().await;
    let long = "é".repeat(500);
    let t = create(input(&format!("  {long} ")), &[], &pool).await;
    assert_eq!(t.title.chars().count(), 500);
    let too_long = inner_create_task(&input(&"é".repeat(501)), &[], now(), &pool).await;
    assert_eq!(err_msg(too_long), "Title is too long (max 500 characters)");
    assert_eq!(
        err_msg(inner_create_task(&input("  "), &[], now(), &pool).await),
        "Title is required"
    );
}

#[tokio::test]
async fn create_rejects_invalid_due_repeat_and_links() {
    let pool = crate::test_db().await;
    let try_create = |i: TaskInput| {
        let pool = pool.clone();
        async move { err_msg(inner_create_task(&i, &[], now(), &pool).await) }
    };
    assert_eq!(
        try_create(due("x", "2026-13-01", None)).await,
        "Invalid due date"
    );
    assert_eq!(
        try_create(due("x", "2026-11-20", Some("25:00"))).await,
        "Invalid due time"
    );
    let time_only = TaskInput {
        due_time: Some("09:00".into()),
        ..input("x")
    };
    assert_eq!(try_create(time_only).await, "A due time needs a due date");
    let repeat_no_due = TaskInput {
        repeat: "daily".into(),
        ..input("x")
    };
    assert_eq!(
        try_create(repeat_no_due).await,
        "A repeating task needs a due date"
    );
    let bad_weekday = TaskInput {
        repeat: "weekly".into(),
        weekdays: vec![8],
        ..due("x", "2026-11-20", None)
    };
    assert_eq!(try_create(bad_weekday).await, "Invalid weekday");
    let bad_repeat = TaskInput {
        repeat: "hourly".into(),
        ..due("x", "2026-11-20", None)
    };
    assert_eq!(try_create(bad_repeat).await, "Invalid repeat");
    let issue = TaskInput {
        issue_id: Some(999),
        ..input("x")
    };
    assert_eq!(try_create(issue).await, "Issue not found");
    let note = TaskInput {
        note_id: Some(999),
        ..input("x")
    };
    assert_eq!(try_create(note).await, "Note not found");
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM tasks")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0, "nothing was written");
}

#[tokio::test]
async fn due_time_drops_seconds_and_repeat_defaults_come_from_the_due_date() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-20", Some("09:30:15")), &[], &pool).await;
    assert_eq!(t.due_time.as_deref(), Some("09:30"));
    // 2026-11-20 is a Friday.
    let weekly = create(
        TaskInput {
            repeat: "weekly".into(),
            ..due("w", "2026-11-20", None)
        },
        &[],
        &pool,
    )
    .await;
    assert_eq!(
        (weekly.repeat.as_str(), weekly.weekdays.clone()),
        ("weekly", vec![5])
    );
    let monthly = create(
        TaskInput {
            repeat: "monthly".into(),
            ..due("m", "2026-11-20", None)
        },
        &[],
        &pool,
    )
    .await;
    assert_eq!(monthly.month_day, Some(20));
}

#[tokio::test]
async fn reminders_are_validated_and_capped_at_ten() {
    let pool = crate::test_db().await;
    let try_create = |rs: Vec<TaskReminderInput>| {
        let pool = pool.clone();
        async move { err_msg(inner_create_task(&due("x", "2026-11-25", None), &rs, now(), &pool).await) }
    };
    assert_eq!(
        try_create(vec![offset(0); 11]).await,
        "A task can have at most 10 reminders"
    );
    assert_eq!(
        try_create(vec![TaskReminderInput::default()]).await,
        "Pick when to remind"
    );
    let both = TaskReminderInput {
        offset_minutes: Some(0),
        at: Some("2026-11-25T09:00".into()),
    };
    assert_eq!(try_create(vec![both]).await, "Pick when to remind");
    assert_eq!(try_create(vec![offset(-5)]).await, "Invalid reminder");
    assert_eq!(
        try_create(vec![custom_at("2026-11-19T11:00")]).await,
        "Time is in the past"
    );
    assert_eq!(try_create(vec![custom_at("soon")]).await, "Invalid time");

    let t = create(due("x", "2026-11-25", None), &vec![offset(0); 10], &pool).await;
    assert_eq!(t.reminders.len(), 10);
    let eleventh = inner_add_task_reminder(t.id, &offset(15), now(), &pool).await;
    assert_eq!(err_msg(eleventh), "A task can have at most 10 reminders");
    assert_eq!(
        err_msg(inner_add_task_reminder(999, &offset(15), now(), &pool).await),
        "Task not found"
    );
}

#[tokio::test]
async fn update_edits_fields_links_and_keeps_or_sets_mac_only() {
    let pool = crate::test_db().await;
    let c = inner_create_customer("Acme", &pool).await.unwrap();
    let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
    let issue = inner_resolve_issue(p.id, "Login bug", &pool).await.unwrap();
    let note = crate::commands::notes::inner_create_note_titled("Plan", None, &pool)
        .await
        .unwrap();
    let t = create(
        TaskInput {
            mac_only: Some(true),
            ..input("a")
        },
        &[],
        &pool,
    )
    .await;
    assert!(t.mac_only);

    let t = edit(
        &t,
        |i| {
            i.title = "b".into();
            i.notes = Some(" n ".into());
            i.due_date = Some("2026-12-24".into());
            i.due_time = Some("14:00".into());
            i.issue_id = Some(issue);
            i.note_id = Some(note.id);
        },
        NOW,
        &pool,
    )
    .await;
    assert_eq!(t.title, "b");
    assert_eq!(t.notes.as_deref(), Some("n"));
    assert_eq!(
        (t.due_date.as_deref(), t.due_time.as_deref()),
        (Some("2026-12-24"), Some("14:00"))
    );
    assert_eq!(t.issue_title.as_deref(), Some("Login bug"));
    assert_eq!(t.project_name.as_deref(), Some("Web"));
    assert_eq!(t.customer_name.as_deref(), Some("Acme"));
    assert_eq!(t.note_title.as_deref(), Some("Plan"));
    assert!(t.mac_only, "an update that doesn't say keeps it");

    let t = edit(
        &t,
        |i| {
            i.issue_id = None;
            i.note_id = None;
            i.due_date = None;
            i.due_time = None;
            i.mac_only = Some(false);
        },
        NOW,
        &pool,
    )
    .await;
    assert_eq!((t.issue_id, t.note_id, t.due_date), (None, None, None));
    assert!(!t.mac_only);

    let missing = inner_update_task(42, &input("x"), now(), &pool).await;
    assert_eq!(err_msg(missing), "Task not found");
    assert_eq!(
        err_msg(inner_delete_task(42, &pool).await),
        "Task not found"
    );
    assert_eq!(err_msg(inner_get_task(42, &pool).await), "Task not found");
    assert_eq!(
        err_msg(inner_create_issue_for_task(42, p.id, "x", &pool).await),
        "Task not found"
    );
}

#[tokio::test]
async fn create_issue_for_task_creates_then_reuses() {
    let pool = crate::test_db().await;
    let c = inner_create_customer("Acme", &pool).await.unwrap();
    let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
    let a = create(input("Fix login"), &[], &pool).await;
    let b = create(input("Other"), &[], &pool).await;
    let a = inner_create_issue_for_task(a.id, p.id, "Fix login", &pool)
        .await
        .unwrap();
    let b = inner_create_issue_for_task(b.id, p.id, "Fix login", &pool)
        .await
        .unwrap();
    assert!(a.issue_id.is_some());
    assert_eq!(a.issue_id, b.issue_id, "same title reuses the issue");
}

#[tokio::test]
async fn list_orders_open_by_due_then_time_untimed_last_then_done_newest_first() {
    let pool = crate::test_db().await;
    create(input("no date"), &[], &pool).await;
    create(due("late untimed", "2026-11-20", None), &[], &pool).await;
    create(due("late 09", "2026-11-20", Some("09:00")), &[], &pool).await;
    create(due("early", "2026-11-18", Some("18:00")), &[], &pool).await;
    let d1 = create(input("done first"), &[], &pool).await;
    let d2 = create(input("done second"), &[], &pool).await;
    inner_complete_task(d1.id, at("2026-11-19T10:00"), &pool)
        .await
        .unwrap();
    inner_complete_task(d2.id, at("2026-11-19T11:00"), &pool)
        .await
        .unwrap();

    let titles: Vec<String> = inner_list_tasks(None, &pool)
        .await
        .unwrap()
        .into_iter()
        .map(|t| t.title)
        .collect();
    assert_eq!(
        titles,
        vec![
            "early",
            "late 09",
            "late untimed",
            "no date",
            "done second",
            "done first"
        ]
    );
    let limited = inner_list_tasks(Some(1), &pool).await.unwrap();
    assert_eq!(limited.len(), 5);
    assert_eq!(limited[4].title, "done second");
}

// ── AC3: fire time math and D20 states ────────────────────────────────────────

#[tokio::test]
async fn fire_times_relative_to_a_timed_or_date_only_due_and_custom() {
    let pool = crate::test_db().await;
    let timed = create(
        due("timed", "2026-11-20", Some("14:00")),
        &[
            offset(0),
            offset(60),
            offset(1440),
            custom_at("2026-11-19T18:30"),
        ],
        &pool,
    )
    .await;
    assert_eq!(
        view(&timed),
        vec![
            (Some(1440), None, "pending".into(), ts("2026-11-19T14:00")),
            (
                None,
                ts("2026-11-19T18:30"),
                "pending".into(),
                ts("2026-11-19T18:30")
            ),
            (Some(60), None, "pending".into(), ts("2026-11-20T13:00")),
            (Some(0), None, "pending".into(), ts("2026-11-20T14:00")),
        ]
    );
    // A date-only due counts from 09:00 that day.
    let date_only = create(
        due("date", "2026-11-21", None),
        &[offset(0), offset(10080)],
        &pool,
    )
    .await;
    assert_eq!(
        view(&date_only),
        vec![
            (
                Some(10080),
                None,
                "dismissed".into(),
                ts("2026-11-14T09:00")
            ),
            (Some(0), None, "pending".into(), ts("2026-11-21T09:00")),
        ],
        "a relative time already past when saved is dismissed silently"
    );
    // No due: relative reminders are dormant, custom ones still fire.
    let no_due = create(
        input("none"),
        &[offset(15), custom_at("2026-11-20T08:00")],
        &pool,
    )
    .await;
    assert_eq!(
        view(&no_due),
        vec![
            (
                None,
                ts("2026-11-20T08:00"),
                "pending".into(),
                ts("2026-11-20T08:00")
            ),
            (Some(15), None, "pending".into(), None),
        ]
    );
    let sent = fire("2026-12-31T00:00", &pool).await;
    assert!(
        !sent
            .iter()
            .any(|f| f.task_id == no_due.id && f.fire_at != at("2026-11-20T08:00")),
        "a relative reminder without a due never fires: {sent:?}"
    );
}

#[test]
fn due_anchor_and_base_time_math() {
    let always = |_: NaiveDateTime| true;
    let anchor = due_anchor(Some("2026-11-20"), Some("14:00"), always);
    assert_eq!(anchor, Some(at("2026-11-20T14:00")));
    assert_eq!(
        due_anchor(Some("2026-11-20"), None, always),
        Some(at("2026-11-20T09:00"))
    );
    assert_eq!(due_anchor(None, None, always), None);
    assert_eq!(
        base_fire_time(anchor, Some(15), None, always),
        Some(at("2026-11-20T13:45"))
    );
    assert_eq!(base_fire_time(None, Some(15), None, always), None);
    assert_eq!(
        base_fire_time(None, None, Some("2026-11-21T07:00:00.000"), always),
        Some(at("2026-11-21T07:00"))
    );
    // DST gap: a reminder landing in it moves to the first valid minute after.
    let gap_day = NaiveDate::from_ymd_opt(2027, 3, 28).unwrap();
    use chrono::Timelike;
    let exists = move |n: NaiveDateTime| !(n.date() == gap_day && n.hour() == 2);
    let anchor = due_anchor(Some("2027-03-28"), Some("03:30"), exists);
    assert_eq!(
        base_fire_time(anchor, Some(60), None, exists),
        Some(at("2027-03-28T03:00"))
    );
}

#[tokio::test]
async fn due_edits_rearm_future_reminders_and_silently_dismiss_past_ones() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-19", Some("13:00")), &[offset(0)], &pool).await;
    assert_eq!(fire("2026-11-19T13:00:10", &pool).await.len(), 1);
    assert_eq!(
        inner_get_task(t.id, &pool).await.unwrap().reminders[0].state,
        "sent"
    );

    // Moved to the future: back to pending, fires again at the new time.
    let t = edit(
        &t,
        |i| i.due_date = Some("2026-11-20".into()),
        "2026-11-19T13:05",
        &pool,
    )
    .await;
    assert_eq!(
        view(&t)[0],
        (Some(0), None, "pending".into(), ts("2026-11-20T13:00"))
    );
    assert!(fire("2026-11-19T13:06", &pool).await.is_empty());

    // Moved into the past: dismissed silently, never fires, doesn't count.
    let t = edit(
        &t,
        |i| i.due_date = Some("2026-11-18".into()),
        "2026-11-19T13:10",
        &pool,
    )
    .await;
    assert_eq!(t.reminders[0].state, "dismissed");
    assert!(fire("2026-11-21T00:00", &pool).await.is_empty());
    assert_eq!(
        overdue("2026-11-19T13:10", &pool).await,
        1,
        "overdue by date only"
    );

    // A title-only edit leaves the state alone.
    let t = edit(&t, |i| i.title = "y".into(), "2026-11-19T13:11", &pool).await;
    assert_eq!(t.reminders[0].state, "dismissed");
}

#[tokio::test]
async fn clearing_the_due_time_moves_at_due_time_to_nine_and_clearing_the_date_makes_it_dormant() {
    let pool = crate::test_db().await;
    let t = create(
        due("x", "2026-11-21", Some("15:00")),
        &[offset(0), custom_at("2026-11-20T10:00")],
        &pool,
    )
    .await;
    let t = edit(&t, |i| i.due_time = None, NOW, &pool).await;
    assert_eq!(
        view(&t)[1],
        (Some(0), None, "pending".into(), ts("2026-11-21T09:00"))
    );

    let t = edit(&t, |i| i.due_date = None, NOW, &pool).await;
    assert_eq!(
        view(&t),
        vec![
            (
                None,
                ts("2026-11-20T10:00"),
                "pending".into(),
                ts("2026-11-20T10:00")
            ),
            (Some(0), None, "pending".into(), None),
        ]
    );
    let sent = fire("2026-11-22T00:00", &pool).await;
    assert_eq!(sent.len(), 1, "only the custom one fires");
    assert_eq!(sent[0].fire_at, at("2026-11-20T10:00"));
}

#[tokio::test]
async fn editing_a_reminder_rearms_or_dismisses_and_delete_removes_it() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-19", Some("12:30")), &[offset(0)], &pool).await;
    fire("2026-11-19T12:30:10", &pool).await;
    let id = t.reminders[0].id;
    let t = inner_update_task_reminder(id, &offset(15), at("2026-11-19T12:31"), &pool)
        .await
        .unwrap();
    assert_eq!(t.reminders[0].state, "dismissed", "12:15 has passed");
    let t = inner_update_task_reminder(
        id,
        &custom_at("2026-11-19T18:00"),
        at("2026-11-19T12:31"),
        &pool,
    )
    .await
    .unwrap();
    assert_eq!(
        view(&t)[0],
        (
            None,
            ts("2026-11-19T18:00"),
            "pending".into(),
            ts("2026-11-19T18:00")
        )
    );
    assert_eq!(
        err_msg(inner_update_task_reminder(id, &custom_at("2026-11-19T08:00"), now(), &pool).await),
        "Time is in the past"
    );
    let t = inner_delete_task_reminder(id, &pool).await.unwrap();
    assert!(t.reminders.is_empty());
    assert_eq!(
        err_msg(inner_delete_task_reminder(id, &pool).await),
        "Reminder not found"
    );
}

#[tokio::test]
async fn snooze_moves_only_that_reminder_and_never_the_due() {
    let pool = crate::test_db().await;
    let t = create(
        due("x", "2026-11-19", Some("12:30")),
        &[offset(0), offset(10)],
        &pool,
    )
    .await;
    fire("2026-11-19T12:30:10", &pool).await;
    let id = t
        .reminders
        .iter()
        .find(|r| r.offset_minutes == Some(0))
        .unwrap()
        .id;
    let t = inner_snooze_task_reminder(id, "2026-11-19T13:00", at("2026-11-19T12:31"), &pool)
        .await
        .unwrap();
    assert_eq!(
        (t.due_date.as_deref(), t.due_time.as_deref()),
        (Some("2026-11-19"), Some("12:30"))
    );
    let r = t.reminders.iter().find(|r| r.id == id).unwrap();
    assert_eq!(
        (r.state.as_str(), r.fire_at.clone()),
        ("pending", ts("2026-11-19T13:00"))
    );
    assert_eq!(r.snoozed_until, ts("2026-11-19T13:00"));
    let other = t.reminders.iter().find(|r| r.id != id).unwrap();
    assert_eq!(other.state, "sent");
    assert_eq!(
        err_msg(
            inner_snooze_task_reminder(id, "2026-11-19T12:00", at("2026-11-19T12:31"), &pool).await
        ),
        "Time is in the past"
    );
    let sent = fire("2026-11-19T13:00:05", &pool).await;
    assert_eq!(
        sent.iter().map(|f| f.reminder_id).collect::<Vec<_>>(),
        vec![id]
    );
    assert_eq!(sent[0].fire_at, at("2026-11-19T13:00"));
}

#[tokio::test]
async fn dismissing_stops_counting_toward_overdue() {
    let pool = crate::test_db().await;
    // No due date: only the sent reminder makes it overdue.
    let t = create(input("x"), &[custom_at("2026-11-19T12:30")], &pool).await;
    assert_eq!(overdue("2026-11-19T12:29", &pool).await, 0);
    fire("2026-11-19T12:30:10", &pool).await;
    assert_eq!(overdue("2026-11-19T12:31", &pool).await, 1);
    let t = inner_dismiss_task_reminder(t.reminders[0].id, &pool)
        .await
        .unwrap();
    assert_eq!(t.reminders[0].state, "dismissed");
    assert_eq!(overdue("2026-11-19T12:31", &pool).await, 0);
    assert_eq!(
        err_msg(inner_dismiss_task_reminder(999, &pool).await),
        "Reminder not found"
    );
}

#[tokio::test]
async fn overdue_counts_past_due_dates_and_times_that_have_come() {
    let pool = crate::test_db().await;
    create(due("yesterday", "2026-11-18", None), &[], &pool).await;
    create(due("today untimed", "2026-11-19", None), &[], &pool).await;
    create(due("today 11:00", "2026-11-19", Some("11:00")), &[], &pool).await;
    create(due("today 12:00", "2026-11-19", Some("12:00")), &[], &pool).await;
    create(due("today 13:00", "2026-11-19", Some("13:00")), &[], &pool).await;
    create(due("tomorrow", "2026-11-20", None), &[], &pool).await;
    let done = create(due("done", "2026-11-01", None), &[], &pool).await;
    inner_complete_task(done.id, now(), &pool).await.unwrap();
    assert_eq!(overdue("2026-11-19T12:00", &pool).await, 3);
    assert_eq!(overdue("2026-11-19T13:00", &pool).await, 4);
    assert_eq!(overdue("2026-11-20T00:00", &pool).await, 5);
}

#[tokio::test]
async fn completing_suppresses_reminders_and_reopening_dismisses_past_ones_silently() {
    let pool = crate::test_db().await;
    let t = create(
        due("x", "2026-11-20", Some("10:00")),
        &[offset(0), custom_at("2026-11-19T15:00")],
        &pool,
    )
    .await;
    let done = inner_complete_task(t.id, now(), &pool).await.unwrap();
    assert!(done.task.done);
    assert_eq!(done.task.completed_at, ts(NOW));
    assert!(
        done.task.reminders.iter().all(|r| r.state == "pending"),
        "rows untouched"
    );
    assert!(
        fire("2026-11-19T16:00", &pool).await.is_empty(),
        "suppressed"
    );
    assert_eq!(overdue("2026-11-21T00:00", &pool).await, 0);

    // Reopened between the two: the 15:00 one has passed and stays quiet; 10:00 tomorrow
    // still fires.
    let t = inner_reopen_task(t.id, at("2026-11-19T16:00"), &pool)
        .await
        .unwrap();
    assert!(!t.done && t.completed_at.is_none());
    assert_eq!(
        view(&t),
        vec![
            (
                None,
                ts("2026-11-19T15:00"),
                "dismissed".into(),
                ts("2026-11-19T15:00")
            ),
            (Some(0), None, "pending".into(), ts("2026-11-20T10:00")),
        ]
    );
    assert!(fire("2026-11-19T16:01", &pool).await.is_empty());
    assert_eq!(fire("2026-11-20T10:00:05", &pool).await.len(), 1);
    // Reopening an open task is a no-op; completing a done one has nothing to undo.
    inner_reopen_task(t.id, now(), &pool).await.unwrap();
    inner_complete_task(t.id, now(), &pool).await.unwrap();
    let again = inner_complete_task(t.id, now(), &pool).await.unwrap();
    assert!(again.undo.is_none());
}

#[tokio::test]
async fn completing_a_repeating_task_advances_the_due_and_rearms_relative_reminders_only() {
    let pool = crate::test_db().await;
    let t = create(
        TaskInput {
            repeat: "daily".into(),
            ..due("standup", "2026-11-19", Some("09:30"))
        },
        &[offset(0), offset(15), custom_at("2026-11-19T12:30")],
        &pool,
    )
    .await;
    // The 09:15 and 09:30 ones were past when saved (dismissed); the custom one fires.
    fire("2026-11-19T12:30:10", &pool).await;
    let custom_id = t.reminders.iter().find(|r| r.at.is_some()).unwrap().id;
    let snoozed =
        inner_snooze_task_reminder(custom_id, "2026-11-19T13:00", at("2026-11-19T12:31"), &pool)
            .await
            .unwrap();
    let before = snoozed
        .reminders
        .iter()
        .find(|r| r.id == custom_id)
        .cloned()
        .unwrap();

    let done = inner_complete_task(t.id, at("2026-11-19T12:40"), &pool)
        .await
        .unwrap();
    let after = done.task;
    assert!(!after.done, "a repeating task stays open");
    assert_eq!(
        (after.due_date.as_deref(), after.due_time.as_deref()),
        (Some("2026-11-20"), Some("09:30"))
    );
    let relative: Vec<(Option<i64>, String, Option<String>)> = after
        .reminders
        .iter()
        .filter(|r| r.offset_minutes.is_some())
        .map(|r| (r.offset_minutes, r.state.clone(), r.fire_at.clone()))
        .collect();
    assert_eq!(
        relative,
        vec![
            (Some(15), "pending".into(), ts("2026-11-20T09:15")),
            (Some(0), "pending".into(), ts("2026-11-20T09:30")),
        ]
    );
    assert_eq!(
        after.reminders.iter().find(|r| r.id == custom_id).unwrap(),
        &before,
        "a custom reminder stays as it is (snooze included)"
    );

    // Overdue occurrence: it stays on its date until completed, then moves past today.
    let weekly = create(
        TaskInput {
            repeat: "weekly".into(),
            weekdays: vec![1],
            ..due("weekly", "2026-11-09", None)
        },
        &[],
        &pool,
    )
    .await;
    assert_eq!(overdue(NOW, &pool).await, 1);
    let moved = inner_complete_task(weekly.id, now(), &pool)
        .await
        .unwrap()
        .task;
    assert_eq!(moved.due_date.as_deref(), Some("2026-11-23"));
}

// ── AC11 (Rust): undo ─────────────────────────────────────────────────────────

fn comparable(t: &Task) -> Task {
    Task {
        updated_at: String::new(),
        ..t.clone()
    }
}

#[tokio::test]
async fn undo_restores_the_exact_snapshot_of_a_completion() {
    let pool = crate::test_db().await;
    let t = create(
        due("x", "2026-11-19", Some("12:30")),
        &[offset(0), offset(60), custom_at("2026-11-20T08:00")],
        &pool,
    )
    .await;
    fire("2026-11-19T12:30:10", &pool).await;
    let snoozed_id = t
        .reminders
        .iter()
        .find(|r| r.offset_minutes == Some(0))
        .unwrap()
        .id;
    inner_snooze_task_reminder(
        snoozed_id,
        "2026-11-19T14:00",
        at("2026-11-19T12:31"),
        &pool,
    )
    .await
    .unwrap();
    let before = inner_get_task(t.id, &pool).await.unwrap();

    let done = inner_complete_task(t.id, at("2026-11-19T12:35"), &pool)
        .await
        .unwrap();
    let undo = done.undo.unwrap();
    let back = inner_undo_complete_task(&undo, &pool).await.unwrap();
    assert_eq!(comparable(&back), comparable(&before));
    let raw_before: Vec<ReminderRow> = {
        let mut conn = pool.acquire().await.unwrap();
        load_reminder_rows(t.id, &mut conn).await.unwrap()
    };
    assert!(raw_before
        .iter()
        .all(|r| undo.reminders.iter().any(|u| u.id == r.id
            && u.state == r.state
            && u.fired_for == r.fired_for
            && u.snoozed_until == r.snoozed_until)));

    // A second undo (or one after another change) is refused.
    assert_eq!(
        err_msg(inner_undo_complete_task(&undo, &pool).await),
        "The task changed meanwhile, so it can't be undone"
    );
    let mut gone = undo.clone();
    gone.task_id = 999;
    assert_eq!(
        err_msg(inner_undo_complete_task(&gone, &pool).await),
        "Task not found"
    );
}

#[tokio::test]
async fn undo_restores_a_repeat_advance() {
    let pool = crate::test_db().await;
    let t = create(
        TaskInput {
            repeat: "weekdays".into(),
            ..due("x", "2026-11-20", Some("10:00"))
        },
        &[offset(30)],
        &pool,
    )
    .await;
    let before = inner_get_task(t.id, &pool).await.unwrap();
    let done = inner_complete_task(t.id, now(), &pool).await.unwrap();
    assert_eq!(
        done.task.due_date.as_deref(),
        Some("2026-11-23"),
        "Friday → Monday"
    );
    let back = inner_undo_complete_task(&done.undo.unwrap(), &pool)
        .await
        .unwrap();
    assert_eq!(comparable(&back), comparable(&before));
}

async fn set_updated_at(id: i64, stamp: &str, pool: &Pool<Sqlite>) {
    sqlx::query("UPDATE tasks SET UpdatedAt = ? WHERE id = ?")
        .bind(stamp)
        .bind(id)
        .execute(pool)
        .await
        .unwrap();
}

async fn updated_at(id: i64, pool: &Pool<Sqlite>) -> String {
    sqlx::query_scalar("SELECT UpdatedAt FROM tasks WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_second_undo_is_refused_even_within_the_same_millisecond() {
    let pool = crate::test_db().await;
    let one_off = create(due("one-off", "2026-11-20", None), &[], &pool).await;
    let repeating = create(
        TaskInput {
            repeat: "daily".into(),
            ..due("daily", "2026-11-20", Some("10:00"))
        },
        &[],
        &pool,
    )
    .await;
    for t in [one_off, repeating] {
        let undo = inner_complete_task(t.id, now(), &pool)
            .await
            .unwrap()
            .undo
            .unwrap();
        inner_undo_complete_task(&undo, &pool).await.unwrap();
        // The undo's stamp landing on the completion's (same millisecond) must not let the
        // same undo apply twice: the task no longer looks like the completion left it.
        set_updated_at(t.id, &undo.completed_updated_at, &pool).await;
        assert_eq!(
            err_msg(inner_undo_complete_task(&undo, &pool).await),
            "The task changed meanwhile, so it can't be undone",
            "{}",
            t.title
        );
    }
}

#[tokio::test]
async fn a_reminder_edit_after_completion_refuses_undo() {
    const REFUSED: &str = "The task changed meanwhile, so it can't be undone";
    let pool = crate::test_db().await;
    // One-off: dismissing its sent reminder's toast after completing it.
    let t = create(due("x", "2026-11-19", Some("12:30")), &[offset(0)], &pool).await;
    fire("2026-11-19T12:30:10", &pool).await;
    let undo = inner_complete_task(t.id, at("2026-11-19T12:31"), &pool)
        .await
        .unwrap()
        .undo
        .unwrap();
    let before = updated_at(t.id, &pool).await;
    let task = inner_dismiss_task_reminder(t.reminders[0].id, &pool)
        .await
        .unwrap();
    assert!(task.updated_at > before, "dismiss bumps the task");
    assert_eq!(
        err_msg(inner_undo_complete_task(&undo, &pool).await),
        REFUSED
    );
    assert_eq!(
        inner_get_task(t.id, &pool).await.unwrap().reminders[0].state,
        "dismissed",
        "the dismissal stands"
    );

    // Repeating (open after completion): add, update, snooze and delete each refuse undo.
    let repeating = || TaskInput {
        repeat: "daily".into(),
        ..due("daily", "2026-11-20", Some("10:00"))
    };
    let edits: [&str; 4] = ["add", "update", "snooze", "delete"];
    for edit in edits {
        let t = create(repeating(), &[offset(30)], &pool).await;
        let rid = t.reminders[0].id;
        let undo = inner_complete_task(t.id, now(), &pool)
            .await
            .unwrap()
            .undo
            .unwrap();
        match edit {
            "add" => inner_add_task_reminder(t.id, &offset(60), now(), &pool).await,
            "update" => inner_update_task_reminder(rid, &offset(15), now(), &pool).await,
            "snooze" => inner_snooze_task_reminder(rid, "2026-11-19T15:00", now(), &pool).await,
            _ => inner_delete_task_reminder(rid, &pool).await,
        }
        .unwrap();
        assert_eq!(
            err_msg(inner_undo_complete_task(&undo, &pool).await),
            REFUSED,
            "{edit}"
        );
    }

    // A no-op write (snoozing a done task's reminder) leaves undo available.
    let t = create(due("y", "2026-11-20", Some("10:00")), &[offset(0)], &pool).await;
    let undo = inner_complete_task(t.id, now(), &pool)
        .await
        .unwrap()
        .undo
        .unwrap();
    inner_snooze_task_reminder(t.reminders[0].id, "2026-11-19T15:00", now(), &pool)
        .await
        .unwrap();
    assert!(!inner_undo_complete_task(&undo, &pool).await.unwrap().done);
}

#[tokio::test]
async fn the_scheduler_firing_after_completion_keeps_undo() {
    let pool = crate::test_db().await;
    // Completing moves the daily task to tomorrow 10:00; its reminder then fires.
    let t = create(
        TaskInput {
            repeat: "daily".into(),
            ..due("daily", "2026-11-19", Some("10:00"))
        },
        &[offset(0)],
        &pool,
    )
    .await;
    let undo = inner_complete_task(t.id, now(), &pool)
        .await
        .unwrap()
        .undo
        .unwrap();
    let before = updated_at(t.id, &pool).await;
    assert_eq!(fire("2026-11-20T10:00:10", &pool).await.len(), 1);
    assert_eq!(updated_at(t.id, &pool).await, before, "firing doesn't bump");
    let back = inner_undo_complete_task(&undo, &pool).await.unwrap();
    assert_eq!(back.due_date.as_deref(), Some("2026-11-19"));
}

#[tokio::test]
async fn completion_and_undo_stamps_always_move_forward() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-20", None), &[], &pool).await;
    // A clock that reads earlier than the stored stamp (or the same millisecond).
    let future = "2999-01-01T00:00:00.000";
    set_updated_at(t.id, future, &pool).await;
    let undo = inner_complete_task(t.id, now(), &pool)
        .await
        .unwrap()
        .undo
        .unwrap();
    assert!(undo.completed_updated_at.as_str() > future);
    assert_eq!(updated_at(t.id, &pool).await, undo.completed_updated_at);
    inner_undo_complete_task(&undo, &pool).await.unwrap();
    assert!(updated_at(t.id, &pool).await > undo.completed_updated_at);
}

#[tokio::test]
async fn a_malformed_undo_is_rejected() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-20", Some("10:00")), &[offset(0)], &pool).await;
    let undo = inner_complete_task(t.id, now(), &pool)
        .await
        .unwrap()
        .undo
        .unwrap();
    let bad: Vec<TaskUndo> = vec![
        TaskUndo {
            due_date: Some("2026-13-40".into()),
            ..undo.clone()
        },
        TaskUndo {
            due_time: Some("25:00".into()),
            ..undo.clone()
        },
        TaskUndo {
            due_date: None,
            ..undo.clone()
        },
        TaskUndo {
            completed_at: Some("yesterday".into()),
            ..undo.clone()
        },
        TaskUndo {
            reminders: vec![ReminderUndo {
                fired_for: Some("soon".into()),
                ..undo.reminders[0].clone()
            }],
            ..undo.clone()
        },
        TaskUndo {
            reminders: vec![ReminderUndo {
                snoozed_until: Some("later".into()),
                ..undo.reminders[0].clone()
            }],
            ..undo.clone()
        },
        TaskUndo {
            reminders: vec![ReminderUndo {
                state: "fired".into(),
                ..undo.reminders[0].clone()
            }],
            ..undo.clone()
        },
    ];
    for b in &bad {
        assert_eq!(
            err_msg(inner_undo_complete_task(b, &pool).await),
            "This undo is no longer valid",
            "{b:?}"
        );
    }
    assert!(
        inner_get_task(t.id, &pool).await.unwrap().done,
        "nothing applied"
    );
    assert!(!inner_undo_complete_task(&undo, &pool).await.unwrap().done);
}

#[tokio::test]
async fn a_repeat_advance_dismisses_sent_custom_reminders_and_keeps_pending_ones() {
    let pool = crate::test_db().await;
    let t = create(
        TaskInput {
            repeat: "daily".into(),
            ..due("standup", "2026-11-19", Some("18:00"))
        },
        &[custom_at("2026-11-19T12:30"), custom_at("2026-11-21T08:00")],
        &pool,
    )
    .await;
    assert_eq!(fire("2026-11-19T12:30:10", &pool).await.len(), 1);
    assert_eq!(overdue("2026-11-19T12:31", &pool).await, 1, "sent counts");

    let done = inner_complete_task(t.id, at("2026-11-19T12:40"), &pool)
        .await
        .unwrap();
    assert!(!done.task.done);
    let states: Vec<(Option<String>, String)> = done
        .task
        .reminders
        .iter()
        .map(|r| (r.at.clone(), r.state.clone()))
        .collect();
    assert_eq!(
        states,
        vec![
            (ts("2026-11-19T12:30"), "dismissed".into()),
            (ts("2026-11-21T08:00"), "pending".into()),
        ]
    );
    assert_eq!(
        overdue("2026-11-19T12:41", &pool).await,
        0,
        "no longer counts"
    );

    // Undo puts the sent one back.
    let back = inner_undo_complete_task(&done.undo.unwrap(), &pool)
        .await
        .unwrap();
    assert_eq!(back.reminders[0].state, "sent");
}

// ── AC7: links and cascades ───────────────────────────────────────────────────

#[tokio::test]
async fn deleting_a_note_or_issue_unlinks_and_deleting_a_task_deletes_its_reminders() {
    let pool = crate::test_db().await;
    let c = inner_create_customer("Acme", &pool).await.unwrap();
    let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
    let issue = inner_resolve_issue(p.id, "Bug", &pool).await.unwrap();
    let note = crate::commands::notes::inner_create_note_titled("Plan", None, &pool)
        .await
        .unwrap();
    let t = create(
        TaskInput {
            issue_id: Some(issue),
            note_id: Some(note.id),
            ..input("linked")
        },
        &[offset(0), custom_at("2026-11-20T09:00")],
        &pool,
    )
    .await;

    crate::commands::notes::inner_delete_note(note.id, &pool)
        .await
        .unwrap();
    let t = inner_get_task(t.id, &pool).await.unwrap();
    assert_eq!((t.note_id, t.note_title.clone()), (None, None));
    assert_eq!(t.issue_id, Some(issue));
    sqlx::query("DELETE FROM customers")
        .execute(&pool)
        .await
        .unwrap();
    let t = inner_get_task(t.id, &pool).await.unwrap();
    assert_eq!(t.issue_id, None);
    assert_eq!(t.reminders.len(), 2, "unlinking keeps the reminders");

    inner_delete_task(t.id, &pool).await.unwrap();
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_reminders")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert!(inner_list_tasks(None, &pool).await.unwrap().is_empty());
}

// ── Scheduler side ────────────────────────────────────────────────────────────

#[tokio::test]
async fn fire_due_sends_each_due_reminder_once_by_time() {
    let pool = crate::test_db().await;
    create(input("b"), &[custom_at("2026-11-19T12:10")], &pool).await;
    create(input("a"), &[custom_at("2026-11-19T12:05")], &pool).await;
    create(input("later"), &[custom_at("2026-11-19T18:00")], &pool).await;
    let sent = fire("2026-11-19T12:10:20", &pool).await;
    let titles: Vec<&str> = sent.iter().map(|f| f.task_title.as_str()).collect();
    assert_eq!(titles, vec!["a", "b"]);
    assert!(fire("2026-11-19T12:10:50", &pool).await.is_empty(), "once");
}

#[tokio::test]
async fn a_stale_read_never_sends() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-19", Some("12:30")), &[offset(0)], &pool).await;
    let r = &t.reminders[0];
    let read = PendingRow {
        id: r.id,
        task_id: t.id,
        offset_minutes: Some(0),
        at: None,
        snoozed_until: None,
        fired_for: ts("2026-11-19T12:30"),
        title: t.title.clone(),
        note_title: None,
        due_date: t.due_date.clone(),
        due_time: t.due_time.clone(),
        updated_at: t.updated_at.clone(),
    };
    // The due moved after the scheduler read the row: the old time must not fire.
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    edit(&t, |i| i.due_time = Some("18:00".into()), NOW, &pool).await;
    assert!(!mark_sent(&read, &pool).await.unwrap());
    // Same fire time but the task changed (completed) meanwhile: not sent either.
    let t2 = create(input("y"), &[custom_at("2026-11-19T12:30")], &pool).await;
    let stale = PendingRow {
        id: t2.reminders[0].id,
        task_id: t2.id,
        offset_minutes: None,
        at: t2.reminders[0].at.clone(),
        fired_for: t2.reminders[0].at.clone(),
        updated_at: t2.updated_at.clone(),
        title: t2.title.clone(),
        ..read.clone()
    };
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    inner_complete_task(t2.id, now(), &pool).await.unwrap();
    assert!(!mark_sent(&stale, &pool).await.unwrap());
    // A fresh read sends.
    inner_reopen_task(t2.id, now(), &pool).await.unwrap();
    assert_eq!(fire("2026-11-19T12:30:10", &pool).await.len(), 1);
}

#[tokio::test]
async fn a_reminder_whose_time_moved_behind_its_back_is_reconciled_not_sent() {
    let pool = crate::test_db().await;
    let t = create(due("x", "2026-11-20", Some("12:30")), &[offset(0)], &pool).await;
    // A raw write (an older build, a hand edit) moves the due without reconciling.
    sqlx::query("UPDATE tasks SET DueDate = '2026-11-18' WHERE id = ?")
        .bind(t.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(fire(NOW, &pool).await.is_empty());
    let t = inner_get_task(t.id, &pool).await.unwrap();
    assert_eq!(t.reminders[0].state, "dismissed");
}
