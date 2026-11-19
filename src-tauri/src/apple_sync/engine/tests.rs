use super::*;
use crate::app_settings::set_apple_sync_enabled;
use crate::apple_sync::fake::{phone, FakeStore};
use crate::commands::notes::{inner_create_note_titled, inner_delete_note, inner_rename_note};
use crate::commands::reminders::{
    inner_complete_reminder, inner_create_reminder, inner_delete_reminder, inner_fire_due,
    inner_get_reminder, inner_update_reminder, Reminder, ReminderInput,
};
use crate::commands::todos::{
    inner_create_todo, inner_delete_todo, inner_get_todo, inner_list_todos, inner_move_todo,
    inner_update_todo, TodoRow,
};

const NOW: &str = "2026-11-19T12:00";
/// When test rows are created (before NOW, so NOW can fire them).
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

async fn sync(pool: &Pool<Sqlite>, store: &FakeStore) -> PassReport {
    run_pass(pool, store, at(NOW), PassMode::Normal).await
}

async fn sync_ok(pool: &Pool<Sqlite>, store: &FakeStore) -> PassReport {
    let report = sync(pool, store).await;
    assert_eq!(report.outcome, Outcome::Synced, "{report:?}");
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    report
}

fn todos_list(store: &FakeStore) -> String {
    store.list_id(ListKind::Todos.list_name()).unwrap()
}

fn reminders_list(store: &FakeStore) -> String {
    store.list_id(ListKind::Reminders.list_name()).unwrap()
}

async fn reminder_in(pool: &Pool<Sqlite>, text: &str, when: &str, repeat: &str) -> Reminder {
    let input = ReminderInput {
        text: text.to_string(),
        remind_at: when.to_string(),
        repeat: repeat.to_string(),
        weekdays: vec![],
        month_day: None,
        todo_id: None,
        note_id: None,
    };
    inner_create_reminder(&input, at(CREATED), pool)
        .await
        .unwrap()
}

async fn reminder(pool: &Pool<Sqlite>, text: &str, when: &str) -> Reminder {
    reminder_in(pool, text, when, "none").await
}

async fn linked_reminder(
    pool: &Pool<Sqlite>,
    text: &str,
    when: &str,
    todo_id: Option<i64>,
    note_id: Option<i64>,
) -> Reminder {
    let input = ReminderInput {
        text: text.to_string(),
        remind_at: when.to_string(),
        repeat: "none".to_string(),
        todo_id,
        note_id,
        ..Default::default()
    };
    inner_create_reminder(&input, at(CREATED), pool)
        .await
        .unwrap()
}

async fn todo(pool: &Pool<Sqlite>, title: &str, status: &str) -> TodoRow {
    inner_create_todo(title, None, None, status, None, pool)
        .await
        .unwrap()
}

async fn pairing_of(pool: &Pool<Sqlite>, kind: ListKind, id: i64) -> Option<Pairing> {
    load_pairings(pool)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.kind == kind && p.badger_id == id)
}

async fn ek_of(pool: &Pool<Sqlite>, kind: ListKind, id: i64) -> String {
    pairing_of(pool, kind, id).await.unwrap().ek_id
}

async fn pairing_count(pool: &Pool<Sqlite>) -> usize {
    load_pairings(pool).await.unwrap().len()
}

async fn reminder_count(pool: &Pool<Sqlite>) -> i64 {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reminders")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

async fn todo_count(pool: &Pool<Sqlite>) -> usize {
    inner_list_todos(pool).await.unwrap().len()
}

fn with_time(title: &str, due: &str) -> PhoneItem {
    let mut p = phone(title);
    p.due = Some(PhoneDue::DateTime(at(due)));
    p
}

// ── AC1: first push ───────────────────────────────────────────────────────────

#[tokio::test]
async fn enabling_pushes_open_one_offs_and_open_todos_and_nothing_else() {
    let (pool, store) = setup().await;
    let fired = reminder(&pool, "Fired one", "2026-11-19T09:00").await;
    let pending = reminder(&pool, "Pending one", "2026-11-20T09:30").await;
    reminder_in(&pool, "Daily", "2026-11-20T08:00", "daily").await;
    let done = reminder(&pool, "Done one", "2026-11-21T09:00").await;
    inner_complete_reminder(done.id, at(CREATED), &pool)
        .await
        .unwrap();
    inner_fire_due(&pool, at(NOW)).await.unwrap();
    assert_eq!(
        inner_get_reminder(fired.id, &pool).await.unwrap().state,
        "fired"
    );

    let open = todo(&pool, "Open", "not_started").await;
    let busy = todo(&pool, "Busy", "in_progress").await;
    todo(&pool, "Finished", "done").await;
    let linked = linked_reminder(&pool, "Call", "2026-11-22T10:00", Some(open.id), None).await;

    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 5);
    assert_eq!(store.list_count(), 2, "both lists created");

    let mut titles: Vec<String> = store
        .items(&reminders_list(&store))
        .into_iter()
        .map(|i| i.title)
        .collect();
    titles.sort();
    assert_eq!(titles, vec!["Call", "Fired one", "Pending one"]);
    let todos: Vec<String> = store
        .items(&todos_list(&store))
        .into_iter()
        .map(|i| i.title)
        .collect();
    assert_eq!(todos, vec!["Open", "Busy"]);

    let item = store
        .item(&ek_of(&pool, ListKind::Reminders, pending.id).await)
        .unwrap();
    let t = at("2026-11-20T09:30");
    assert_eq!(item.due, Some(PhoneDue::DateTime(t)));
    assert_eq!(item.alarms, vec![t], "one absolute alarm at RemindAt");
    assert!(!item.completed);
    assert_eq!(item.notes, None, "unlinked: notes untouched");
    let call = store
        .item(&ek_of(&pool, ListKind::Reminders, linked.id).await)
        .unwrap();
    assert_eq!(call.notes.as_deref(), Some("Todo: Open"));
    assert!(pairing_of(&pool, ListKind::Todos, busy.id).await.is_some());
    assert_eq!(pairing_count(&pool).await, 5);

    // Nothing changed: the second pass writes nothing anywhere.
    let before = inner_list_todos(&pool).await.unwrap();
    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 0);
    assert!(!report.badger_changed());
    assert_eq!(store.writes(), 5);
    assert_eq!(inner_list_todos(&pool).await.unwrap().len(), before.len());

    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_sync_at.is_some());
    assert_eq!((s.last_error, s.paused), (None, None));
}

#[tokio::test]
async fn disabled_sync_does_nothing() {
    let (pool, store) = setup().await;
    set_apple_sync_enabled(&pool, false).await.unwrap();
    todo(&pool, "Open", "not_started").await;
    let report = sync(&pool, &store).await;
    assert_eq!(report.outcome, Outcome::Disabled);
    assert_eq!(store.list_count(), 0);
}

// ── AC2: imports ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn phone_created_items_import_with_the_documented_rules() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await; // creates the lists
    let rl = reminders_list(&store);
    let tl = todos_list(&store);

    let timed = store.add(&rl, with_time("Dentist 🦷", "2026-11-20T14:15"));
    let mut d = phone("Date only");
    d.due = Some(PhoneDue::Date(date("2026-11-21")));
    let date_only = store.add(&rl, d);
    let no_date = store.add(&rl, phone("Someday"));
    let past = store.add(&rl, with_time("Missed", "2026-11-19T08:00"));
    let mut r = with_time("Weekly", "2026-11-23T10:00");
    r.recurring = true;
    let repeating = store.add(&rl, r);
    store.add(&rl, with_time("   ", "2026-11-23T10:00"));

    let mut t = phone("Buy milk");
    t.notes = Some("  2 litres ".to_string());
    t.due = Some(PhoneDue::DateTime(at("2026-11-24T17:45")));
    let todo_item = store.add(&tl, t);
    store.add(&tl, phone(""));
    let existing = todo(&pool, "Existing", "not_started").await;

    let report = sync_ok(&pool, &store).await;
    assert!(report.badger_changed());

    let find = |ek: String| {
        let pool = pool.clone();
        async move {
            let p = load_pairings(&pool)
                .await
                .unwrap()
                .into_iter()
                .find(|p| p.ek_id == ek)
                .unwrap();
            inner_get_reminder(p.badger_id, &pool).await.unwrap()
        }
    };
    let r = find(timed.clone()).await;
    assert_eq!(
        (r.text.as_str(), r.state.as_str()),
        ("Dentist 🦷", "pending")
    );
    assert_eq!(r.remind_at, "2026-11-20T14:15:00.000");
    assert_eq!(r.repeat, "none");
    assert_eq!(
        store.item(&timed).unwrap().alarms,
        vec![at("2026-11-20T14:15")],
        "the import gets its alarm"
    );

    let r = find(date_only.clone()).await;
    assert_eq!(r.remind_at, "2026-11-21T09:00:00.000");
    let item = store.item(&date_only).unwrap();
    assert_eq!(item.due, Some(PhoneDue::DateTime(at("2026-11-21T09:00"))));
    assert_eq!(item.alarms, vec![at("2026-11-21T09:00")]);

    let r = find(past.clone()).await;
    assert_eq!(r.state, "fired", "a past time imports as overdue");

    let r = find(repeating.clone()).await;
    assert_eq!(r.repeat, "none");
    assert_eq!(r.remind_at, "2026-11-23T10:00:00.000");
    assert!(!store.item(&repeating).unwrap().recurring, "rule stripped");

    // Skipped: no date and no title, counted with reasons; the phone item is untouched.
    let mut reasons: Vec<(String, String)> = report
        .skipped
        .iter()
        .map(|s| (s.title.clone(), s.reason.clone()))
        .collect();
    reasons.sort();
    assert_eq!(
        reasons,
        vec![
            ("(no title)".to_string(), "needs a title".to_string()),
            ("(no title)".to_string(), "needs a title".to_string()),
            ("Someday".to_string(), "needs a date".to_string()),
        ]
    );
    assert_eq!(store.item(&no_date).unwrap(), {
        let mut p = phone("Someday");
        p.ek_id = no_date.clone();
        p.external_id = store.item(&no_date).unwrap().external_id;
        p
    });
    let stored = get_apple_sync_settings(&pool)
        .await
        .unwrap()
        .skipped
        .unwrap();
    assert!(stored.contains("needs a date"));

    // Todo: not_started at the end of the column, date part only, notes cleaned.
    let p = load_pairings(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.ek_id == todo_item)
        .unwrap();
    let t = inner_get_todo(p.badger_id, &pool).await.unwrap();
    assert_eq!(t.title, "Buy milk");
    assert_eq!(t.notes.as_deref(), Some("2 litres"));
    assert_eq!(t.due_date.as_deref(), Some("2026-11-24"));
    assert_eq!(t.status, "not_started");
    assert_eq!(t.position, existing.position + 1);
    assert_eq!(
        store.item(&todo_item).unwrap().due,
        Some(PhoneDue::DateTime(at("2026-11-24T17:45"))),
        "the phone keeps its time"
    );

    // Retried each pass, and still nothing duplicated.
    let (reminders, todos) = (reminder_count(&pool).await, todo_count(&pool).await);
    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.skipped.len(), 3);
    assert_eq!(report.phone_writes, 0);
    assert_eq!(reminder_count(&pool).await, reminders);
    assert_eq!(todo_count(&pool).await, todos);

    // Once it has a date, it imports.
    store.edit(&no_date, |p| {
        p.due = Some(PhoneDue::Date(date("2026-11-25")))
    });
    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.skipped.len(), 2);
    assert_eq!(reminder_count(&pool).await, reminders + 1);
}

#[tokio::test]
async fn long_phone_titles_are_cut_to_500_chars_and_unicode_round_trips() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "Ring Åsa 📞 — ünïcødé", "not_started").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    assert_eq!(store.item(&ek).unwrap().title, "Ring Åsa 📞 — ünïcødé");
    store.edit(&ek, |p| p.title = "Ring 🎉 Örjan".to_string());
    sync_ok(&pool, &store).await;
    assert_eq!(
        inner_get_todo(t.id, &pool).await.unwrap().title,
        "Ring 🎉 Örjan"
    );

    let rl = reminders_list(&store);
    let long = "ö".repeat(600);
    let ek = store.add(&rl, with_time(&long, "2026-11-20T10:00"));
    sync_ok(&pool, &store).await;
    let p = load_pairings(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.ek_id == ek)
        .unwrap();
    let r = inner_get_reminder(p.badger_id, &pool).await.unwrap();
    assert_eq!(r.text.chars().count(), 500);
    assert_eq!(store.item(&ek).unwrap().title.chars().count(), 500);
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

// ── AC3: edits both ways ──────────────────────────────────────────────────────

#[tokio::test]
async fn todo_edits_flow_both_ways_and_badger_wins_a_conflict() {
    let (pool, store) = setup().await;
    let a = todo(&pool, "A", "not_started").await;
    let b = todo(&pool, "B", "in_progress").await;
    sync_ok(&pool, &store).await;
    let ek_a = ek_of(&pool, ListKind::Todos, a.id).await;
    let ek_b = ek_of(&pool, ListKind::Todos, b.id).await;

    // Phone edits every field of A.
    store.edit(&ek_a, |p| {
        p.title = "A from phone".to_string();
        p.notes = Some("phone notes".to_string());
        p.due = Some(PhoneDue::Date(date("2026-11-30")));
    });
    // Badger edits every field of B.
    inner_update_todo(
        b.id,
        "B in Badger",
        Some("badger notes"),
        Some("2026-12-01"),
        None,
        &pool,
    )
    .await
    .unwrap();
    sync_ok(&pool, &store).await;

    let a_now = inner_get_todo(a.id, &pool).await.unwrap();
    assert_eq!(a_now.title, "A from phone");
    assert_eq!(a_now.notes.as_deref(), Some("phone notes"));
    assert_eq!(a_now.due_date.as_deref(), Some("2026-11-30"));
    let b_phone = store.item(&ek_b).unwrap();
    assert_eq!(b_phone.title, "B in Badger");
    assert_eq!(b_phone.notes.as_deref(), Some("badger notes"));
    assert_eq!(b_phone.due, Some(PhoneDue::Date(date("2026-12-01"))));
    assert_eq!(
        inner_get_todo(b.id, &pool).await.unwrap().status,
        "in_progress"
    );

    // Both sides change the title; the phone also changes notes: Badger wins the title,
    // the notes come from the phone.
    store.edit(&ek_a, |p| {
        p.title = "phone title".to_string();
        p.notes = Some("phone notes 2".to_string());
    });
    inner_update_todo(
        a.id,
        "badger title",
        Some("phone notes"),
        Some("2026-11-30"),
        None,
        &pool,
    )
    .await
    .unwrap();
    sync_ok(&pool, &store).await;
    let a_now = inner_get_todo(a.id, &pool).await.unwrap();
    assert_eq!(a_now.title, "badger title");
    assert_eq!(a_now.notes.as_deref(), Some("phone notes 2"));
    let a_phone = store.item(&ek_a).unwrap();
    assert_eq!(a_phone.title, "badger title");
    assert_eq!(a_phone.notes.as_deref(), Some("phone notes 2"));

    // Phone clears the due date.
    store.edit(&ek_a, |p| p.due = None);
    sync_ok(&pool, &store).await;
    assert_eq!(inner_get_todo(a.id, &pool).await.unwrap().due_date, None);

    // Quiet: nothing written on either side.
    let writes = store.writes();
    let report = sync_ok(&pool, &store).await;
    assert_eq!((report.phone_writes, report.badger_changed()), (0, false));
    assert_eq!(store.writes(), writes);
}

#[tokio::test]
async fn reminder_edits_flow_both_ways() {
    let (pool, store) = setup().await;
    let a = reminder(&pool, "A", "2026-11-20T09:00").await;
    let b = reminder(&pool, "B", "2026-11-20T10:00").await;
    sync_ok(&pool, &store).await;
    let ek_a = ek_of(&pool, ListKind::Reminders, a.id).await;
    let ek_b = ek_of(&pool, ListKind::Reminders, b.id).await;

    // Phone renames A and moves it (a reschedule in Badger).
    store.edit(&ek_a, |p| {
        p.title = "A on phone".to_string();
        p.due = Some(PhoneDue::DateTime(at("2026-11-21T16:30")));
    });
    // Badger renames B and moves it.
    let input = ReminderInput {
        text: "B in Badger".to_string(),
        remind_at: "2026-11-22T11:00".to_string(),
        repeat: "none".to_string(),
        ..Default::default()
    };
    inner_update_reminder(b.id, &input, at(NOW), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;

    let a_now = inner_get_reminder(a.id, &pool).await.unwrap();
    assert_eq!(a_now.text, "A on phone");
    assert_eq!(a_now.remind_at, "2026-11-21T16:30:00.000");
    assert_eq!(a_now.time_of_day, "16:30");
    assert_eq!(a_now.state, "pending");
    assert_eq!(
        store.item(&ek_a).unwrap().alarms,
        vec![at("2026-11-21T16:30")],
        "the alarm follows the new time"
    );
    let b_phone = store.item(&ek_b).unwrap();
    assert_eq!(b_phone.title, "B in Badger");
    assert_eq!(
        b_phone.due,
        Some(PhoneDue::DateTime(at("2026-11-22T11:00")))
    );
    assert_eq!(b_phone.alarms, vec![at("2026-11-22T11:00")]);

    // Both change the time: Badger wins.
    store.edit(&ek_a, |p| {
        p.due = Some(PhoneDue::DateTime(at("2026-11-25T08:00")))
    });
    let input = ReminderInput {
        text: "A on phone".to_string(),
        remind_at: "2026-11-24T08:00".to_string(),
        repeat: "none".to_string(),
        ..Default::default()
    };
    inner_update_reminder(a.id, &input, at(NOW), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    assert_eq!(
        inner_get_reminder(a.id, &pool).await.unwrap().remind_at,
        "2026-11-24T08:00:00.000"
    );
    assert_eq!(
        store.item(&ek_a).unwrap().due,
        Some(PhoneDue::DateTime(at("2026-11-24T08:00")))
    );

    // Phone removes the date: Badger can't hold that, so its time goes back.
    store.edit(&ek_a, |p| p.due = None);
    sync_ok(&pool, &store).await;
    assert_eq!(
        store.item(&ek_a).unwrap().due,
        Some(PhoneDue::DateTime(at("2026-11-24T08:00")))
    );

    // Phone edits a reminder's notes: not synced, left as they are.
    store.edit(&ek_b, |p| p.notes = Some("my notes".to_string()));
    let report = sync_ok(&pool, &store).await;
    assert_eq!(report.phone_writes, 0);
    assert_eq!(
        store.item(&ek_b).unwrap().notes.as_deref(),
        Some("my notes")
    );
}

#[tokio::test]
async fn linked_reminder_notes_follow_todo_and_note_titles() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "Plan", "not_started").await;
    let note = inner_create_note_titled("Weekly", None, &pool)
        .await
        .unwrap();
    let rt = linked_reminder(&pool, "Plan it", "2026-11-20T09:00", Some(t.id), None).await;
    let rn = linked_reminder(&pool, "Read it", "2026-11-20T10:00", None, Some(note.id)).await;
    sync_ok(&pool, &store).await;
    let ek_rt = ek_of(&pool, ListKind::Reminders, rt.id).await;
    let ek_rn = ek_of(&pool, ListKind::Reminders, rn.id).await;
    assert_eq!(
        store.item(&ek_rn).unwrap().notes.as_deref(),
        Some("Note: Weekly")
    );

    // A phone rename of the todo reaches the reminder's notes in the same pass.
    store.edit(&ek_of(&pool, ListKind::Todos, t.id).await, |p| {
        p.title = "Plan v2".to_string()
    });
    inner_rename_note(note.id, "Monthly", &pool).await.unwrap();
    sync_ok(&pool, &store).await;
    assert_eq!(
        store.item(&ek_rt).unwrap().notes.as_deref(),
        Some("Todo: Plan v2")
    );
    assert_eq!(
        store.item(&ek_rn).unwrap().notes.as_deref(),
        Some("Note: Monthly")
    );

    // Deleting the note cascades its reminder, so its phone item goes too.
    inner_delete_note(note.id, &pool).await.unwrap();
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek_rn).is_none());
}

#[tokio::test]
async fn unlinking_a_reminder_takes_its_link_line_off_the_phone() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "Plan", "not_started").await;
    let a = linked_reminder(&pool, "A", "2026-11-20T09:00", Some(t.id), None).await;
    let b = linked_reminder(&pool, "B", "2026-11-20T10:00", Some(t.id), None).await;
    sync_ok(&pool, &store).await;
    let ek_a = ek_of(&pool, ListKind::Reminders, a.id).await;
    let ek_b = ek_of(&pool, ListKind::Reminders, b.id).await;
    // B's notes were rewritten on the phone: from now on they're the user's.
    store.edit(&ek_b, |p| p.notes = Some("my own notes".to_string()));

    for (r, text) in [(&a, "A"), (&b, "B")] {
        let input = ReminderInput {
            text: text.to_string(),
            remind_at: r.remind_at.clone(),
            repeat: "none".to_string(),
            ..Default::default()
        };
        inner_update_reminder(r.id, &input, at(NOW), &pool)
            .await
            .unwrap();
    }
    sync_ok(&pool, &store).await;
    assert_eq!(store.item(&ek_a).unwrap().notes, None, "link line cleared");
    assert_eq!(
        store.item(&ek_b).unwrap().notes.as_deref(),
        Some("my own notes")
    );
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

// ── AC4: completion ───────────────────────────────────────────────────────────

#[tokio::test]
async fn completion_maps_both_ways() {
    let (pool, store) = setup().await;
    let r = reminder(&pool, "R", "2026-11-20T09:00").await;
    let t = todo(&pool, "T", "in_progress").await;
    let linked = linked_reminder(&pool, "For T", "2026-11-21T09:00", Some(t.id), None).await;
    let r2 = reminder(&pool, "R2", "2026-11-20T11:00").await;
    let t2 = todo(&pool, "T2", "not_started").await;
    sync_ok(&pool, &store).await;
    let ek_r = ek_of(&pool, ListKind::Reminders, r.id).await;
    let ek_t = ek_of(&pool, ListKind::Todos, t.id).await;
    let ek_linked = ek_of(&pool, ListKind::Reminders, linked.id).await;
    let ek_r2 = ek_of(&pool, ListKind::Reminders, r2.id).await;
    let ek_t2 = ek_of(&pool, ListKind::Todos, t2.id).await;

    // Phone completes: reminder done; todo done and its reminders with it (also on the phone).
    store.edit(&ek_r, |p| p.completed = true);
    store.edit(&ek_t, |p| p.completed = true);
    sync_ok(&pool, &store).await;
    assert_eq!(inner_get_reminder(r.id, &pool).await.unwrap().state, "done");
    assert_eq!(inner_get_todo(t.id, &pool).await.unwrap().status, "done");
    assert_eq!(
        inner_get_reminder(linked.id, &pool).await.unwrap().state,
        "done"
    );
    assert!(store.item(&ek_linked).unwrap().completed);

    // Badger completes: the phone items complete.
    inner_complete_reminder(r2.id, at(NOW), &pool)
        .await
        .unwrap();
    inner_move_todo(t2.id, "done", 0, &pool).await.unwrap();
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek_r2).unwrap().completed);
    assert!(store.item(&ek_t2).unwrap().completed);

    // Phone un-completes: the todo goes back to not_started (end of column); a done
    // reminder is final and is completed again.
    let other = todo(&pool, "Other", "not_started").await;
    store.edit(&ek_t, |p| p.completed = false);
    store.edit(&ek_r, |p| {
        p.completed = false;
        p.title = "renamed on phone".to_string();
    });
    sync_ok(&pool, &store).await;
    let t_now = inner_get_todo(t.id, &pool).await.unwrap();
    assert_eq!(t_now.status, "not_started");
    assert!(t_now.position > other.position);
    let r_phone = store.item(&ek_r).unwrap();
    assert!(r_phone.completed);
    assert_eq!(r_phone.title, "R");
    assert_eq!(inner_get_reminder(r.id, &pool).await.unwrap().text, "R");

    // Badger reopens a todo: the phone item reopens.
    inner_move_todo(t2.id, "in_progress", 0, &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    assert!(!store.item(&ek_t2).unwrap().completed);
}

// ── AC5, AC15: deletes and the guard ──────────────────────────────────────────

#[tokio::test]
async fn deletes_flow_both_ways() {
    let (pool, store) = setup().await;
    let t1 = todo(&pool, "T1", "not_started").await;
    let t2 = todo(&pool, "T2", "not_started").await;
    let keep = todo(&pool, "Keep", "not_started").await;
    let r1 = reminder(&pool, "R1", "2026-11-20T09:00").await;
    let r2 = reminder(&pool, "R2", "2026-11-20T10:00").await;
    let rk = reminder(&pool, "RK", "2026-11-20T11:00").await;
    sync_ok(&pool, &store).await;
    let ek_t1 = ek_of(&pool, ListKind::Todos, t1.id).await;
    let ek_t2 = ek_of(&pool, ListKind::Todos, t2.id).await;
    let ek_r1 = ek_of(&pool, ListKind::Reminders, r1.id).await;
    let ek_r2 = ek_of(&pool, ListKind::Reminders, r2.id).await;

    store.remove(&ek_t1);
    store.remove(&ek_r1);
    inner_delete_todo(t2.id, &pool).await.unwrap();
    inner_delete_reminder(r2.id, &pool).await.unwrap();
    let report = sync_ok(&pool, &store).await;
    assert!(!report.badger_changed(), "a first miss deletes nothing");
    assert!(inner_get_todo(t1.id, &pool).await.is_ok());
    assert_eq!(
        pairing_of(&pool, ListKind::Todos, t1.id)
            .await
            .unwrap()
            .missed,
        1
    );
    let report = sync_ok(&pool, &store).await;
    assert!(report.badger_changed());

    assert!(
        inner_get_todo(t1.id, &pool).await.is_err(),
        "phone delete → Badger delete"
    );
    assert!(inner_get_reminder(r1.id, &pool).await.is_err());
    assert!(store.item(&ek_t2).is_none(), "Badger delete → phone delete");
    assert!(store.item(&ek_r2).is_none());
    assert!(inner_get_todo(keep.id, &pool).await.is_ok());
    assert!(inner_get_reminder(rk.id, &pool).await.is_ok());
    assert_eq!(pairing_count(&pool).await, 2);
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn phone_delete_of_a_completed_item_only_unpairs() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    let r = reminder(&pool, "R", "2026-11-20T09:00").await;
    sync_ok(&pool, &store).await;
    let ek_t = ek_of(&pool, ListKind::Todos, t.id).await;
    let ek_r = ek_of(&pool, ListKind::Reminders, r.id).await;
    store.edit(&ek_t, |p| p.completed = true);
    store.edit(&ek_r, |p| p.completed = true);
    sync_ok(&pool, &store).await;
    // "Clear completed" on the phone.
    store.remove(&ek_t);
    store.remove(&ek_r);
    sync_ok(&pool, &store).await;
    assert_eq!(inner_get_todo(t.id, &pool).await.unwrap().status, "done");
    assert_eq!(inner_get_reminder(r.id, &pool).await.unwrap().state, "done");
    assert_eq!(pairing_count(&pool).await, 0);
}

#[tokio::test]
async fn phone_delete_of_an_open_todo_cascades_its_reminders() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    let linked = linked_reminder(&pool, "For T", "2026-11-21T09:00", Some(t.id), None).await;
    // Other items keep the lists non-empty (an emptied list trips the guard).
    todo(&pool, "Other", "not_started").await;
    reminder(&pool, "Other", "2026-11-21T10:00").await;
    sync_ok(&pool, &store).await;
    let ek_linked = ek_of(&pool, ListKind::Reminders, linked.id).await;
    store.remove(&ek_of(&pool, ListKind::Todos, t.id).await);
    sync_ok(&pool, &store).await;
    sync_ok(&pool, &store).await; // missing twice in a row
    assert!(inner_get_todo(t.id, &pool).await.is_err());
    assert!(
        inner_get_reminder(linked.id, &pool).await.is_err(),
        "cascade"
    );
    assert!(
        store.item(&ek_linked).is_none(),
        "its phone item goes in the same pass"
    );
    assert_eq!(pairing_count(&pool).await, 2);
}

#[tokio::test]
async fn a_phone_delete_applies_only_when_missing_two_passes_in_a_row() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    todo(&pool, "Other", "not_started").await;
    sync_ok(&pool, &store).await;
    let tl = todos_list(&store);
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    let item = store.item(&ek).unwrap();

    // Missing for one pass (an iCloud glitch), then back: nothing deleted, count reset.
    store.remove(&ek);
    sync_ok(&pool, &store).await;
    assert_eq!(
        pairing_of(&pool, ListKind::Todos, t.id)
            .await
            .unwrap()
            .missed,
        1
    );
    store.add(&tl, item);
    let report = sync_ok(&pool, &store).await;
    assert!(!report.badger_changed());
    assert_eq!(
        pairing_of(&pool, ListKind::Todos, t.id)
            .await
            .unwrap()
            .missed,
        0
    );
    assert_eq!(store.items(&tl).len(), 2, "no duplicate pushed");

    // Missing again: the count starts over, so the second miss in a row deletes.
    store.remove(&ek);
    sync_ok(&pool, &store).await;
    assert!(inner_get_todo(t.id, &pool).await.is_ok());
    let report = sync_ok(&pool, &store).await;
    assert!(report.todos_changed);
    assert!(inner_get_todo(t.id, &pool).await.is_err());
    assert!(pairing_of(&pool, ListKind::Todos, t.id).await.is_none());
}

#[tokio::test]
async fn a_badger_edit_wins_over_a_phone_delete() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    let r = reminder(&pool, "R", "2026-11-20T09:00").await;
    todo(&pool, "Other", "not_started").await;
    reminder(&pool, "Other", "2026-11-20T10:00").await;
    sync_ok(&pool, &store).await;
    let ek_t = ek_of(&pool, ListKind::Todos, t.id).await;
    let ek_r = ek_of(&pool, ListKind::Reminders, r.id).await;

    store.remove(&ek_t);
    store.remove(&ek_r);
    sync_ok(&pool, &store).await;
    // Edited in Badger before the delete applies.
    inner_update_todo(t.id, "T edited", None, None, None, &pool)
        .await
        .unwrap();
    let input = ReminderInput {
        text: "R edited".to_string(),
        remind_at: "2026-11-20T09:00".to_string(),
        repeat: "none".to_string(),
        ..Default::default()
    };
    inner_update_reminder(r.id, &input, at(NOW), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;

    assert_eq!(inner_get_todo(t.id, &pool).await.unwrap().title, "T edited");
    assert_eq!(
        inner_get_reminder(r.id, &pool).await.unwrap().text,
        "R edited"
    );
    let new_t = pairing_of(&pool, ListKind::Todos, t.id).await.unwrap();
    assert_ne!(new_t.ek_id, ek_t, "pushed again");
    assert_eq!(new_t.missed, 0);
    assert_eq!(store.item(&new_t.ek_id).unwrap().title, "T edited");
    let new_r = ek_of(&pool, ListKind::Reminders, r.id).await;
    assert_eq!(store.item(&new_r).unwrap().title, "R edited");
    assert_eq!(store.items(&todos_list(&store)).len(), 2);
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn most_of_a_list_missing_at_once_pauses_until_delete_anyway() {
    let (pool, store) = setup().await;
    let mut todos = Vec::new();
    for title in ["A", "B", "C", "D", "E"] {
        todos.push(todo(&pool, title, "not_started").await);
    }
    sync_ok(&pool, &store).await;
    let eks = [
        ek_of(&pool, ListKind::Todos, todos[0].id).await,
        ek_of(&pool, ListKind::Todos, todos[1].id).await,
        ek_of(&pool, ListKind::Todos, todos[2].id).await,
    ];

    // Two of five missing: below the guard, handled item by item.
    store.remove(&eks[0]);
    store.remove(&eks[1]);
    sync_ok(&pool, &store).await;

    // Three of five: paused, nothing deleted, nothing written.
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
    assert_eq!(todo_count(&pool).await, 5);
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_error.unwrap().contains("3 of the 5"));
    let report = sync(&pool, &store).await;
    assert!(
        matches!(report.outcome, Outcome::Paused { .. }),
        "stays paused"
    );
    assert_eq!(todo_count(&pool).await, 5);

    // Confirmed: all three deletions apply at once.
    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert_eq!(todo_count(&pool).await, 2);
    assert_eq!(pairing_count(&pool).await, 2);
    sync_ok(&pool, &store).await;
}

#[tokio::test]
async fn a_pairing_that_cant_be_saved_takes_its_new_phone_item_back() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let t = todo(&pool, "T", "not_started").await;
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
        store.items(&todos_list(&store)).is_empty(),
        "no unpaired orphan left on the phone"
    );

    sqlx::query("DROP TRIGGER no_pairing")
        .execute(&pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    sync_ok(&pool, &store).await;
    assert_eq!(store.items(&todos_list(&store)).len(), 1);
    assert_eq!(
        todo_count(&pool).await,
        1,
        "nothing imported back as a duplicate"
    );
    assert!(pairing_of(&pool, ListKind::Todos, t.id).await.is_some());
}

#[tokio::test]
async fn delete_anyway_only_overrides_an_empty_fetch_pause() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    let u = todo(&pool, "U", "not_started").await;
    todo(&pool, "V", "not_started").await;
    sync_ok(&pool, &store).await;
    store.remove(&ek_of(&pool, ListKind::Todos, t.id).await);

    // Not paused (a stale click): an ordinary pass, so the first miss deletes nothing.
    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(inner_get_todo(t.id, &pool).await.is_ok());

    // Paused for another reason (access): still an ordinary pass once access is back.
    store.remove(&ek_of(&pool, ListKind::Todos, u.id).await);
    store.set_access(AccessState::Denied);
    sync(&pool, &store).await;
    store.set_access(AccessState::Granted);
    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(
        inner_get_todo(u.id, &pool).await.is_ok(),
        "first miss: {report:?}"
    );
    assert!(
        inner_get_todo(t.id, &pool).await.is_err(),
        "second miss in a row"
    );
}

#[tokio::test]
async fn the_missing_guard_counts_only_rows_a_miss_would_delete() {
    let (pool, store) = setup().await;
    let mut t = Vec::new();
    for title in ["A", "B", "C", "D", "E", "F"] {
        t.push(todo(&pool, title, "not_started").await);
    }
    sync_ok(&pool, &store).await;
    let mut eks = Vec::new();
    for x in &t {
        eks.push(ek_of(&pool, ListKind::Todos, x.id).await);
    }
    // Badger completes A and B and deletes C; the phone loses A–D before the next pass.
    inner_move_todo(t[0].id, "done", 0, &pool).await.unwrap();
    inner_move_todo(t[1].id, "done", 0, &pool).await.unwrap();
    inner_delete_todo(t[2].id, &pool).await.unwrap();
    for ek in &eks[..4] {
        store.remove(ek);
    }
    // Only D would be deleted in Badger: one of three, no pause.
    sync_ok(&pool, &store).await;
    assert_eq!(inner_get_todo(t[0].id, &pool).await.unwrap().status, "done");
    assert!(pairing_of(&pool, ListKind::Todos, t[0].id).await.is_none());
    assert_eq!(
        pairing_of(&pool, ListKind::Todos, t[3].id)
            .await
            .unwrap()
            .missed,
        1
    );
}

#[tokio::test]
async fn moving_three_items_to_the_other_list_does_not_trip_the_guard() {
    let (pool, store) = setup().await;
    let mut t = Vec::new();
    for title in ["A", "B", "C", "D"] {
        t.push(todo(&pool, title, "not_started").await);
    }
    sync_ok(&pool, &store).await;
    let rl = reminders_list(&store);
    for x in &t[..3] {
        let ek = ek_of(&pool, ListKind::Todos, x.id).await;
        store.edit(&ek, |p| {
            p.due = Some(PhoneDue::DateTime(at("2026-11-20T15:00")))
        });
        store.move_to(&ek, &rl);
    }
    sync_ok(&pool, &store).await;
    assert_eq!(todo_count(&pool).await, 1);
    assert_eq!(reminder_count(&pool).await, 3);
    assert_eq!(pairing_count(&pool).await, 4);
}

#[tokio::test]
async fn a_re_push_whose_pairing_cant_be_saved_takes_its_phone_item_back() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    todo(&pool, "U", "not_started").await;
    sync_ok(&pool, &store).await;
    let tl = todos_list(&store);
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    store.remove(&ek);
    let mut conn = pool.acquire().await.unwrap();
    set_apple_sync_restore_pending(&mut conn, true)
        .await
        .unwrap();
    drop(conn);
    sqlx::query(
        "CREATE TRIGGER no_repair BEFORE UPDATE OF EkId ON apple_sync_items \
         WHEN NEW.EkId != OLD.EkId BEGIN SELECT RAISE(ABORT, 'disk full'); END",
    )
    .execute(&pool)
    .await
    .unwrap();
    let report = sync(&pool, &store).await;
    assert_eq!(report.errors.len(), 1, "{report:?}");
    assert_eq!(
        store.items(&tl).len(),
        1,
        "the re-pushed item was removed again"
    );

    sqlx::query("DROP TRIGGER no_repair")
        .execute(&pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    sync_ok(&pool, &store).await;
    assert_eq!(store.items(&tl).len(), 2);
    assert_eq!(todo_count(&pool).await, 2);
    assert_ne!(ek_of(&pool, ListKind::Todos, t.id).await, ek);
}

async fn synced_pair(pool: &Pool<Sqlite>, store: &FakeStore) -> (TodoRow, Reminder) {
    let t = todo(pool, "T", "not_started").await;
    let r = reminder(pool, "R", "2026-11-20T09:00").await;
    sync_ok(pool, store).await;
    (t, r)
}

async fn assert_nothing_deleted(pool: &Pool<Sqlite>, t: &TodoRow, r: &Reminder) {
    assert!(inner_get_todo(t.id, pool).await.is_ok());
    assert!(inner_get_reminder(r.id, pool).await.is_ok());
    assert_eq!(pairing_count(pool).await, 2);
}

#[tokio::test]
async fn guard_pauses_without_deleting_when_a_list_is_missing() {
    let (pool, store) = setup().await;
    let (t, r) = synced_pair(&pool, &store).await;
    store.remove_list(&todos_list(&store));
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_LIST_MISSING,
            ..
        }
    ));
    assert_eq!(report.phone_writes, 0);
    assert_nothing_deleted(&pool, &t, &r).await;
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert_eq!(s.paused.as_deref(), Some(PAUSED_LIST_MISSING));
    assert!(s.last_error.unwrap().contains("missing"));
}

#[tokio::test]
async fn guard_pauses_without_deleting_when_access_is_lost() {
    let (pool, store) = setup().await;
    let (t, r) = synced_pair(&pool, &store).await;
    store.set_access(AccessState::Denied);
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_ACCESS,
            ..
        }
    ));
    assert_nothing_deleted(&pool, &t, &r).await;

    // Access back: sync resumes by itself.
    store.set_access(AccessState::Granted);
    sync_ok(&pool, &store).await;
    assert_eq!(get_apple_sync_settings(&pool).await.unwrap().paused, None);
}

#[tokio::test]
async fn guard_pauses_on_an_empty_list_until_delete_anyway() {
    let (pool, store) = setup().await;
    let (t, r) = synced_pair(&pool, &store).await;
    let t2 = todo(&pool, "Second", "not_started").await;
    sync_ok(&pool, &store).await;
    let other = todo(&pool, "Not yet pushed", "not_started").await;
    store.clear_list(&todos_list(&store));
    let report = sync(&pool, &store).await;
    assert!(matches!(
        report.outcome,
        Outcome::Paused {
            kind: PAUSED_EMPTY_FETCH,
            ..
        }
    ));
    assert_eq!(report.phone_writes, 0, "the pass writes nothing at all");
    assert!(inner_get_todo(t.id, &pool).await.is_ok());
    assert!(inner_get_todo(t2.id, &pool).await.is_ok());
    assert!(inner_get_reminder(r.id, &pool).await.is_ok());
    assert_eq!(pairing_count(&pool).await, 3);
    assert!(pairing_of(&pool, ListKind::Todos, other.id).await.is_none());

    // Confirmed: the deletions apply once.
    let report = run_pass(&pool, &store, at(NOW), PassMode::DeleteAnyway).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(inner_get_todo(t.id, &pool).await.is_err());
    assert!(inner_get_todo(t2.id, &pool).await.is_err());
    assert!(inner_get_todo(other.id, &pool).await.is_ok());
    assert_eq!(get_apple_sync_settings(&pool).await.unwrap().paused, None);
}

#[tokio::test]
async fn deleting_the_only_open_item_on_the_phone_does_not_pause() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "Only one", "not_started").await;
    sync_ok(&pool, &store).await;
    store.clear_list(&todos_list(&store));

    // First miss: no pause, nothing deleted yet (two-pass rule).
    let report = sync(&pool, &store).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(inner_get_todo(t.id, &pool).await.is_ok());

    // Second miss: deleted in Badger.
    assert_eq!(sync(&pool, &store).await.outcome, Outcome::Synced);
    assert!(inner_get_todo(t.id, &pool).await.is_err());
    assert_eq!(get_apple_sync_settings(&pool).await.unwrap().paused, None);
}

// ── AC6: one-off ↔ recurring ──────────────────────────────────────────────────

#[tokio::test]
async fn becoming_recurring_removes_the_phone_item_and_one_off_pushes_it_again() {
    let (pool, store) = setup().await;
    let r = reminder(&pool, "Standup", "2026-11-20T09:00").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Reminders, r.id).await;

    let daily = ReminderInput {
        text: "Standup".to_string(),
        remind_at: "2026-11-20T09:00".to_string(),
        repeat: "daily".to_string(),
        ..Default::default()
    };
    inner_update_reminder(r.id, &daily, at(NOW), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    assert!(store.item(&ek).is_none());
    assert!(pairing_of(&pool, ListKind::Reminders, r.id).await.is_none());
    assert_eq!(
        inner_get_reminder(r.id, &pool).await.unwrap().repeat,
        "daily"
    );
    assert_eq!(
        sync_ok(&pool, &store).await.phone_writes,
        0,
        "stays Mac-only"
    );

    let once = ReminderInput {
        repeat: "none".to_string(),
        remind_at: "2026-11-21T09:00".to_string(),
        ..daily
    };
    inner_update_reminder(r.id, &once, at(NOW), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Reminders, r.id).await;
    assert_eq!(
        store.item(&ek).unwrap().due,
        Some(PhoneDue::DateTime(at("2026-11-21T09:00")))
    );
}

// ── AC7: re-pairing ───────────────────────────────────────────────────────────

#[tokio::test]
async fn a_changed_eventkit_id_re_pairs_by_external_id() {
    let (pool, store) = setup().await;
    let (t, r) = synced_pair(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    store.change_ek_id(&ek, "ek-renumbered");
    store.edit("ek-renumbered", |p| p.title = "T on phone".to_string());
    sync_ok(&pool, &store).await;
    assert_eq!(ek_of(&pool, ListKind::Todos, t.id).await, "ek-renumbered");
    assert_eq!(
        inner_get_todo(t.id, &pool).await.unwrap().title,
        "T on phone"
    );
    assert_eq!(
        store.items(&todos_list(&store)).len(),
        1,
        "no duplicate on the phone"
    );
    assert_eq!(todo_count(&pool).await, 1, "no duplicate in Badger");
    assert!(inner_get_reminder(r.id, &pool).await.is_ok());
}

// ── AC8: restore ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn after_a_restore_badger_is_authoritative_for_one_pass() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    let r = reminder(&pool, "R", "2026-11-20T09:00").await;
    reminder(&pool, "Another", "2026-11-20T10:00").await; // keeps the list non-empty
    let gone = todo(&pool, "Not in the backup", "not_started").await;
    sync_ok(&pool, &store).await;
    let ek_t = ek_of(&pool, ListKind::Todos, t.id).await;
    let ek_r = ek_of(&pool, ListKind::Reminders, r.id).await;
    let ek_gone = ek_of(&pool, ListKind::Todos, gone.id).await;

    // As if the restored data lacked `gone`, and the phone drifted.
    sqlx::query("DELETE FROM todos WHERE id = ?")
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
    assert_eq!(inner_get_todo(t.id, &pool).await.unwrap().title, "T");
    assert!(
        inner_get_reminder(r.id, &pool).await.is_ok(),
        "nothing deleted in Badger"
    );
    let new_ek = ek_of(&pool, ListKind::Reminders, r.id).await;
    assert_ne!(new_ek, ek_r);
    assert_eq!(
        store.item(&new_ek).unwrap().title,
        "R",
        "re-created on the phone"
    );
    // The phone item of a row the backup lacks is kept and imported, not deleted.
    assert!(store.item(&ek_gone).is_some());
    assert_eq!(
        inner_list_todos(&pool)
            .await
            .unwrap()
            .iter()
            .filter(|x| x.title == "Not in the backup")
            .count(),
        1
    );
    assert!(
        !get_apple_sync_settings(&pool)
            .await
            .unwrap()
            .restore_pending
    );

    // Back to normal merging.
    store.edit(&ek_t, |p| p.title = "edited again".to_string());
    sync_ok(&pool, &store).await;
    assert_eq!(
        inner_get_todo(t.id, &pool).await.unwrap().title,
        "edited again"
    );
}

// ── AC14: interleavings ───────────────────────────────────────────────────────

#[tokio::test]
async fn a_scheduler_fire_then_a_phone_time_change_keeps_the_phone_time() {
    let (pool, store) = setup().await;
    let r = reminder(&pool, "R", "2026-11-19T09:00").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Reminders, r.id).await;
    // The scheduler fires it, then the user moves it on the phone.
    inner_fire_due(&pool, at(NOW)).await.unwrap();
    store.edit(&ek, |p| {
        p.due = Some(PhoneDue::DateTime(at("2026-11-19T18:00")))
    });
    sync_ok(&pool, &store).await;
    let now = inner_get_reminder(r.id, &pool).await.unwrap();
    assert_eq!(now.remind_at, "2026-11-19T18:00:00.000");
    assert_eq!(now.state, "pending", "a snooze");
    assert_eq!(
        store.item(&ek).unwrap().due,
        Some(PhoneDue::DateTime(at("2026-11-19T18:00"))),
        "no revert"
    );
    // A move into the past imports as overdue.
    store.edit(&ek, |p| {
        p.due = Some(PhoneDue::DateTime(at("2026-11-19T10:00")))
    });
    sync_ok(&pool, &store).await;
    assert_eq!(
        inner_get_reminder(r.id, &pool).await.unwrap().state,
        "fired"
    );
}

#[tokio::test]
async fn a_phone_completion_and_a_badger_title_edit_are_both_kept() {
    let (pool, store) = setup().await;
    let r = reminder(&pool, "R", "2026-11-20T09:00").await;
    let t = todo(&pool, "T", "in_progress").await;
    sync_ok(&pool, &store).await;
    let ek_r = ek_of(&pool, ListKind::Reminders, r.id).await;
    let ek_t = ek_of(&pool, ListKind::Todos, t.id).await;
    store.edit(&ek_r, |p| p.completed = true);
    store.edit(&ek_t, |p| p.completed = true);
    let input = ReminderInput {
        text: "R renamed".to_string(),
        remind_at: "2026-11-20T09:00".to_string(),
        repeat: "none".to_string(),
        ..Default::default()
    };
    inner_update_reminder(r.id, &input, at(NOW), &pool)
        .await
        .unwrap();
    inner_update_todo(t.id, "T renamed", None, None, None, &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    let r_now = inner_get_reminder(r.id, &pool).await.unwrap();
    assert_eq!(
        (r_now.text.as_str(), r_now.state.as_str()),
        ("R renamed", "done")
    );
    let t_now = inner_get_todo(t.id, &pool).await.unwrap();
    assert_eq!(
        (t_now.title.as_str(), t_now.status.as_str()),
        ("T renamed", "done")
    );
    let rp = store.item(&ek_r).unwrap();
    assert_eq!((rp.title.as_str(), rp.completed), ("R renamed", true));
    let tp = store.item(&ek_t).unwrap();
    assert_eq!((tp.title.as_str(), tp.completed), ("T renamed", true));
}

// ── AC16: non-synced fields ───────────────────────────────────────────────────

#[tokio::test]
async fn phone_edits_leave_issue_and_position_alone() {
    use crate::commands::customers::inner_create_customer;
    use crate::commands::issues::inner_resolve_issue;
    use crate::commands::projects::inner_create_project;
    let (pool, store) = setup().await;
    let c = inner_create_customer("Acme", &pool).await.unwrap();
    let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
    let issue = inner_resolve_issue(p.id, "Bug", &pool).await.unwrap();
    todo(&pool, "First", "not_started").await;
    let t = inner_create_todo("T", None, None, "not_started", Some(issue), &pool)
        .await
        .unwrap();
    sync_ok(&pool, &store).await;
    store.edit(&ek_of(&pool, ListKind::Todos, t.id).await, |p| {
        p.title = "T on phone".to_string()
    });
    sync_ok(&pool, &store).await;
    let now = inner_get_todo(t.id, &pool).await.unwrap();
    assert_eq!(now.title, "T on phone");
    assert_eq!((now.issue_id, now.position), (Some(issue), t.position));
}

// ── Edge cases ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn moving_an_item_between_the_lists_changes_its_kind() {
    let (pool, store) = setup().await;
    let t = todo(&pool, "Call the bank", "not_started").await;
    todo(&pool, "Stays", "not_started").await; // an emptied list would trip the guard
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    store.edit(&ek, |p| {
        p.due = Some(PhoneDue::DateTime(at("2026-11-20T15:00")))
    });
    store.move_to(&ek, &reminders_list(&store));
    sync_ok(&pool, &store).await;
    assert!(
        inner_get_todo(t.id, &pool).await.is_err(),
        "the todo is gone"
    );
    let p = load_pairings(&pool)
        .await
        .unwrap()
        .into_iter()
        .find(|p| p.ek_id == ek)
        .unwrap();
    assert_eq!(p.kind, ListKind::Reminders);
    let r = inner_get_reminder(p.badger_id, &pool).await.unwrap();
    assert_eq!(r.text, "Call the bank");
    assert_eq!(r.remind_at, "2026-11-20T15:00:00.000");
    assert_eq!(pairing_count(&pool).await, 2);
}

#[tokio::test]
async fn a_failed_pass_is_retried_without_duplicates() {
    let (pool, store) = setup().await;
    sync_ok(&pool, &store).await;
    let t = todo(&pool, "T", "not_started").await;
    let ek = store.add(
        &reminders_list(&store),
        with_time("Phone one", "2026-11-20T09:00"),
    );
    store.fail_writes(true);
    let report = sync(&pool, &store).await;
    assert_eq!(report.outcome, Outcome::Synced);
    assert!(!report.errors.is_empty());
    assert!(pairing_of(&pool, ListKind::Todos, t.id).await.is_none());
    let s = get_apple_sync_settings(&pool).await.unwrap();
    assert!(s.last_error.is_some(), "errors show in Settings");
    // The import itself committed and paired; only its phone touch-up failed.
    assert_eq!(reminder_count(&pool).await, 1);

    store.fail_writes(false);
    sync_ok(&pool, &store).await;
    assert_eq!(store.items(&todos_list(&store)).len(), 1);
    assert_eq!(store.items(&reminders_list(&store)).len(), 1);
    assert_eq!(reminder_count(&pool).await, 1);
    assert_eq!(
        store.item(&ek).unwrap().alarms,
        vec![at("2026-11-20T09:00")]
    );
    assert_eq!(
        get_apple_sync_settings(&pool).await.unwrap().last_error,
        None
    );
    assert_eq!(sync_ok(&pool, &store).await.phone_writes, 0);
}

#[tokio::test]
async fn a_real_restore_keeps_pairings_and_makes_one_pass_authoritative() {
    use crate::commands::backup::{inner_backup_data, inner_restore_data};
    let (pool, store) = setup().await;
    let t = todo(&pool, "T", "not_started").await;
    todo(&pool, "U", "not_started").await;
    sync_ok(&pool, &store).await;
    let ek = ek_of(&pool, ListKind::Todos, t.id).await;
    let backup = inner_backup_data(&pool).await.unwrap();

    // After the backup: the phone edits T, Badger edits it too.
    store.edit(&ek, |p| p.title = "phone".to_string());
    inner_update_todo(t.id, "after backup", None, None, None, &pool)
        .await
        .unwrap();
    inner_restore_data(&backup, &pool).await.unwrap();
    assert_eq!(pairing_count(&pool).await, 2, "pairings survive restore");

    let report = sync_ok(&pool, &store).await;
    assert_eq!(inner_get_todo(t.id, &pool).await.unwrap().title, "T");
    assert_eq!(
        store.item(&ek).unwrap().title,
        "T",
        "the restored data wins"
    );
    assert!(!report.todos_changed, "nothing changed in Badger");
    assert_eq!(todo_count(&pool).await, 2, "no duplicates");
    assert!(
        !get_apple_sync_settings(&pool)
            .await
            .unwrap()
            .restore_pending
    );
}

#[tokio::test]
async fn a_synced_reminder_is_left_to_the_phone_until_its_time_differs() {
    use crate::apple_sync::phone_notifies;
    let (pool, store) = setup().await;
    let r = reminder(&pool, "R", "2026-11-19T09:00").await;
    let unsynced = reminder(&pool, "Later", "2026-11-19T09:00").await;
    sync_ok(&pool, &store).await;
    // Created after the pass: not on the phone yet.
    sqlx::query("DELETE FROM apple_sync_items WHERE BadgerId = ? AND Kind = 'reminder'")
        .bind(unsynced.id)
        .execute(&pool)
        .await
        .unwrap();
    let fired = inner_fire_due(&pool, at(NOW)).await.unwrap();
    let skip = phone_notifies(&pool, &fired).await;
    assert!(skip.contains(&r.id));
    assert!(!skip.contains(&unsynced.id));

    // The phone item lost its alarm: Badger notifies again until the next push.
    let ek = ek_of(&pool, ListKind::Reminders, r.id).await;
    store.edit(&ek, |p| p.alarms.clear());
    let mut item = store.item(&ek).unwrap();
    item.alarms.clear();
    assert_eq!(phone_due(ListKind::Reminders, &item), None);
}
