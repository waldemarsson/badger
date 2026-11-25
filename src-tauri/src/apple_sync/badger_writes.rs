//! The sync's database side: loading the tasks in scope, conditional Badger writes, imports
//! and the pairing table.
//!
//! Writes are limited to the synced columns and conditional on the value the pass read
//! (`WHERE Title IS ?`), never whole-row rewrites, so a concurrent user edit and the
//! non-synced fields (links, repeat, reminders' states) are never clobbered. A lost race
//! returns `Ok(false)` and rolls back; the item is left for the next pass.

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};

use crate::commands::tasks::{
    base_fire_time, clean_due, clean_notes, clean_title, complete_in, due_anchor,
    reconcile_reminders, reopen_in, MAX_REMINDERS, MAX_TITLE_CHARS, UNDO_WINDOW_SECS,
};
use crate::recurrence::{format_timestamp, local_exists, local_now, parse_timestamp};
use crate::AppError;

use super::{alarm_key, PhoneDue};

/// The synced fields of one side, in Badger terms, as JSON in the pairing table. `due` is
/// `YYYY-MM-DD` or `YYYY-MM-DDTHH:MM` (see [`due_key`]). Alarms aren't merged: Badger owns
/// them.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Snap {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    #[serde(default)]
    pub due: Option<String>,
    #[serde(default)]
    pub done: bool,
}

impl Snap {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// An unreadable snapshot reads as empty, so both sides look changed and Badger wins.
    pub fn from_json(s: &str) -> Snap {
        serde_json::from_str(s).unwrap_or_default()
    }
}

// ── Due ───────────────────────────────────────────────────────────────────────

/// A task's due in snapshot form: the date, plus `THH:MM` when it has a time.
pub fn due_key(due_date: Option<&str>, due_time: Option<&str>) -> Option<String> {
    due_date.map(|d| match due_time {
        Some(t) => format!("{d}T{t}"),
        None => d.to_string(),
    })
}

/// A phone due in snapshot form (seconds dropped).
pub fn phone_due_key(due: Option<PhoneDue>) -> Option<String> {
    due.map(|d| match d {
        PhoneDue::Date(d) => d.format("%Y-%m-%d").to_string(),
        PhoneDue::DateTime(t) => t.format("%Y-%m-%dT%H:%M").to_string(),
    })
}

/// A snapshot due back as (date, time) columns.
pub fn split_due(key: Option<&str>) -> (Option<String>, Option<String>) {
    match key.map(|k| k.split_once('T')) {
        None => (None, None),
        Some(Some((d, t))) => (Some(d.to_string()), Some(t.to_string())),
        Some(None) => (key.map(str::to_string), None),
    }
}

/// A snapshot due as the phone's due: a floating date, or a date-time.
pub fn to_phone_due(key: Option<&str>) -> Option<PhoneDue> {
    let (date, time) = split_due(key);
    let date = NaiveDate::parse_from_str(date.as_deref()?, "%Y-%m-%d").ok()?;
    Some(
        match time.and_then(|t| NaiveTime::parse_from_str(&t, "%H:%M").ok()) {
            Some(t) => PhoneDue::DateTime(date.and_time(t)),
            None => PhoneDue::Date(date),
        },
    )
}

/// Task title from a phone title: trimmed and cut to 500 characters; `None` when empty.
pub fn cut_title(title: &str) -> Option<String> {
    let cut: String = title.trim().chars().take(MAX_TITLE_CHARS).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

// ── Loading ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct BTask {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub completed_at: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub repeat: String,
    /// "Keep off phone": out of the sync.
    pub mac_only: bool,
    /// The reminders' own fire times (snoozes are Badger-only), filled in after loading.
    #[sqlx(skip)]
    pub alarm_times: Vec<NaiveDateTime>,
}

impl BTask {
    /// Out of the sync, so never pushed or deleted because of the phone: repeating (D6) or
    /// kept off the phone.
    pub fn off_phone(&self) -> bool {
        self.mac_only || self.repeat != "none"
    }

    /// Open and synced: pushed when unpaired.
    pub fn in_scope(&self) -> bool {
        !self.done && !self.off_phone()
    }

    pub fn snap(&self) -> Snap {
        Snap {
            title: self.title.clone(),
            notes: self.notes.clone(),
            due: due_key(self.due_date.as_deref(), self.due_time.as_deref()),
            done: self.done,
        }
    }

    /// Completed less than the Undo window ago: sync leaves it for a later pass, so an
    /// undone completion never reaches the phone (D21).
    pub fn in_undo_window(&self, now: NaiveDateTime) -> bool {
        self.done
            && self
                .completed_at
                .as_deref()
                .and_then(|c| parse_timestamp(c).ok())
                .is_some_and(|c| now < c + chrono::Duration::seconds(UNDO_WINDOW_SECS))
    }

    /// The absolute alarms the phone item should have: one per reminder whose time is still
    /// ahead, plus past ones the item already has (kept, so they aren't churned or cancelled
    /// as they pass, and never newly pushed).
    pub fn wanted_alarms(
        &self,
        on_phone: &[NaiveDateTime],
        now: NaiveDateTime,
    ) -> Vec<NaiveDateTime> {
        let phone: Vec<String> = on_phone.iter().map(|t| alarm_key(*t)).collect();
        let mut want: Vec<NaiveDateTime> = self
            .alarm_times
            .iter()
            .map(|t| t.with_nanosecond(0).unwrap_or(*t))
            .filter(|t| *t > now || phone.contains(&alarm_key(*t)))
            .collect();
        want.sort();
        want.dedup();
        want
    }
}

const TASK_COLUMNS: &str = "SELECT id, Title AS title, Notes AS notes, Done AS done, \
     CompletedAt AS completed_at, DueDate AS due_date, DueTime AS due_time, Repeat AS repeat, \
     MacOnly AS mac_only FROM tasks";

async fn with_alarms(mut task: BTask, conn: &mut SqliteConnection) -> Result<BTask, AppError> {
    let reminders: Vec<(Option<i64>, Option<String>)> =
        sqlx::query_as("SELECT OffsetMinutes, At FROM task_reminders WHERE TaskId = ? ORDER BY id")
            .bind(task.id)
            .fetch_all(conn)
            .await?;
    let anchor = due_anchor(
        task.due_date.as_deref(),
        task.due_time.as_deref(),
        local_exists,
    );
    task.alarm_times = reminders
        .into_iter()
        .filter_map(|(offset, at)| base_fire_time(anchor, offset, at.as_deref(), local_exists))
        .collect();
    Ok(task)
}

/// Open tasks plus every paired task (done ones included, so they aren't taken for deleted).
pub async fn load_tasks(pool: &Pool<Sqlite>) -> Result<Vec<BTask>, AppError> {
    let sql = format!(
        "{TASK_COLUMNS} WHERE Done = 0 OR id IN (SELECT BadgerId FROM apple_sync_items) ORDER BY id"
    );
    let mut conn = pool.acquire().await?;
    let rows = sqlx::query_as::<_, BTask>(sqlx::AssertSqlSafe(sql))
        .fetch_all(&mut *conn)
        .await?;
    let mut tasks = Vec::with_capacity(rows.len());
    for t in rows {
        tasks.push(with_alarms(t, &mut conn).await?);
    }
    Ok(tasks)
}

pub async fn load_task(id: i64, pool: &Pool<Sqlite>) -> Result<Option<BTask>, AppError> {
    let mut conn = pool.acquire().await?;
    load_task_in(id, &mut conn).await
}

async fn load_task_in(id: i64, conn: &mut SqliteConnection) -> Result<Option<BTask>, AppError> {
    let sql = format!("{TASK_COLUMNS} WHERE id = ?");
    let row = sqlx::query_as::<_, BTask>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(&mut *conn)
        .await?;
    match row {
        Some(t) => Ok(Some(with_alarms(t, conn).await?)),
        None => Ok(None),
    }
}

// ── Pairings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    pub id: i64,
    pub badger_id: i64,
    pub ek_id: String,
    pub external_id: Option<String>,
    pub badger_snap: Snap,
    pub phone_snap: Snap,
    /// The item's absolute alarms after the last sync (storage format).
    pub phone_alarms: Vec<String>,
    /// Consecutive passes the phone item was missing from its list.
    pub missed: i64,
}

#[derive(FromRow)]
struct PairingRow {
    id: i64,
    badger_id: i64,
    ek_id: String,
    external_id: Option<String>,
    badger_snap: String,
    phone_snap: String,
    phone_alarms: String,
    missed: i64,
}

pub async fn load_pairings(pool: &Pool<Sqlite>) -> Result<Vec<Pairing>, AppError> {
    let rows = sqlx::query_as::<_, PairingRow>(
        "SELECT id, BadgerId AS badger_id, EkId AS ek_id, ExternalId AS external_id, \
             BadgerSnap AS badger_snap, PhoneSnap AS phone_snap, PhoneAlarms AS phone_alarms, \
             MissedPasses AS missed \
         FROM apple_sync_items ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Pairing {
            id: r.id,
            badger_id: r.badger_id,
            ek_id: r.ek_id,
            external_id: r.external_id,
            badger_snap: Snap::from_json(&r.badger_snap),
            phone_snap: Snap::from_json(&r.phone_snap),
            phone_alarms: serde_json::from_str(&r.phone_alarms).unwrap_or_default(),
            missed: r.missed,
        })
        .collect())
}

/// What a pairing records after a sync of the item (the phone item was seen, so the
/// missed-pass count starts over).
#[derive(Debug, Clone, PartialEq)]
pub struct PairingState {
    pub ek_id: String,
    pub external_id: Option<String>,
    pub badger_snap: Snap,
    pub phone_snap: Snap,
    pub phone_alarms: Vec<String>,
}

fn alarms_json(alarms: &[String]) -> String {
    serde_json::to_string(alarms).unwrap_or_else(|_| "[]".to_string())
}

/// Returns the new pairing's id.
pub async fn insert_pairing(
    badger_id: i64,
    state: &PairingState,
    conn: &mut SqliteConnection,
) -> Result<i64, AppError> {
    let result = sqlx::query(
        "INSERT INTO apple_sync_items \
             (BadgerId, EkId, ExternalId, BadgerSnap, PhoneSnap, PhoneAlarms, SyncedAt) \
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(badger_id)
    .bind(&state.ek_id)
    .bind(&state.external_id)
    .bind(state.badger_snap.to_json())
    .bind(state.phone_snap.to_json())
    .bind(alarms_json(&state.phone_alarms))
    .bind(format_timestamp(local_now()))
    .execute(conn)
    .await?;
    Ok(result.last_insert_rowid())
}

pub async fn update_pairing(
    pairing_id: i64,
    state: &PairingState,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE apple_sync_items SET EkId = ?, ExternalId = ?, BadgerSnap = ?, PhoneSnap = ?, \
             PhoneAlarms = ?, SyncedAt = ?, MissedPasses = 0 \
         WHERE id = ?",
    )
    .bind(&state.ek_id)
    .bind(&state.external_id)
    .bind(state.badger_snap.to_json())
    .bind(state.phone_snap.to_json())
    .bind(alarms_json(&state.phone_alarms))
    .bind(format_timestamp(local_now()))
    .bind(pairing_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn set_pairing_ek_id(
    pairing_id: i64,
    ek_id: &str,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    sqlx::query("UPDATE apple_sync_items SET EkId = ? WHERE id = ?")
        .bind(ek_id)
        .bind(pairing_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Records how many consecutive passes the pairing's phone item was missing.
pub async fn set_pairing_missed(
    pairing_id: i64,
    missed: i64,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    sqlx::query("UPDATE apple_sync_items SET MissedPasses = ? WHERE id = ?")
        .bind(missed)
        .bind(pairing_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Starts every missed-pass count over (sync turned on: passes while it was off don't
/// count as consecutive).
pub async fn reset_missed(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    sqlx::query("UPDATE apple_sync_items SET MissedPasses = 0 WHERE MissedPasses != 0")
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_pairing(
    pairing_id: i64,
    conn: impl sqlx::SqliteExecutor<'_>,
) -> Result<(), AppError> {
    sqlx::query("DELETE FROM apple_sync_items WHERE id = ?")
        .bind(pairing_id)
        .execute(conn)
        .await?;
    Ok(())
}

/// Drops every pairing ("Recreate list").
pub async fn delete_all_pairings(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    sqlx::query("DELETE FROM apple_sync_items")
        .execute(pool)
        .await?;
    Ok(())
}

// ── Conditional writes ────────────────────────────────────────────────────────

/// Writes `target`'s fields that differ from `loaded`, each conditional on the loaded value.
/// A new due moves the task's reminders with it (re-armed or dismissed silently, D20); done
/// suppresses them; reopening dismisses past ones (D8). `Ok(false)`: the task changed
/// meanwhile, nothing written.
pub async fn apply_task(
    loaded: &BTask,
    target: &Snap,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let current = loaded.snap();
    let stamp = format_timestamp(local_now());
    if target.title != current.title {
        let title = clean_title(&target.title)?;
        let done = sqlx::query(
            "UPDATE tasks SET Title = ?, UpdatedAt = ? WHERE id = ? AND Title IS ? AND Repeat = 'none'",
        )
        .bind(&title)
        .bind(&stamp)
        .bind(loaded.id)
        .bind(&loaded.title)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
    }
    if target.notes != current.notes {
        let done = sqlx::query(
            "UPDATE tasks SET Notes = ?, UpdatedAt = ? WHERE id = ? AND Notes IS ? AND Repeat = 'none'",
        )
        .bind(clean_notes(target.notes.as_deref()))
        .bind(&stamp)
        .bind(loaded.id)
        .bind(&loaded.notes)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
    }
    if target.due != current.due {
        let (date, time) = split_due(target.due.as_deref());
        let (date, time) = clean_due(date.as_deref(), time.as_deref())?;
        let done = sqlx::query(
            "UPDATE tasks SET DueDate = ?, DueTime = ?, UpdatedAt = ? \
             WHERE id = ? AND DueDate IS ? AND DueTime IS ? AND Repeat = 'none'",
        )
        .bind(&date)
        .bind(&time)
        .bind(&stamp)
        .bind(loaded.id)
        .bind(&loaded.due_date)
        .bind(&loaded.due_time)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
        reconcile_reminders(loaded.id, now, &mut tx).await?;
    }
    if target.done != current.done {
        let still: Option<bool> =
            sqlx::query_scalar("SELECT Done FROM tasks WHERE id = ? AND Repeat = 'none'")
                .bind(loaded.id)
                .fetch_optional(&mut *tx)
                .await?;
        if still != Some(loaded.done) {
            return Ok(false);
        }
        if target.done {
            complete_in(loaded.id, now, &mut tx).await?;
        } else {
            reopen_in(loaded.id, now, &mut tx).await?;
        }
    }
    tx.commit().await?;
    Ok(true)
}

// ── Imports and deletes ───────────────────────────────────────────────────────

/// A phone-created task, open, paired in the same transaction. Its absolute alarms become
/// reminders (D22): one at the due time is "At due time", others are custom; the earliest
/// ten are kept, and those already past are stored dismissed. Returns the task id and the
/// pairing's.
pub async fn import_task(
    snap: &Snap,
    alarms: &[NaiveDateTime],
    pairing: &PairingState,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<(i64, i64), AppError> {
    let title = clean_title(&snap.title)?;
    let (date, time) = split_due(snap.due.as_deref());
    let (due_date, due_time) = clean_due(date.as_deref(), time.as_deref())?;
    let notes = clean_notes(snap.notes.as_deref());
    let stamp = format_timestamp(local_now());
    let mut tx = pool.begin().await?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO tasks (Title, Notes, Done, DueDate, DueTime, CreatedAt, UpdatedAt) \
         VALUES (?, ?, 0, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&title)
    .bind(&notes)
    .bind(&due_date)
    .bind(&due_time)
    .bind(&stamp)
    .bind(&stamp)
    .fetch_one(&mut *tx)
    .await?;

    let at_due = match to_phone_due(snap.due.as_deref()) {
        Some(PhoneDue::DateTime(t)) => Some(t),
        _ => None,
    };
    let mut alarms: Vec<NaiveDateTime> = alarms
        .iter()
        .map(|t| t.with_nanosecond(0).unwrap_or(*t))
        .collect();
    alarms.sort();
    alarms.dedup();
    for t in alarms.into_iter().take(MAX_REMINDERS) {
        let (offset, at) = if at_due.is_some_and(|d| d.with_second(0) == t.with_second(0)) {
            (Some(0_i64), None)
        } else {
            (None, Some(format_timestamp(t)))
        };
        sqlx::query(
            "INSERT INTO task_reminders (TaskId, OffsetMinutes, At, State, CreatedAt) \
             VALUES (?, ?, ?, 'pending', ?)",
        )
        .bind(id)
        .bind(offset)
        .bind(at)
        .bind(&stamp)
        .execute(&mut *tx)
        .await?;
    }
    // Arms each for its time: past ones become dismissed, without a notification.
    reconcile_reminders(id, now, &mut tx).await?;

    let mut state = pairing.clone();
    state.badger_snap = Snap {
        title,
        notes,
        due: due_key(due_date.as_deref(), due_time.as_deref()),
        done: false,
    };
    let pairing_id = insert_pairing(id, &state, &mut tx).await?;
    tx.commit().await?;
    Ok((id, pairing_id))
}

/// Whether Badger's side differs from the last-synced snapshot.
pub fn badger_edited(now: &Snap, synced: &Snap) -> bool {
    now != synced
}

/// Deletes a task the phone deleted (its reminders cascade away) and its pairing, in one
/// transaction. Conditional on the task still matching the last-synced snapshot `synced`
/// and still being synced: when it was edited in Badger meanwhile, or made Mac-only or
/// repeating, nothing is deleted and `Ok(false)` comes back (Badger wins; the caller
/// re-pushes or unpairs).
pub async fn delete_and_unpair(
    badger_id: i64,
    pairing_id: i64,
    synced: &Snap,
    pool: &Pool<Sqlite>,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let now = load_task_in(badger_id, &mut tx).await?;
    if now.is_some_and(|t| t.off_phone() || badger_edited(&t.snap(), synced)) {
        return Ok(false);
    }
    sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(badger_id)
        .execute(&mut *tx)
        .await?;
    delete_pairing(pairing_id, &mut *tx).await?;
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;
    use crate::commands::issues::inner_resolve_issue;
    use crate::commands::projects::inner_create_project;
    use crate::commands::tasks::{inner_create_task, inner_get_task, inner_update_task, TaskInput};

    fn now() -> NaiveDateTime {
        parse_timestamp("2026-11-19T12:00").unwrap()
    }

    fn input(title: &str) -> TaskInput {
        TaskInput {
            title: title.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn due_keys_round_trip_between_columns_snapshots_and_the_phone() {
        assert_eq!(
            due_key(Some("2026-11-20"), None).as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(
            due_key(Some("2026-11-20"), Some("09:30")).as_deref(),
            Some("2026-11-20T09:30")
        );
        assert_eq!(due_key(None, None), None);
        assert_eq!(
            split_due(Some("2026-11-20T09:30")),
            (Some("2026-11-20".into()), Some("09:30".into()))
        );
        assert_eq!(
            split_due(Some("2026-11-20")),
            (Some("2026-11-20".into()), None)
        );
        let d = NaiveDate::from_ymd_opt(2026, 11, 20).unwrap();
        let t = d.and_hms_opt(9, 30, 45).unwrap();
        assert_eq!(
            phone_due_key(Some(PhoneDue::Date(d))).as_deref(),
            Some("2026-11-20")
        );
        assert_eq!(
            phone_due_key(Some(PhoneDue::DateTime(t))).as_deref(),
            Some("2026-11-20T09:30"),
            "seconds dropped"
        );
        assert_eq!(to_phone_due(Some("2026-11-20")), Some(PhoneDue::Date(d)));
        assert_eq!(
            to_phone_due(Some("2026-11-20T09:30")),
            Some(PhoneDue::DateTime(d.and_hms_opt(9, 30, 0).unwrap()))
        );
    }

    #[tokio::test]
    async fn a_conditional_write_loses_to_a_concurrent_user_edit() {
        let pool = crate::test_db().await;
        let c = inner_create_customer("Acme", &pool).await.unwrap();
        let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
        let issue = inner_resolve_issue(p.id, "Bug", &pool).await.unwrap();
        let i = TaskInput {
            issue_id: Some(issue),
            ..input("Call")
        };
        let t = inner_create_task(&i, &[], now(), &pool).await.unwrap();
        let loaded = load_task(t.id, &pool).await.unwrap().unwrap();

        // The user renames it after the pass read it.
        let renamed = TaskInput {
            title: "Call Anna".into(),
            ..i.clone()
        };
        inner_update_task(t.id, &renamed, now(), &pool)
            .await
            .unwrap();

        let mut target = loaded.snap();
        target.title = "Call Bob".to_string();
        target.notes = Some("from phone".to_string());
        assert!(!apply_task(&loaded, &target, now(), &pool).await.unwrap());
        let now_t = inner_get_task(t.id, &pool).await.unwrap();
        assert_eq!(now_t.title, "Call Anna", "the user's edit stays");
        assert_eq!(now_t.notes, None, "the whole item rolled back");
        assert_eq!(now_t.issue_id, Some(issue));
    }

    #[tokio::test]
    async fn a_phone_delete_loses_to_a_badger_edit_or_mac_only_made_meanwhile() {
        let pool = crate::test_db().await;
        let t = inner_create_task(&input("Call"), &[], now(), &pool)
            .await
            .unwrap();
        let synced = load_task(t.id, &pool).await.unwrap().unwrap().snap();
        let state = PairingState {
            ek_id: "ek-1".to_string(),
            external_id: None,
            badger_snap: synced.clone(),
            phone_snap: synced.clone(),
            phone_alarms: vec![],
        };
        let mut conn = pool.acquire().await.unwrap();
        let pairing = insert_pairing(t.id, &state, &mut conn).await.unwrap();
        drop(conn);

        inner_update_task(t.id, &input("Call Anna"), now(), &pool)
            .await
            .unwrap();
        assert!(!delete_and_unpair(t.id, pairing, &synced, &pool)
            .await
            .unwrap());
        assert_eq!(
            inner_get_task(t.id, &pool).await.unwrap().title,
            "Call Anna"
        );
        assert_eq!(load_pairings(&pool).await.unwrap().len(), 1, "still paired");

        let synced = load_task(t.id, &pool).await.unwrap().unwrap().snap();
        let mac_only = TaskInput {
            mac_only: Some(true),
            ..input("Call Anna")
        };
        inner_update_task(t.id, &mac_only, now(), &pool)
            .await
            .unwrap();
        assert!(!delete_and_unpair(t.id, pairing, &synced, &pool)
            .await
            .unwrap());

        inner_update_task(
            t.id,
            &TaskInput {
                mac_only: Some(false),
                ..input("Call Anna")
            },
            now(),
            &pool,
        )
        .await
        .unwrap();
        assert!(delete_and_unpair(t.id, pairing, &synced, &pool)
            .await
            .unwrap());
        assert!(inner_get_task(t.id, &pool).await.is_err());
        assert!(load_pairings(&pool).await.unwrap().is_empty());
    }

    #[test]
    fn titles_are_trimmed_and_cut_to_500_chars() {
        assert_eq!(cut_title("  "), None);
        assert_eq!(cut_title(" Ring 📞 "), Some("Ring 📞".to_string()));
        let long = "é".repeat(600);
        assert_eq!(cut_title(&long).unwrap().chars().count(), 500);
    }

    #[test]
    fn wanted_alarms_keep_future_times_and_only_past_ones_already_on_the_phone() {
        let at = |s: &str| parse_timestamp(s).unwrap();
        let task = BTask {
            id: 1,
            title: "x".into(),
            notes: None,
            done: false,
            completed_at: None,
            due_date: None,
            due_time: None,
            repeat: "none".into(),
            mac_only: false,
            alarm_times: vec![
                at("2026-11-19T13:00"),
                at("2026-11-19T08:00"),
                at("2026-11-19T09:00"),
                at("2026-11-19T13:00"),
            ],
        };
        assert_eq!(
            task.wanted_alarms(&[], now()),
            vec![at("2026-11-19T13:00")],
            "a past time is never newly pushed"
        );
        assert_eq!(
            task.wanted_alarms(&[at("2026-11-19T09:00"), at("2026-11-18T07:00")], now()),
            vec![at("2026-11-19T09:00"), at("2026-11-19T13:00")],
            "a past one the phone has stays; one Badger doesn't have goes"
        );
    }
}
