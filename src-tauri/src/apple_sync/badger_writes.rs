//! The sync's database side: loading the rows in scope, conditional Badger writes, imports
//! and the pairing table.
//!
//! Writes are limited to the synced columns and conditional on the value the pass read
//! (`WHERE Title IS ?`), never whole-row rewrites, so a concurrent user edit and the
//! non-synced fields (IssueId, Position, links) are never clobbered. A lost race returns
//! `Ok(false)` and rolls back; the item is left for the next pass.

use chrono::{NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};

use crate::commands::reminders::{
    anchor_of, format_time_of_day, format_timestamp, local_exists, local_now, parse_timestamp,
    resolve_gap, MAX_TEXT_CHARS,
};
use crate::commands::todos::{clean_due_date, clean_notes, clean_title, move_todo_in};
use crate::AppError;

use super::ListKind;

/// A huge index: "end of the column" for `move_todo_in` (it clamps).
const END_OF_COLUMN: i64 = i64::MAX;

/// The synced fields of one side, in Badger terms, as JSON in the pairing table.
/// Todo `due` is `YYYY-MM-DD`; reminder `due` is a stored timestamp. A reminder's `notes`
/// is the Badger-owned link line ("Todo: …") and only tracked on the Badger side.
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

// ── Loading ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct BTodo {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub status: String,
    pub due_date: Option<String>,
}

impl BTodo {
    pub fn in_scope(&self) -> bool {
        self.status != "done"
    }

    pub fn snap(&self) -> Snap {
        Snap {
            title: self.title.clone(),
            notes: self.notes.clone(),
            due: self.due_date.clone(),
            done: self.status == "done",
        }
    }
}

#[derive(Debug, Clone, PartialEq, FromRow)]
pub struct BReminder {
    pub id: i64,
    pub text: String,
    pub remind_at: String,
    pub repeat: String,
    pub state: String,
    pub todo_title: Option<String>,
    pub note_title: Option<String>,
}

impl BReminder {
    /// One-off and not done: pushed when unpaired.
    pub fn in_scope(&self) -> bool {
        self.repeat == "none" && self.state != "done"
    }

    /// The line Badger writes into the phone item's notes; `None` when unlinked.
    pub fn link_note(&self) -> Option<String> {
        if let Some(t) = &self.todo_title {
            Some(format!("Todo: {t}"))
        } else {
            self.note_title.as_ref().map(|t| format!("Note: {t}"))
        }
    }

    pub fn snap(&self) -> Snap {
        Snap {
            title: self.text.clone(),
            notes: self.link_note(),
            due: Some(normalize_timestamp(&self.remind_at)),
            done: self.state == "done",
        }
    }
}

/// A stored timestamp in the canonical storage format, whole seconds (unparseable: as is).
pub fn normalize_timestamp(s: &str) -> String {
    parse_timestamp(s)
        .ok()
        .and_then(|t| t.with_nanosecond(0))
        .map(format_timestamp)
        .unwrap_or_else(|| s.to_string())
}

const TODO_COLUMNS: &str = "SELECT id, Title AS title, Notes AS notes, Status AS status, \
     DueDate AS due_date FROM todos";

const REMINDER_COLUMNS: &str = "SELECT r.id, r.Text AS text, r.RemindAt AS remind_at, \
     r.Repeat AS repeat, r.State AS state, t.Title AS todo_title, n.Title AS note_title \
     FROM reminders r LEFT JOIN todos t ON t.id = r.TodoId LEFT JOIN notes n ON n.id = r.NoteId";

/// Open todos plus every paired todo (done ones included, so they aren't taken for deleted).
pub async fn load_todos(pool: &Pool<Sqlite>) -> Result<Vec<BTodo>, AppError> {
    let sql = format!(
        "{TODO_COLUMNS} WHERE Status IN ('not_started', 'in_progress') \
         OR id IN (SELECT BadgerId FROM apple_sync_items WHERE Kind = 'todo') ORDER BY id"
    );
    Ok(sqlx::query_as::<_, BTodo>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?)
}

pub async fn load_todo(id: i64, pool: &Pool<Sqlite>) -> Result<Option<BTodo>, AppError> {
    let sql = format!("{TODO_COLUMNS} WHERE id = ?");
    Ok(sqlx::query_as::<_, BTodo>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

/// Pending/fired one-offs plus every paired reminder.
pub async fn load_reminders(pool: &Pool<Sqlite>) -> Result<Vec<BReminder>, AppError> {
    let sql = format!(
        "{REMINDER_COLUMNS} WHERE (r.Repeat = 'none' AND r.State IN ('pending', 'fired')) \
         OR r.id IN (SELECT BadgerId FROM apple_sync_items WHERE Kind = 'reminder') ORDER BY r.id"
    );
    Ok(sqlx::query_as::<_, BReminder>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?)
}

pub async fn load_reminder(id: i64, pool: &Pool<Sqlite>) -> Result<Option<BReminder>, AppError> {
    let sql = format!("{REMINDER_COLUMNS} WHERE r.id = ?");
    Ok(sqlx::query_as::<_, BReminder>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(pool)
        .await?)
}

// ── Pairings ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub struct Pairing {
    pub id: i64,
    pub kind: ListKind,
    pub badger_id: i64,
    pub ek_id: String,
    pub external_id: Option<String>,
    pub badger_snap: Snap,
    pub phone_snap: Snap,
    pub phone_due: Option<String>,
    /// Consecutive passes the phone item was missing from its list.
    pub missed: i64,
}

#[derive(FromRow)]
struct PairingRow {
    id: i64,
    kind: String,
    badger_id: i64,
    ek_id: String,
    external_id: Option<String>,
    badger_snap: String,
    phone_snap: String,
    phone_due: Option<String>,
    missed: i64,
}

pub async fn load_pairings(pool: &Pool<Sqlite>) -> Result<Vec<Pairing>, AppError> {
    let rows = sqlx::query_as::<_, PairingRow>(
        "SELECT id, Kind AS kind, BadgerId AS badger_id, EkId AS ek_id, \
             ExternalId AS external_id, BadgerSnap AS badger_snap, PhoneSnap AS phone_snap, \
             PhoneDue AS phone_due, MissedPasses AS missed \
         FROM apple_sync_items ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| Pairing {
            id: r.id,
            kind: if r.kind == "reminder" {
                ListKind::Reminders
            } else {
                ListKind::Todos
            },
            badger_id: r.badger_id,
            ek_id: r.ek_id,
            external_id: r.external_id,
            badger_snap: Snap::from_json(&r.badger_snap),
            phone_snap: Snap::from_json(&r.phone_snap),
            phone_due: r.phone_due,
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
    pub phone_due: Option<String>,
}

/// Returns the new pairing's id.
pub async fn insert_pairing(
    kind: ListKind,
    badger_id: i64,
    state: &PairingState,
    conn: &mut SqliteConnection,
) -> Result<i64, AppError> {
    let result = sqlx::query(
        "INSERT INTO apple_sync_items \
             (Kind, BadgerId, EkId, ExternalId, BadgerSnap, PhoneSnap, PhoneDue, SyncedAt) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(kind.kind())
    .bind(badger_id)
    .bind(&state.ek_id)
    .bind(&state.external_id)
    .bind(state.badger_snap.to_json())
    .bind(state.phone_snap.to_json())
    .bind(&state.phone_due)
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
             PhoneDue = ?, SyncedAt = ?, MissedPasses = 0 \
         WHERE id = ?",
    )
    .bind(&state.ek_id)
    .bind(&state.external_id)
    .bind(state.badger_snap.to_json())
    .bind(state.phone_snap.to_json())
    .bind(&state.phone_due)
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

/// Drops every pairing of one kind ("Recreate lists").
pub async fn delete_pairings_of(kind: ListKind, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    sqlx::query("DELETE FROM apple_sync_items WHERE Kind = ?")
        .bind(kind.kind())
        .execute(pool)
        .await?;
    Ok(())
}

// ── Conditional writes ────────────────────────────────────────────────────────

/// Writes `target`'s todo fields that differ from `loaded`, each conditional on the loaded
/// value. Done ↔ open moves to the end of `done` / `not_started` (done completes the todo's
/// reminders, as on the board). `Ok(false)`: the row changed meanwhile, nothing written.
pub async fn apply_todo(
    loaded: &BTodo,
    target: &Snap,
    pool: &Pool<Sqlite>,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let current = loaded.snap();
    if target.title != current.title {
        let title = clean_title(&target.title)?;
        if !update_if(&mut tx, "todos", "Title", &title, &loaded.title, loaded.id).await? {
            return Ok(false);
        }
    }
    if target.notes != current.notes {
        let notes = clean_notes(target.notes.as_deref());
        if !update_opt_if(&mut tx, "todos", "Notes", &notes, &loaded.notes, loaded.id).await? {
            return Ok(false);
        }
    }
    if target.due != current.due {
        let due = clean_due_date(target.due.as_deref())?;
        if !update_opt_if(
            &mut tx,
            "todos",
            "DueDate",
            &due,
            &loaded.due_date,
            loaded.id,
        )
        .await?
        {
            return Ok(false);
        }
    }
    if target.done != current.done {
        let (status,): (String,) = match sqlx::query_as("SELECT Status FROM todos WHERE id = ?")
            .bind(loaded.id)
            .fetch_optional(&mut *tx)
            .await?
        {
            Some(row) => row,
            None => return Ok(false),
        };
        if status != loaded.status {
            return Ok(false);
        }
        let to = if target.done { "done" } else { "not_started" };
        move_todo_in(loaded.id, to, END_OF_COLUMN, &mut tx).await?;
    }
    tx.commit().await?;
    Ok(true)
}

/// Writes `target`'s reminder fields (title, time, done) that differ from `loaded`. A new
/// time is a reschedule: pending, or fired when it has already passed (like the
/// scheduler's catch-up). Only open one-offs are written.
pub async fn apply_reminder(
    loaded: &BReminder,
    target: &Snap,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<bool, AppError> {
    let current = loaded.snap();
    let stamp = format_timestamp(local_now());
    let mut tx = pool.begin().await?;
    if target.title != current.title {
        let text = clean_reminder_text(&target.title)
            .ok_or_else(|| AppError::Custom("Text is required".to_string()))?;
        let done = sqlx::query(
            "UPDATE reminders SET Text = ?, UpdatedAt = ? \
             WHERE id = ? AND Text = ? AND State != 'done' AND Repeat = 'none'",
        )
        .bind(&text)
        .bind(&stamp)
        .bind(loaded.id)
        .bind(&loaded.text)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
    }
    if target.due != current.due {
        let Some(due) = target.due.as_deref() else {
            return Ok(false);
        };
        let at = resolve_gap(parse_timestamp(due)?, local_exists);
        let state = if at <= now { "fired" } else { "pending" };
        let done = sqlx::query(
            "UPDATE reminders SET RemindAt = ?, TimeOfDay = ?, State = ?, UpdatedAt = ? \
             WHERE id = ? AND RemindAt = ? AND State != 'done' AND Repeat = 'none'",
        )
        .bind(format_timestamp(at))
        .bind(format_time_of_day(anchor_of(at)))
        .bind(state)
        .bind(&stamp)
        .bind(loaded.id)
        .bind(&loaded.remind_at)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
    }
    if target.done && !current.done {
        let done = sqlx::query(
            "UPDATE reminders SET State = 'done', UpdatedAt = ? \
             WHERE id = ? AND State IN ('pending', 'fired') AND Repeat = 'none'",
        )
        .bind(&stamp)
        .bind(loaded.id)
        .execute(&mut *tx)
        .await?;
        if done.rows_affected() == 0 {
            return Ok(false);
        }
    }
    tx.commit().await?;
    Ok(true)
}

async fn update_if(
    tx: &mut SqliteConnection,
    table: &str,
    column: &str,
    value: &str,
    expected: &str,
    id: i64,
) -> Result<bool, AppError> {
    let sql = format!("UPDATE {table} SET {column} = ? WHERE id = ? AND {column} IS ?");
    let result = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(value)
        .bind(id)
        .bind(expected)
        .execute(tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

async fn update_opt_if(
    tx: &mut SqliteConnection,
    table: &str,
    column: &str,
    value: &Option<String>,
    expected: &Option<String>,
    id: i64,
) -> Result<bool, AppError> {
    let sql = format!("UPDATE {table} SET {column} = ? WHERE id = ? AND {column} IS ?");
    let result = sqlx::query(sqlx::AssertSqlSafe(sql))
        .bind(value)
        .bind(id)
        .bind(expected)
        .execute(tx)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Reminder text from a phone title: trimmed and cut to 500 characters; `None` when empty.
pub fn clean_reminder_text(title: &str) -> Option<String> {
    let cut: String = title.trim().chars().take(MAX_TEXT_CHARS).collect();
    let cut = cut.trim_end().to_string();
    (!cut.is_empty()).then_some(cut)
}

// ── Imports and deletes ───────────────────────────────────────────────────────

/// A phone-created todo: `not_started`, at the end of that column, paired in the same
/// transaction. Returns its id and the pairing's.
pub async fn import_todo(
    title: &str,
    notes: Option<&str>,
    due_date: Option<&str>,
    pairing: &PairingState,
    pool: &Pool<Sqlite>,
) -> Result<(i64, i64), AppError> {
    let title = clean_title(title)?;
    let due_date = clean_due_date(due_date)?;
    let created_at = format_timestamp(local_now());
    let mut tx = pool.begin().await?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO todos (Title, Notes, Status, Position, DueDate, CreatedAt) \
         VALUES (?, ?, 'not_started', \
             (SELECT COALESCE(MAX(Position), -1) + 1 FROM todos WHERE Status = 'not_started'), ?, ?) \
         RETURNING id",
    )
    .bind(&title)
    .bind(clean_notes(notes))
    .bind(&due_date)
    .bind(&created_at)
    .fetch_one(&mut *tx)
    .await?;
    let mut state = pairing.clone();
    state.badger_snap = Snap {
        title,
        notes: clean_notes(notes),
        due: due_date,
        done: false,
    };
    let pairing_id = insert_pairing(ListKind::Todos, id, &state, &mut tx).await?;
    tx.commit().await?;
    Ok((id, pairing_id))
}

/// A phone-created reminder: a one-off at `at` (already resolved out of a DST gap), fired
/// when it has passed, paired in the same transaction. Returns its id and the pairing's.
pub async fn import_reminder(
    text: &str,
    at: NaiveDateTime,
    now: NaiveDateTime,
    pairing: &PairingState,
    pool: &Pool<Sqlite>,
) -> Result<(i64, i64), AppError> {
    let text = clean_reminder_text(text)
        .ok_or_else(|| AppError::Custom("Text is required".to_string()))?;
    let state = if at <= now { "fired" } else { "pending" };
    let stamp = format_timestamp(local_now());
    let mut tx = pool.begin().await?;
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO reminders (Text, RemindAt, TimeOfDay, Repeat, Weekdays, MonthDay, State, \
             TodoId, NoteId, CreatedAt, UpdatedAt) \
         VALUES (?, ?, ?, 'none', '', NULL, ?, NULL, NULL, ?, ?) RETURNING id",
    )
    .bind(&text)
    .bind(format_timestamp(at))
    .bind(format_time_of_day(anchor_of(at)))
    .bind(state)
    .bind(&stamp)
    .bind(&stamp)
    .fetch_one(&mut *tx)
    .await?;
    let mut pairing = pairing.clone();
    pairing.badger_snap = Snap {
        title: text,
        notes: None,
        due: Some(format_timestamp(at)),
        done: false,
    };
    let pairing_id = insert_pairing(ListKind::Reminders, id, &pairing, &mut tx).await?;
    tx.commit().await?;
    Ok((id, pairing_id))
}

/// Whether Badger's side differs from the last-synced snapshot in a field the user edits.
/// A reminder's notes are the derived link line (its todo's or note's title), not an edit
/// of the reminder.
pub fn badger_edited(kind: ListKind, now: &Snap, synced: &Snap) -> bool {
    match kind {
        ListKind::Todos => now != synced,
        ListKind::Reminders => {
            now.title != synced.title || now.due != synced.due || now.done != synced.done
        }
    }
}

/// Deletes a Badger row the phone deleted, and its pairing, in one transaction. A todo's
/// reminders cascade away, as when it's deleted on the board. Conditional on the row still
/// matching the last-synced snapshot `synced`: when it was edited in Badger meanwhile,
/// nothing is deleted and `Ok(false)` comes back (Badger wins; the caller re-pushes).
pub async fn delete_and_unpair(
    kind: ListKind,
    badger_id: i64,
    pairing_id: i64,
    synced: &Snap,
    pool: &Pool<Sqlite>,
) -> Result<bool, AppError> {
    let mut tx = pool.begin().await?;
    let now = match kind {
        ListKind::Todos => {
            let sql = format!("{TODO_COLUMNS} WHERE id = ?");
            sqlx::query_as::<_, BTodo>(sqlx::AssertSqlSafe(sql))
                .bind(badger_id)
                .fetch_optional(&mut *tx)
                .await?
                .map(|t| t.snap())
        }
        ListKind::Reminders => {
            let sql = format!("{REMINDER_COLUMNS} WHERE r.id = ?");
            sqlx::query_as::<_, BReminder>(sqlx::AssertSqlSafe(sql))
                .bind(badger_id)
                .fetch_optional(&mut *tx)
                .await?
                .map(|r| r.snap())
        }
    };
    if now.is_some_and(|now| badger_edited(kind, &now, synced)) {
        return Ok(false);
    }
    let sql = match kind {
        ListKind::Todos => "DELETE FROM todos WHERE id = ?",
        ListKind::Reminders => "DELETE FROM reminders WHERE id = ?",
    };
    sqlx::query(sql).bind(badger_id).execute(&mut *tx).await?;
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
    use crate::commands::todos::{inner_create_todo, inner_get_todo, inner_update_todo};

    #[tokio::test]
    async fn conditional_todo_write_loses_to_a_concurrent_user_edit() {
        let pool = crate::test_db().await;
        let c = inner_create_customer("Acme", &pool).await.unwrap();
        let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
        let issue = inner_resolve_issue(p.id, "Bug", &pool).await.unwrap();
        inner_create_todo("Other", None, None, "not_started", None, &pool)
            .await
            .unwrap();
        let t = inner_create_todo("Call", None, None, "not_started", Some(issue), &pool)
            .await
            .unwrap();
        let loaded = load_todo(t.id, &pool).await.unwrap().unwrap();

        // The user renames it after the pass read it.
        inner_update_todo(t.id, "Call Anna", None, None, Some(issue), &pool)
            .await
            .unwrap();

        let mut target = loaded.snap();
        target.title = "Call Bob".to_string();
        target.notes = Some("from phone".to_string());
        assert!(!apply_todo(&loaded, &target, &pool).await.unwrap());

        let now = inner_get_todo(t.id, &pool).await.unwrap();
        assert_eq!(now.title, "Call Anna", "the user's edit stays");
        assert_eq!(now.notes, None, "the whole item rolled back");
        assert_eq!(now.issue_id, Some(issue));
        assert_eq!(now.position, 1);
    }

    #[tokio::test]
    async fn todo_write_touches_only_the_synced_columns() {
        let pool = crate::test_db().await;
        let c = inner_create_customer("Acme", &pool).await.unwrap();
        let p = inner_create_project(c.id, "Web", &pool).await.unwrap();
        let issue = inner_resolve_issue(p.id, "Bug", &pool).await.unwrap();
        inner_create_todo("First", None, None, "in_progress", None, &pool)
            .await
            .unwrap();
        let t = inner_create_todo("Call", None, None, "in_progress", Some(issue), &pool)
            .await
            .unwrap();
        let loaded = load_todo(t.id, &pool).await.unwrap().unwrap();
        let mut target = loaded.snap();
        target.title = "  Call Bob ".to_string();
        target.due = Some("2026-11-20".to_string());
        assert!(apply_todo(&loaded, &target, &pool).await.unwrap());

        let now = inner_get_todo(t.id, &pool).await.unwrap();
        assert_eq!(now.title, "Call Bob");
        assert_eq!(now.due_date.as_deref(), Some("2026-11-20"));
        assert_eq!(now.status, "in_progress");
        assert_eq!((now.issue_id, now.position), (Some(issue), 1));
    }

    #[tokio::test]
    async fn a_phone_delete_loses_to_a_badger_edit_made_meanwhile() {
        let pool = crate::test_db().await;
        let t = inner_create_todo("Call", None, None, "not_started", None, &pool)
            .await
            .unwrap();
        let synced = load_todo(t.id, &pool).await.unwrap().unwrap().snap();
        let state = PairingState {
            ek_id: "ek-1".to_string(),
            external_id: None,
            badger_snap: synced.clone(),
            phone_snap: synced.clone(),
            phone_due: None,
        };
        let mut conn = pool.acquire().await.unwrap();
        let pairing = insert_pairing(ListKind::Todos, t.id, &state, &mut conn)
            .await
            .unwrap();
        drop(conn);

        // The user edits it after the pass decided to delete it.
        inner_update_todo(t.id, "Call Anna", None, None, None, &pool)
            .await
            .unwrap();
        let deleted = delete_and_unpair(ListKind::Todos, t.id, pairing, &synced, &pool)
            .await
            .unwrap();
        assert!(!deleted);
        assert_eq!(
            inner_get_todo(t.id, &pool).await.unwrap().title,
            "Call Anna"
        );
        assert_eq!(load_pairings(&pool).await.unwrap().len(), 1, "still paired");

        // Unchanged since the snapshot: deleted with its pairing.
        let synced = load_todo(t.id, &pool).await.unwrap().unwrap().snap();
        assert!(
            delete_and_unpair(ListKind::Todos, t.id, pairing, &synced, &pool)
                .await
                .unwrap()
        );
        assert!(inner_get_todo(t.id, &pool).await.is_err());
        assert!(load_pairings(&pool).await.unwrap().is_empty());
    }

    #[test]
    fn reminder_text_is_trimmed_and_cut_to_500_chars() {
        assert_eq!(clean_reminder_text("  "), None);
        assert_eq!(
            clean_reminder_text(" Ring 📞 "),
            Some("Ring 📞".to_string())
        );
        let long = "é".repeat(600);
        assert_eq!(clean_reminder_text(&long).unwrap().chars().count(), 500);
    }
}
