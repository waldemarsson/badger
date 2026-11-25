use super::*;
use crate::app_settings::set_apple_sync_enabled;
use crate::apple_sync::fake::{phone, FakeStore};
use crate::apple_sync::PhoneDue;
use crate::commands::tasks::{
    inner_complete_task, inner_create_task, inner_delete_task, inner_get_task, inner_list_tasks,
    inner_snooze_task_reminder, inner_undo_complete_task, inner_update_task, Task, TaskInput,
    TaskReminderInput,
};
use crate::recurrence::parse_timestamp;
use chrono::NaiveDate;

const NOW: &str = "2026-11-19T12:00";
/// When test tasks are created (before NOW).
const CREATED: &str = "2026-11-18T08:00";

fn at(s: &str) -> NaiveDateTime {
    parse_timestamp(s).unwrap()
}

fn date(s: &str) -> NaiveDate {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
}

async fn setup() -> (Pool<Sqlite>, FakeStore) {
    let pool = crate::test_db().await;
    set_apple_sync_enabled(&pool, true).await.unwrap();
    (pool, FakeStore::default())
}

async fn sync_at(pool: &Pool<Sqlite>, store: &FakeStore, now: &str) -> PassReport {
    run_pass(pool, store, at(now), PassMode::Normal).await
}

async fn sync(pool: &Pool<Sqlite>, store: &FakeStore) -> PassReport {
    sync_at(pool, store, NOW).await
}

async fn sync_ok(pool: &Pool<Sqlite>, store: &FakeStore) -> PassReport {
    let report = sync(pool, store).await;
    assert_eq!(report.outcome, Outcome::Synced, "{report:?}");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    report
}

fn list(store: &FakeStore) -> String {
    store.list_id(list_name()).unwrap()
}

fn input(title: &str) -> TaskInput {
    TaskInput {
        title: title.to_string(),
        ..Default::default()
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

async fn task_with(pool: &Pool<Sqlite>, i: TaskInput, reminders: &[TaskReminderInput]) -> Task {
    inner_create_task(&i, reminders, at(CREATED), pool)
        .await
        .unwrap()
}

async fn task(pool: &Pool<Sqlite>, title: &str) -> Task {
    task_with(pool, input(title), &[]).await
}

async fn get(pool: &Pool<Sqlite>, id: i64) -> Task {
    inner_get_task(id, pool).await.unwrap()
}

async fn edit(pool: &Pool<Sqlite>, id: i64, f: impl FnOnce(&mut TaskInput)) -> Task {
    let t = get(pool, id).await;
    let mut i = TaskInput {
        title: t.title,
        notes: t.notes,
        due_date: t.due_date,
        due_time: t.due_time,
        repeat: t.repeat,
        weekdays: t.weekdays.iter().map(|d| i64::from(*d)).collect(),
        month_day: t.month_day,
        issue_id: t.issue_id,
        note_id: t.note_id,
        mac_only: None,
    };
    f(&mut i);
    inner_update_task(id, &i, at(NOW), pool).await.unwrap()
}

async fn complete(pool: &Pool<Sqlite>, id: i64, when: &str) {
    inner_complete_task(id, at(when), pool).await.unwrap();
}

async fn pairing_of(pool: &Pool<Sqlite>, id: i64) -> Option<Pairing> {
    load_pairings(pool)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.badger_id == id)
}

async fn ek_of(pool: &Pool<Sqlite>, id: i64) -> String {
    pairing_of(pool, id).await.unwrap().ek_id
}

async fn pairing_count(pool: &Pool<Sqlite>) -> usize {
    load_pairings(pool).await.unwrap().len()
}

async fn task_count(pool: &Pool<Sqlite>) -> usize {
    inner_list_tasks(None, pool).await.unwrap().len()
}

async fn find(pool: &Pool<Sqlite>, title: &str) -> Task {
    inner_list_tasks(None, pool)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.title == title)
        .unwrap_or_else(|| panic!("no task {title}"))
}

fn with_due(title: &str, due: PhoneDue) -> PhoneItem {
    let mut p = phone(title);
    p.due = Some(due);
    p
}

/// (offset, at, state) of a task's reminders, in its order.
fn reminders(t: &Task) -> Vec<(Option<i64>, Option<String>, String)> {
    t.reminders
        .iter()
        .map(|r| (r.offset_minutes, r.at.clone(), r.state.clone()))
        .collect()
}

fn ts(s: &str) -> Option<String> {
    Some(format_timestamp(at(s)))
}

// ── First push ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_first_pass_pushes_open_one_off_tasks_with_one_alarm_per_reminder() {
    let (pool, store) = setup().await;
    let plain = task(&pool, "Plain").await;
    let dated = task_with(
        &pool,
        TaskInput {
            due_date: Some("2026-11-20".into()),
            ..input("Dated")
        },
        &[offset(0), custom_at("2026-11-19T18:00")],
    )
    .await;
    let timed = task_with(
        &pool,
        TaskInput {
            due_date: Some("2026-11-20".into()),
            due_time: Some("14:00".into()),
            ..input("Timed")
        },
        &[offset(0), offset(60)],
    )
    .await;
    let done = task(&pool, "Done").await;
    complete(&pool, done.id, "2026-11-18T09:00").await;
    task_with(
        &pool,
        TaskInput {
            repeat: "daily".into(),
            due_date: Some("2026-11-20".into()),
            ..input("Repeating")
        },
        &[],
    )
    .await;
    task_with(
        &pool,
        TaskInput {
            mac_only: Some(true),
            ..input("Mac only")
        },
        &[],
    )
    .await;

    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 3);
    assert_eq!(store.list_count(), 1, "one list");
    let titles: Vec<String> = store
        .items(&list(&store))
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(titles, vec!["Plain", "Dated", "Timed"]);

    let item = store.item(&ek_of(&pool, plain.id).await).unwrap();
    assert_eq!(
        (item.due, item.alarms.len(), item.completed),
        (None, 0, false)
    );
    let item = store.item(&ek_of(&pool, dated.id).await).unwrap();
    assert_eq!(item.due, Some(PhoneDue::Date(date("2026-11-20"))));
    assert_eq!(
        item.alarms,
        vec![at("2026-11-19T18:00"), at("2026-11-20T09:00")],
        "a date-only due's At due time is 09:00"
    );
    let item = store.item(&ek_of(&pool, timed.id).await).unwrap();
    assert_eq!(item.due, Some(PhoneDue::DateTime(at("2026-11-20T14:00"))));
    assert_eq!(
        item.alarms,
        vec![at("2026-11-20T13:00"), at("2026-11-20T14:00")]
    );
    assert_eq!(
        pairing_of(&pool, timed.id).await.unwrap().phone_alarms,
        vec![
            format_timestamp(at("2026-11-20T13:00")),
            format_timestamp(at("2026-11-20T14:00"))
        ]
    );
    assert_eq!(pairing_count(&pool).await, 3);

    // Nothing changed: the second pass writes nothing anywhere.
    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 0);
    assert!(!report.tasks_changed);
    assert_eq!(store.writes(), 3);
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_sync_at.is_some());
    assert_eq!((s.last_error, s.paused), (None, None));
}

#[tokio::test]
async fn disabled_sync_does_nothing() {
    let pool = crate::test_db().await;
    let store = FakeStore::default();
    task(&pool, "T").await;
    let report = sync(&pool, &store).await;
    assert_eq!(report.outcome, Outcome::Disabled);
    assert_eq!(store.list_count(), 0);
    assert_eq!(
        get_apple_sync_settings(&pool).await.unwrap().last_sync_at,
        None
    );
}

// ── Imports (D22) ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn phone_items_import_with_the_documented_rules() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let l = list(&store);
    store.add(
        &l,
        with_due("Date only", PhoneDue::Date(date("2026-11-25"))),
    );
    let mut timed = with_due("Timed", PhoneDue::DateTime(at("2026-11-25T15:00:30")));
    timed.alarms = vec![
        at("2026-11-25T15:00"),
        at("2026-11-25T14:00"),
        at("2026-11-19T08:00"),
    ];
    store.add(&l, timed);
    let mut undated = phone("No due");
    undated.notes = Some(" from the phone ".into());
    undated.alarms = vec![at("2026-11-21T10:00")];
    store.add(&l, undated);
    let mut many = phone("Many alarms");
    many.alarms = (1..=12)
        .rev()
        .map(|d| at(&format!("2026-12-{d:02}T09:00")))
        .collect();
    let many_ek = store.add(&l, many);
    let mut weekly = with_due("Weekly", PhoneDue::Date(date("2026-11-23")));
    weekly.recurring = true;
    let weekly_ek = store.add(&l, weekly);
    store.add(&l, phone("   "));
    let mut finished = phone("Finished");
    finished.completed = true;
    store.add(&l, finished);

    let report = sync_ok(&pool, &store).await;
    assert!(report.tasks_changed);
    assert_eq!(report.skipped.len(), 1);
    assert_eq!(report.skipped[0].reason, "needs a title");
    assert_eq!(
        task_count(&pool).await,
        5,
        "completed items aren't imported"
    );

    let t = find(&pool, "Date only").await;
    assert_eq!(
        (t.due_date.as_deref(), t.due_time.as_deref()),
        (Some("2026-11-25"), None)
    );
    assert!(t.reminders.is_empty());
    let t = find(&pool, "Timed").await;
    assert_eq!(t.due_time.as_deref(), Some("15:00"));
    assert_eq!(
        reminders(&t),
        vec![
            (None, ts("2026-11-19T08:00"), "dismissed".into()),
            (None, ts("2026-11-25T14:00"), "pending".into()),
            (Some(0), None, "pending".into()),
        ],
        "the alarm at the due time is At due time; a past one is dismissed"
    );
    let t = find(&pool, "No due").await;
    assert_eq!(t.notes.as_deref(), Some("from the phone"));
    assert_eq!(
        reminders(&t),
        vec![(None, ts("2026-11-21T10:00"), "pending".into())]
    );
    let t = find(&pool, "Many alarms").await;
    assert_eq!(t.reminders.len(), 10);
    assert_eq!(
        t.reminders[0].at,
        ts("2026-12-01T09:00"),
        "the earliest ten"
    );
    assert_eq!(t.reminders[9].at, ts("2026-12-10T09:00"));
    assert_eq!(
        store.item(&many_ek).unwrap().alarms.len(),
        10,
        "the rest are overwritten"
    );
    let t = find(&pool, "Weekly").await;
    assert_eq!(t.repeat, "none", "imports as a one-off");
    assert!(
        !store.item(&weekly_ek).unwrap().recurring,
        "its rule is stripped"
    );

    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 0);
    assert!(!report.tasks_changed);
    assert_eq!(task_count(&pool).await, 5, "no duplicates");
}

#[tokio::test]
async fn long_phone_titles_are_cut_to_500_chars_and_unicode_round_trips() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let ek = store.add(&list(&store), phone(&format!(" {} ", "å📞".repeat(300))));
    task(&pool, "Ring 📞 Åsa").await;
    sync_ok(&pool, &store).await;
    let imported = inner_list_tasks(None, &pool)
        .await
        .unwrap()
        .into_iter()
        .find(|t| t.title.starts_with('å'))
        .unwrap();
    assert_eq!(imported.title.chars().count(), 500);
    assert_eq!(
        store.item(&ek).unwrap().title,
        imported.title,
        "the cut title is pushed"
    );
    assert!(store
        .items(&list(&store))
        .iter()
        .any(|i| i.title == "Ring 📞 Åsa"));
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

// ── Edits ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn edits_flow_both_ways_and_badger_wins_a_conflict() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;

    // Phone → Badger: title, notes and a dated due.
    store.edit(&ek, |p| {
        p.title = "From phone".into();
        p.notes = Some("phone notes".into());
        p.due = Some(PhoneDue::DateTime(at("2026-11-21T08:15")));
    });
    let report = sync_ok(&pool, &store).await;
    assert!(report.tasks_changed);
    let now = get(&pool, t.id).await;
    assert_eq!(now.title, "From phone");
    assert_eq!(now.notes.as_deref(), Some("phone notes"));
    assert_eq!(
        (now.due_date.as_deref(), now.due_time.as_deref()),
        (Some("2026-11-21"), Some("08:15"))
    );

    // Badger → phone.
    edit(&pool, t.id, |i| {
        i.title = "From Badger".into();
        i.notes = None;
        i.due_time = None;
    })
    .await;
    sync_ok(&pool, &store).await;
    let item = store.item(&ek).unwrap();
    assert_eq!(item.title, "From Badger");
    assert_eq!(item.notes, None);
    assert_eq!(item.due, Some(PhoneDue::Date(date("2026-11-21"))));

    // Both edit the title: Badger wins; the phone's notes edit still applies.
    edit(&pool, t.id, |i| i.title = "Badger wins".into()).await;
    store.edit(&ek, |p| {
        p.title = "Phone loses".into();
        p.notes = Some("kept".into());
    });
    sync_ok(&pool, &store).await;
    let now = get(&pool, t.id).await;
    assert_eq!(
        (now.title.as_str(), now.notes.as_deref()),
        ("Badger wins", Some("kept"))
    );
    assert_eq!(store.item(&ek).unwrap().title, "Badger wins");

    // The phone clears the due: the task gets no due.
    store.edit(&ek, |p| p.due = None);
    sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.due_date, None);
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn a_due_change_on_either_side_recomputes_the_alarms() {
    let (pool, store) = setup().await;
    let t = task_with(
        &pool,
        TaskInput {
            due_date: Some("2026-11-20".into()),
            due_time: Some("14:00".into()),
            ..input("T")
        },
        &[offset(60)],
    )
    .await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    assert_eq!(
        store.item(&ek).unwrap().alarms,
        vec![at("2026-11-20T13:00")]
    );

    // In Badger.
    edit(&pool, t.id, |i| {
        i.due_date = Some("2026-11-21".into());
        i.due_time = Some("10:00".into());
    })
    .await;
    sync_ok(&pool, &store).await;
    let item = store.item(&ek).unwrap();
    assert_eq!(item.due, Some(PhoneDue::DateTime(at("2026-11-21T10:00"))));
    assert_eq!(item.alarms, vec![at("2026-11-21T09:00")]);

    // On the phone, to a date-only due: the reminder follows it (09:00 − 1 h).
    store.edit(&ek, |p| p.due = Some(PhoneDue::Date(date("2026-11-22"))));
    let report = sync_ok(&pool, &store).await;
    assert!(report.tasks_changed);
    let now = get(&pool, t.id).await;
    assert_eq!(
        (now.due_date.as_deref(), now.due_time.as_deref()),
        (Some("2026-11-22"), None)
    );
    assert_eq!(now.reminders[0].fire_at, ts("2026-11-22T08:00"));
    assert_eq!(now.reminders[0].state, "pending");
    assert_eq!(
        store.item(&ek).unwrap().alarms,
        vec![at("2026-11-22T08:00")]
    );

    // On the phone, into the past: dismissed silently, and a past alarm is never pushed.
    store.edit(&ek, |p| {
        p.due = Some(PhoneDue::DateTime(at("2026-11-18T10:00")))
    });
    sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.reminders[0].state, "dismissed");
    assert!(store.item(&ek).unwrap().alarms.is_empty());
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn phone_alarm_edits_are_overwritten_and_past_alarms_stay_put() {
    let (pool, store) = setup().await;
    let t = task_with(
        &pool,
        input("T"),
        &[custom_at("2026-11-19T12:30"), custom_at("2026-11-20T09:00")],
    )
    .await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    store.edit(&ek, |p| p.alarms = vec![at("2026-11-25T07:00")]);
    sync_ok(&pool, &store).await;
    assert_eq!(
        store.item(&ek).unwrap().alarms,
        vec![at("2026-11-19T12:30"), at("2026-11-20T09:00")],
        "Badger owns the alarms"
    );

    // 12:30 passes: its alarm stays on the phone (no churn, no cancelled alert).
    let writes = store.writes();
    let report = sync_at(&pool, &store, "2026-11-19T12:31").await;
    assert_eq!(report.phone_writes, 0);
    assert_eq!(store.writes(), writes);
    assert_eq!(store.item(&ek).unwrap().alarms.len(), 2);

    // A new reminder adds an alarm; deleting one in Badger removes it on the phone.
    let t = crate::commands::tasks::inner_add_task_reminder(
        t.id,
        &custom_at("2026-11-21T09:00"),
        at("2026-11-19T12:31"),
        &pool,
    )
    .await
    .unwrap();
    sync_at(&pool, &store, "2026-11-19T12:32").await;
    assert_eq!(store.item(&ek).unwrap().alarms.len(), 3);
    let last = t
        .reminders
        .iter()
        .find(|r| r.at == ts("2026-11-21T09:00"))
        .unwrap();
    crate::commands::tasks::inner_delete_task_reminder(last.id, &pool)
        .await
        .unwrap();
    sync_at(&pool, &store, "2026-11-19T12:33").await;
    assert_eq!(
        store.item(&ek).unwrap().alarms,
        vec![at("2026-11-19T12:30"), at("2026-11-20T09:00")]
    );
}

#[tokio::test]
async fn phone_edits_leave_links_and_reminder_states_alone() {
    let (pool, store) = setup().await;
    let note = crate::commands::notes::inner_create_note_titled("Plan", None, &pool)
        .await
        .unwrap();
    let t = task_with(
        &pool,
        TaskInput {
            note_id: Some(note.id),
            ..input("T")
        },
        &[custom_at("2026-11-19T11:00")],
    )
    .await;
    crate::commands::tasks::inner_fire_due(&pool, at(NOW))
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    store.edit(&ek_of(&pool, t.id).await, |p| p.title = "Renamed".into());
    sync_ok(&pool, &store).await;
    let now = get(&pool, t.id).await;
    assert_eq!(now.title, "Renamed");
    assert_eq!(now.note_id, Some(note.id));
    assert_eq!(now.reminders[0].state, "sent");
}

// ── Completion ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn completion_maps_both_ways() {
    let (pool, store) = setup().await;
    let t = task_with(&pool, input("T"), &[custom_at("2026-11-19T11:00")]).await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;

    complete(&pool, t.id, "2026-11-19T11:50").await;
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek).unwrap().completed);

    // Unchecked on the phone: reopened in Badger; the past reminder stays quiet (D8).
    store.edit(&ek, |p| p.completed = false);
    sync_ok(&pool, &store).await;
    let now = get(&pool, t.id).await;
    assert!(!now.done);
    assert_eq!(now.reminders[0].state, "dismissed");

    // Checked on the phone: done in Badger.
    store.edit(&ek, |p| p.completed = true);
    let report = sync_ok(&pool, &store).await;
    assert!(report.tasks_changed);
    assert!(get(&pool, t.id).await.done);
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn a_phone_completion_wins_over_a_badger_snooze_and_stops_the_reminders() {
    let (pool, store) = setup().await;
    let t = task_with(&pool, input("T"), &[custom_at("2026-11-19T11:00")]).await;
    crate::commands::tasks::inner_fire_due(&pool, at("2026-11-19T11:00:10"))
        .await
        .unwrap();
    inner_snooze_task_reminder(
        t.reminders[0].id,
        "2026-11-19T13:00",
        at("2026-11-19T11:01"),
        &pool,
    )
    .await
    .unwrap();
    sync_ok(&pool, &store).await;
    store.edit(&ek_of(&pool, t.id).await, |p| p.completed = true);
    sync_ok(&pool, &store).await;
    assert!(get(&pool, t.id).await.done);
    let sent = crate::commands::tasks::inner_fire_due(&pool, at("2026-11-19T13:00:10"))
        .await
        .unwrap()
        .fired;
    assert!(
        sent.is_empty(),
        "done wins: the snoozed reminder never fires"
    );
}

#[tokio::test]
async fn a_phone_completion_is_not_held_back_by_the_undo_window() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    store.edit(&ek, |p| p.completed = true);
    sync_ok(&pool, &store).await;
    assert!(pairing_of(&pool, t.id).await.unwrap().badger_snap.done);
    // The next pass, still inside the window, handles the item normally.
    store.edit(&ek, |p| p.title = "Renamed".into());
    sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.title, "Renamed");
}

#[tokio::test]
async fn a_completion_is_not_pushed_before_the_undo_window_closes() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;

    // Completed at 12:00:00; passes during the 5 s window leave the phone alone.
    let done = inner_complete_task(t.id, at(NOW), &pool).await.unwrap();
    let report = sync_at(&pool, &store, NOW).await;
    assert_eq!(report.phone_writes, 0);
    sync_at(&pool, &store, "2026-11-19T12:00:04").await;
    assert!(!store.item(&ek).unwrap().completed);

    // Undone inside the window: the phone never saw the completion.
    inner_undo_complete_task(&done.undo.unwrap(), &pool)
        .await
        .unwrap();
    let report = sync_at(&pool, &store, "2026-11-19T12:00:10").await;
    assert_eq!(report.phone_writes, 0);
    assert!(!store.item(&ek).unwrap().completed);
    assert!(!report.tasks_changed);

    // Not undone: pushed once the window has passed.
    complete(&pool, t.id, "2026-11-19T12:01:00").await;
    sync_at(&pool, &store, "2026-11-19T12:01:03").await;
    assert!(!store.item(&ek).unwrap().completed);
    sync_at(&pool, &store, "2026-11-19T12:01:06").await;
    assert!(store.item(&ek).unwrap().completed);
}

// ── Deletes ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn deletes_flow_both_ways() {
    let (pool, store) = setup().await;
    let a = task_with(&pool, input("A"), &[custom_at("2026-11-20T09:00")]).await;
    let b = task_with(&pool, input("B"), &[custom_at("2026-11-20T09:00")]).await;
    task(&pool, "C").await;
    sync_ok(&pool, &store).await;
    let ek_a = ek_of(&pool, a.id).await;
    let ek_b = ek_of(&pool, b.id).await;

    // Deleted in Badger: its phone item goes on the next pass.
    inner_delete_task(a.id, &pool).await.unwrap();
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek_a).is_none());
    assert!(pairing_of(&pool, a.id).await.is_none());

    // Deleted on the phone: the task (and its reminders) go after two missed passes.
    store.remove(&ek_b);
    sync_ok(&pool, &store).await;
    assert!(inner_get_task(b.id, &pool).await.is_ok());
    let report = sync_ok(&pool, &store).await;
    assert!(report.tasks_changed);
    assert!(inner_get_task(b.id, &pool).await.is_err());
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_reminders WHERE TaskId = ?")
        .bind(b.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(pairing_count(&pool).await, 1);
}

#[tokio::test]
async fn phone_delete_of_a_completed_item_only_unpairs() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    store.edit(&ek, |p| p.completed = true);
    sync_ok(&pool, &store).await;
    store.remove(&ek);
    sync_ok(&pool, &store).await;
    assert!(get(&pool, t.id).await.done, "kept, done");
    assert!(pairing_of(&pool, t.id).await.is_none());
}

#[tokio::test]
async fn a_phone_delete_applies_only_when_missing_two_passes_in_a_row() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    let item = store.item(&ek).unwrap();

    store.remove(&ek);
    sync_ok(&pool, &store).await;
    assert_eq!(pairing_of(&pool, t.id).await.unwrap().missed, 1);
    store.add(&list(&store), item);
    let report = sync_ok(&pool, &store).await;
    assert!(!report.tasks_changed);
    assert_eq!(pairing_of(&pool, t.id).await.unwrap().missed, 0);
    assert_eq!(store.items(&list(&store)).len(), 2, "no duplicate pushed");

    store.remove(&ek);
    sync_ok(&pool, &store).await;
    assert!(inner_get_task(t.id, &pool).await.is_ok());
    sync_ok(&pool, &store).await;
    assert!(inner_get_task(t.id, &pool).await.is_err());
}

#[tokio::test]
async fn a_badger_edit_wins_over_a_phone_delete() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "Other").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    store.remove(&ek);
    sync_ok(&pool, &store).await;
    edit(&pool, t.id, |i| i.title = "T edited".into()).await;
    sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.title, "T edited");
    let p = pairing_of(&pool, t.id).await.unwrap();
    assert_ne!(p.ek_id, ek, "pushed again");
    assert_eq!(p.missed, 0);
    assert_eq!(store.item(&p.ek_id).unwrap().title, "T edited");
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

// ── Guards ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn most_of_the_list_missing_at_once_pauses_until_delete_anyway() {
    let (pool, store) = setup().await;
    let mut tasks = Vec::new();
    for title in ["A", "B", "C", "D", "E"] {
        tasks.push(task(&pool, title).await);
    }
    sync_ok(&pool, &store).await;
    let mut eks = Vec::new();
    for t in &tasks[..3] {
        eks.push(ek_of(&pool, t.id).await);
    }
    store.remove(&eks[0]);
    store.remove(&eks[1]);
    sync_ok(&pool, &store).await;

    store.remove(&eks[2]);
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_EMPTY_FETCH,
            ..
        }
    ));
    assert_eq!(report.phone_writes, 0);
    assert_eq!(task_count(&pool).await, 5);
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_error.unwrap().contains("3 of the 5"));

    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert_eq!(task_count(&pool).await, 2);
    assert_eq!(pairing_count(&pool).await, 2);
    sync_ok(&pool, &store).await;
}

#[tokio::test]
async fn the_missing_guard_counts_only_tasks_a_miss_would_delete() {
    let (pool, store) = setup().await;
    let mut t = Vec::new();
    for title in ["A", "B", "C", "D", "E", "F"] {
        t.push(task(&pool, title).await);
    }
    sync_ok(&pool, &store).await;
    let mut eks = Vec::new();
    for x in &t {
        eks.push(ek_of(&pool, x.id).await);
    }
    complete(&pool, t[0].id, "2026-11-19T11:00").await;
    complete(&pool, t[1].id, "2026-11-19T11:00").await;
    inner_delete_task(t[2].id, &pool).await.unwrap();
    for ek in &eks[..4] {
        store.remove(ek);
    }
    sync_ok(&pool, &store).await;
    assert!(get(&pool, t[0].id).await.done);
    assert!(pairing_of(&pool, t[0].id).await.is_none());
    assert_eq!(pairing_of(&pool, t[3].id).await.unwrap().missed, 1);
}

#[tokio::test]
async fn guard_pauses_without_deleting_when_the_list_is_missing_or_access_is_lost() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    sync_ok(&pool, &store).await;

    store.set_access(AccessState::Denied);
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_ACCESS,
            ..
        }
    ));
    store.set_access(AccessState::Granted);
    sync_ok(&pool, &store).await;
    assert_eq!(get_apple_sync_settings(&pool).await.unwrap().paused, None);

    store.remove_list(&list(&store));
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_LIST_MISSING,
            ..
        }
    ));
    assert_eq!(report.phone_writes, 0);
    assert!(inner_get_task(t.id, &pool).await.is_ok());
    assert_eq!(pairing_count(&pool).await, 1);
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_error.unwrap().contains("missing"));
}

#[tokio::test]
async fn guard_pauses_on_an_empty_list_until_delete_anyway() {
    let (pool, store) = setup().await;
    let a = task(&pool, "A").await;
    let b = task(&pool, "B").await;
    sync_ok(&pool, &store).await;
    let fresh = task(&pool, "Not yet pushed").await;
    store.clear_list(&list(&store));
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_EMPTY_FETCH,
            ..
        }
    ));
    assert_eq!(report.phone_writes, 0, "the pass writes nothing at all");
    assert!(pairing_of(&pool, fresh.id).await.is_none());

    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(inner_get_task(a.id, &pool).await.is_err());
    assert!(inner_get_task(b.id, &pool).await.is_err());
    assert!(inner_get_task(fresh.id, &pool).await.is_ok());
    assert_eq!(get_apple_sync_settings(&pool).await.unwrap().paused, None);
}

#[tokio::test]
async fn deleting_the_only_open_item_on_the_phone_does_not_pause() {
    let (pool, store) = setup().await;
    let t = task(&pool, "Only one").await;
    sync_ok(&pool, &store).await;
    store.clear_list(&list(&store));
    assert_eq!(sync(&pool, &store).await.outcome, Outcome::Synced);
    assert!(inner_get_task(t.id, &pool).await.is_ok());
    assert_eq!(sync(&pool, &store).await.outcome, Outcome::Synced);
    assert!(inner_get_task(t.id, &pool).await.is_err());
}

#[tokio::test]
async fn a_pairing_that_cant_be_saved_takes_its_new_phone_item_back() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let t = task(&pool, "T").await;
    sqlx::query(
        "CREATE TRIGGER no_pairing BEFORE INSERT ON apple_sync_items \
         BEGIN SELECT RAISE(ABORT, 'disk full'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let report = sync(&pool, &store).await;
    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert!(
        store.items(&list(&store)).is_empty(),
        "no unpaired orphan left"
    );

    sqlx::query("DROP TRIGGER no_pairing")
        .execute(&pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    sync_ok(&pool, &store).await;
    assert_eq!(store.items(&list(&store)).len(), 1);
    assert_eq!(
        task_count(&pool).await,
        1,
        "nothing imported back as a duplicate"
    );
    assert!(pairing_of(&pool, t.id).await.is_some());
}

#[tokio::test]
async fn a_failed_pass_is_retried_without_duplicates() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let t = task(&pool, "T").await;
    let mut item = with_due("Phone one", PhoneDue::DateTime(at("2026-11-20T09:00")));
    item.alarms = vec![at("2026-11-20T09:00"), at("2026-11-20T08:00")];
    let ek = store.add(&list(&store), item);
    store.fail_writes(true);
    let report = sync(&pool, &store).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(!report.errors.is_empty());
    assert!(pairing_of(&pool, t.id).await.is_none());
    assert!(get_apple_sync_settings(&pool)
        .await
        .unwrap()
        .last_error
        .is_some());
    assert_eq!(
        task_count(&pool).await,
        2,
        "the import itself committed and paired"
    );

    store.fail_writes(false);
    sync_ok(&pool, &store).await;
    assert_eq!(store.items(&list(&store)).len(), 2);
    assert_eq!(task_count(&pool).await, 2);
    let mut alarms = store.item(&ek).unwrap().alarms;
    alarms.sort();
    assert_eq!(alarms, vec![at("2026-11-20T08:00"), at("2026-11-20T09:00")]);
    let imported = find(&pool, "Phone one").await;
    assert_eq!(imported.reminders.len(), 2, "both alarms became reminders");
    assert_eq!(
        get_apple_sync_settings(&pool).await.unwrap().last_error,
        None
    );
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

// ── Off the phone: repeating and Mac-only ─────────────────────────────────────

#[tokio::test]
async fn becoming_repeating_removes_the_phone_item_and_a_one_off_pushes_it_again() {
    let (pool, store) = setup().await;
    let t = task_with(
        &pool,
        TaskInput {
            due_date: Some("2026-11-20".into()),
            ..input("Standup")
        },
        &[],
    )
    .await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    edit(&pool, t.id, |i| i.repeat = "weekdays".into()).await;
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek).is_none());
    assert!(pairing_of(&pool, t.id).await.is_none());
    assert_eq!(get(&pool, t.id).await.repeat, "weekdays", "the task stays");
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0, "never pushed");

    // Completing it advances it on the Mac only.
    complete(&pool, t.id, NOW).await;
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);

    edit(&pool, t.id, |i| i.repeat = "none".into()).await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    assert_eq!(
        store.item(&ek).unwrap().due,
        Some(PhoneDue::Date(date("2026-11-23")))
    );
}

#[tokio::test]
async fn mac_only_tasks_are_never_pushed_and_leave_or_return_when_toggled() {
    let (pool, store) = setup().await;
    let m = task_with(
        &pool,
        TaskInput {
            mac_only: Some(true),
            ..input("Private")
        },
        &[],
    )
    .await;
    let t = task(&pool, "Shared").await;
    sync_ok(&pool, &store).await;
    assert!(pairing_of(&pool, m.id).await.is_none());
    let ek = ek_of(&pool, t.id).await;

    // Kept off the phone after syncing: the phone item goes, the task stays.
    edit(&pool, t.id, |i| i.mac_only = Some(true)).await;
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek).is_none());
    assert!(inner_get_task(t.id, &pool).await.is_ok());
    assert!(store.items(&list(&store)).is_empty());

    // Back on the phone.
    edit(&pool, m.id, |i| i.mac_only = Some(false)).await;
    sync_ok(&pool, &store).await;
    let titles: Vec<String> = store
        .items(&list(&store))
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(titles, vec!["Private"]);
}

#[tokio::test]
async fn a_mac_only_task_is_never_deleted_when_its_phone_item_vanishes_or_counted_by_guards() {
    let (pool, store) = setup().await;
    let mut tasks = Vec::new();
    for title in ["A", "B", "C", "D"] {
        tasks.push(task(&pool, title).await);
    }
    sync_ok(&pool, &store).await;
    // Three made Mac-only, then the phone loses them before the pass runs.
    for t in &tasks[..3] {
        let ek = ek_of(&pool, t.id).await;
        edit(&pool, t.id, |i| i.mac_only = Some(true)).await;
        store.remove(&ek);
    }
    let report = sync_ok(&pool, &store).await;
    assert!(!report.tasks_changed);
    for t in &tasks[..3] {
        assert!(inner_get_task(t.id, &pool).await.is_ok());
        assert!(pairing_of(&pool, t.id).await.is_none());
    }
    sync_ok(&pool, &store).await;
    assert_eq!(task_count(&pool).await, 4);
}

// ── Re-pairing and restore ────────────────────────────────────────────────────

#[tokio::test]
async fn a_changed_eventkit_id_re_pairs_by_external_id() {
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    store.change_ek_id(&ek, "ek-renumbered");
    store.edit("ek-renumbered", |p| p.title = "T on phone".to_string());
    sync_ok(&pool, &store).await;
    assert_eq!(ek_of(&pool, t.id).await, "ek-renumbered");
    assert_eq!(get(&pool, t.id).await.title, "T on phone");
    assert_eq!(
        store.items(&list(&store)).len(),
        1,
        "no duplicate on the phone"
    );
    assert_eq!(task_count(&pool).await, 1, "no duplicate in Badger");
}

#[tokio::test]
async fn after_a_restore_badger_is_authoritative_for_one_pass() {
    use crate::app_settings::set_apple_sync_restore_pending;
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    let r = task(&pool, "R").await;
    task(&pool, "Another").await;
    let gone = task(&pool, "Not in the backup").await;
    sync_ok(&pool, &store).await;
    let ek_t = ek_of(&pool, t.id).await;
    let ek_r = ek_of(&pool, r.id).await;
    let ek_gone = ek_of(&pool, gone.id).await;

    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(gone.id)
        .execute(&pool)
        .await
        .unwrap();
    let mut conn = pool.acquire().await.unwrap();
    set_apple_sync_restore_pending(&mut conn, true)
        .await
        .unwrap();
    drop(conn);
    store.edit(&ek_t, |p| p.title = "edited on phone".to_string());
    store.remove(&ek_r);

    sync_ok(&pool, &store).await;
    assert_eq!(
        store.item(&ek_t).unwrap().title,
        "T",
        "overwritten from Badger"
    );
    assert_eq!(get(&pool, t.id).await.title, "T");
    let new_ek = ek_of(&pool, r.id).await;
    assert_ne!(new_ek, ek_r);
    assert_eq!(
        store.item(&new_ek).unwrap().title,
        "R",
        "re-created on the phone"
    );
    assert!(
        store.item(&ek_gone).is_some(),
        "kept and imported, not deleted"
    );
    assert!(!find(&pool, "Not in the backup").await.done);
    assert!(
        !get_apple_sync_settings(&pool)
            .await
            .unwrap()
            .restore_pending
    );

    store.edit(&ek_t, |p| p.title = "edited again".to_string());
    sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.title, "edited again");
}

// ── Phone notifications ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_synced_reminder_is_left_to_the_phone_unless_snoozed_or_off_the_phone() {
    use crate::apple_sync::phone_alarms_cover;
    use crate::commands::tasks::inner_fire_due;
    let (pool, store) = setup().await;
    let synced = task_with(&pool, input("Synced"), &[custom_at("2026-11-19T12:30")]).await;
    let snoozed = task_with(&pool, input("Snoozed"), &[custom_at("2026-11-19T12:10")]).await;
    sync_ok(&pool, &store).await;
    let late = task_with(
        &pool,
        input("Not pushed yet"),
        &[custom_at("2026-11-19T12:30")],
    )
    .await;
    inner_fire_due(&pool, at("2026-11-19T12:10:10"))
        .await
        .unwrap();
    inner_snooze_task_reminder(
        snoozed.reminders[0].id,
        "2026-11-19T12:30",
        at("2026-11-19T12:11"),
        &pool,
    )
    .await
    .unwrap();

    let fired = inner_fire_due(&pool, at("2026-11-19T12:30:10"))
        .await
        .unwrap()
        .fired;
    assert_eq!(fired.len(), 3);
    let skip = phone_alarms_cover(&pool, &fired).await;
    assert!(
        skip.contains(&synced.reminders[0].id),
        "the phone alarms for it"
    );
    assert!(
        !skip.contains(&snoozed.reminders[0].id),
        "a snooze is Badger-only"
    );
    assert!(
        !skip.contains(&late.reminders[0].id),
        "not on the phone yet"
    );

    // Once Mac-only, the Mac notifies even before the pass drops the pairing.
    edit(&pool, synced.id, |i| i.mac_only = Some(true)).await;
    assert!(phone_alarms_cover(&pool, &fired).await.is_empty());
    let _ = store;
}

#[tokio::test]
async fn a_real_restore_keeps_pairings_and_makes_one_pass_authoritative() {
    use crate::commands::backup::{inner_backup_data, inner_restore_data};
    let (pool, store) = setup().await;
    let t = task(&pool, "T").await;
    task(&pool, "U").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, t.id).await;
    let backup = inner_backup_data(&pool).await.unwrap();

    store.edit(&ek, |p| p.title = "phone".to_string());
    edit(&pool, t.id, |i| i.title = "after backup".into()).await;
    inner_restore_data(&backup, &pool).await.unwrap();
    assert_eq!(pairing_count(&pool).await, 2, "pairings survive restore");

    let report = sync_ok(&pool, &store).await;
    assert_eq!(get(&pool, t.id).await.title, "T");
    assert_eq!(
        store.item(&ek).unwrap().title,
        "T",
        "the restored data wins"
    );
    assert!(!report.tasks_changed, "nothing changed in Badger");
    assert_eq!(task_count(&pool).await, 2, "no duplicates");
    assert!(
        !get_apple_sync_settings(&pool)
            .await
            .unwrap()
            .restore_pending
    );
}
