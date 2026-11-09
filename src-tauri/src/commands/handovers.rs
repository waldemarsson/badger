//! Handovers: one end-of-day markdown note per local date, shown on Today the next working
//! day. They live in their own table, so they never reach the notes search index, the link
//! index or the semantic index. Dates come from the frontend (local `today()`); Rust only
//! checks that they are real `YYYY-MM-DD` dates.
use crate::commands::notes::{self, Note};
use crate::semantic::SemanticState;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteExecutor};
use tauri::State;

/// One day's handover. `dismissed_at` is set once its incoming card was dismissed.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Handover {
    pub date: String,
    pub body: String,
    pub dismissed_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

const SELECT_HANDOVERS: &str = "SELECT Date AS date, Body AS body, DismissedAt AS dismissed_at, \
     CreatedAt AS created_at, UpdatedAt AS updated_at FROM handovers";

fn now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

fn not_found() -> AppError {
    AppError::Custom("Handover not found".to_string())
}

/// True for a real calendar date written exactly as `YYYY-MM-DD` (no `2026-1-5`, no
/// `2026-02-30`).
pub(crate) fn is_valid_date(date: &str) -> bool {
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .is_ok_and(|d| d.format("%Y-%m-%d").to_string() == date)
}

fn check_date(date: &str) -> Result<(), AppError> {
    if is_valid_date(date) {
        Ok(())
    } else {
        Err(AppError::Custom("Invalid date".to_string()))
    }
}

async fn fetch(
    date: &str,
    executor: impl SqliteExecutor<'_>,
) -> Result<Option<Handover>, AppError> {
    let sql = format!("{SELECT_HANDOVERS} WHERE Date = ?");
    Ok(sqlx::query_as::<_, Handover>(&sql)
        .bind(date)
        .fetch_optional(executor)
        .await?)
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// The handover written on `date`, or None when there is none.
pub async fn inner_get_handover(
    date: &str,
    pool: &Pool<Sqlite>,
) -> Result<Option<Handover>, AppError> {
    check_date(date)?;
    fetch(date, pool).await
}

/// Every handover, newest date first (dismissed ones included).
pub async fn inner_list_handovers(pool: &Pool<Sqlite>) -> Result<Vec<Handover>, AppError> {
    let sql = format!("{SELECT_HANDOVERS} ORDER BY Date DESC");
    Ok(sqlx::query_as::<_, Handover>(&sql).fetch_all(pool).await?)
}

/// Stores `body` as the handover for `date`, creating it on first save. An empty or
/// whitespace-only body deletes it (returns None). An identical body is a no-op that keeps
/// `UpdatedAt`; `DismissedAt` is never touched. No length limit, like note bodies.
pub async fn inner_save_handover(
    date: &str,
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<Option<Handover>, AppError> {
    check_date(date)?;
    if body.trim().is_empty() {
        sqlx::query("DELETE FROM handovers WHERE Date = ?")
            .bind(date)
            .execute(pool)
            .await?;
        return Ok(None);
    }
    let ts = now();
    sqlx::query(
        "INSERT INTO handovers (Date, Body, CreatedAt, UpdatedAt) VALUES (?, ?, ?, ?)
         ON CONFLICT(Date) DO UPDATE SET Body = excluded.Body, UpdatedAt = excluded.UpdatedAt
         WHERE Body IS NOT excluded.Body",
    )
    .bind(date)
    .bind(body)
    .bind(&ts)
    .bind(&ts)
    .execute(pool)
    .await?;
    fetch(date, pool).await
}

/// Hides the handover's incoming card for good (it stays in history).
pub async fn inner_dismiss_handover(date: &str, pool: &Pool<Sqlite>) -> Result<Handover, AppError> {
    check_date(date)?;
    let done = sqlx::query("UPDATE handovers SET DismissedAt = ? WHERE Date = ?")
        .bind(now())
        .bind(date)
        .execute(pool)
        .await?;
    if done.rows_affected() == 0 {
        return Err(not_found());
    }
    fetch(date, pool).await?.ok_or_else(not_found)
}

/// The note title a handover is saved under.
pub fn note_title(date: &str) -> String {
    format!("Handover {date}")
}

/// Copies the stored handover for `date` into a root note titled `Handover YYYY-MM-DD`.
/// The handover stays. The title follows the notes rules (unique case-insensitively).
pub async fn inner_save_handover_as_note(
    date: &str,
    pool: &Pool<Sqlite>,
) -> Result<Note, AppError> {
    check_date(date)?;
    let mut tx = pool.begin().await?;
    let handover = fetch(date, &mut *tx).await?.ok_or_else(not_found)?;
    let id = notes::insert_titled_note(&note_title(date), &handover.body, None, &mut tx).await?;
    tx.commit().await?;
    notes::inner_get_note(id, pool).await
}

/// `inner_save_handover_as_note`, then queues the new note for the semantic index.
pub async fn save_handover_as_note_and_queue(
    date: &str,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<Note, AppError> {
    let note = inner_save_handover_as_note(date, pool).await?;
    semantic.enqueue([note.id]);
    Ok(note)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_handover(
    date: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Option<Handover>, AppError> {
    inner_get_handover(&date, pool.inner()).await
}

#[tauri::command]
pub async fn list_handovers(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<Handover>, AppError> {
    inner_list_handovers(pool.inner()).await
}

#[tauri::command]
pub async fn save_handover(
    date: String,
    body: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Option<Handover>, AppError> {
    inner_save_handover(&date, &body, pool.inner()).await
}

#[tauri::command]
pub async fn dismiss_handover(
    date: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Handover, AppError> {
    inner_dismiss_handover(&date, pool.inner()).await
}

#[tauri::command]
pub async fn save_handover_as_note(
    date: String,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<Note, AppError> {
    save_handover_as_note_and_queue(&date, pool.inner(), semantic.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn count(pool: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM handovers")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn invalid_and_impossible_dates_are_rejected_everywhere() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        let bad = [
            "2026-13-01",
            "2026-02-30",
            "2026-00-10",
            "2026-1-5",
            "26-01-05",
            "2026/01/05",
            "",
            "today",
            "2026-01-05T00:00",
        ];
        for date in bad {
            let msg = |r: AppError| r.to_string();
            assert_eq!(
                msg(inner_get_handover(date, &pool).await.unwrap_err()),
                "Invalid date",
                "{date:?}"
            );
            assert_eq!(
                msg(inner_save_handover(date, "x", &pool).await.unwrap_err()),
                "Invalid date"
            );
            assert_eq!(
                msg(inner_save_handover(date, "", &pool).await.unwrap_err()),
                "Invalid date"
            );
            assert_eq!(
                msg(inner_dismiss_handover(date, &pool).await.unwrap_err()),
                "Invalid date"
            );
            assert_eq!(
                msg(save_handover_as_note_and_queue(date, &pool, &state)
                    .await
                    .unwrap_err()),
                "Invalid date"
            );
        }
        assert_eq!(count(&pool).await, 0);
        assert!(is_valid_date("2028-02-29"), "leap day");
        assert!(!is_valid_date("2026-02-29"));
    }

    #[tokio::test]
    async fn save_then_get_round_trips_and_unchanged_keeps_updated_at() {
        let pool = crate::test_db().await;
        assert_eq!(inner_get_handover("2026-11-05", &pool).await.unwrap(), None);
        let body = "# Tomorrow\r\n- [ ] ring Åsa 📞\n";
        let saved = inner_save_handover("2026-11-05", body, &pool)
            .await
            .unwrap()
            .expect("stored");
        assert_eq!(saved.date, "2026-11-05");
        assert_eq!(saved.body, body);
        assert_eq!(saved.dismissed_at, None);
        assert_eq!(
            inner_get_handover("2026-11-05", &pool).await.unwrap(),
            Some(saved.clone())
        );

        sqlx::query("UPDATE handovers SET UpdatedAt = 'old'")
            .execute(&pool)
            .await
            .unwrap();
        let same = inner_save_handover("2026-11-05", body, &pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(same.updated_at, "old", "no-op write");

        let edited = inner_save_handover("2026-11-05", "changed", &pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(edited.body, "changed");
        assert_ne!(edited.updated_at, "old");
        assert_eq!(edited.created_at, saved.created_at, "created stays");
    }

    #[tokio::test]
    async fn empty_or_whitespace_body_deletes_the_row() {
        let pool = crate::test_db().await;
        for blank in ["", "  \n\t \r\n"] {
            inner_save_handover("2026-11-05", "keep", &pool)
                .await
                .unwrap();
            inner_save_handover("2026-11-06", "other day", &pool)
                .await
                .unwrap();
            assert_eq!(count(&pool).await, 2);
            let cleared = inner_save_handover("2026-11-05", blank, &pool)
                .await
                .unwrap();
            assert_eq!(cleared, None);
            assert_eq!(inner_get_handover("2026-11-05", &pool).await.unwrap(), None);
            assert_eq!(count(&pool).await, 1, "only that day's row goes");
        }
        // Clearing a day that has none is fine.
        assert_eq!(
            inner_save_handover("2026-11-07", "", &pool).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn list_is_newest_first_and_includes_dismissed() {
        let pool = crate::test_db().await;
        for date in ["2026-11-03", "2026-12-01", "2025-12-31", "2026-11-06"] {
            inner_save_handover(date, &format!("from {date}"), &pool)
                .await
                .unwrap();
        }
        inner_dismiss_handover("2026-11-06", &pool).await.unwrap();
        let list = inner_list_handovers(&pool).await.unwrap();
        let dates: Vec<&str> = list.iter().map(|h| h.date.as_str()).collect();
        assert_eq!(
            dates,
            ["2026-12-01", "2026-11-06", "2026-11-03", "2025-12-31"]
        );
        assert!(list[1].dismissed_at.is_some());
        assert_eq!(list[1].body, "from 2026-11-06");
    }

    #[tokio::test]
    async fn dismiss_sets_dismissed_at_and_editing_keeps_it() {
        let pool = crate::test_db().await;
        inner_save_handover("2026-11-05", "- [ ] ship it", &pool)
            .await
            .unwrap();
        let dismissed = inner_dismiss_handover("2026-11-05", &pool).await.unwrap();
        let at = dismissed.dismissed_at.clone().expect("dismissed");

        // A checkbox ticked in history rewrites the body; it stays dismissed.
        let edited = inner_save_handover("2026-11-05", "- [x] ship it", &pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(edited.body, "- [x] ship it");
        assert_eq!(edited.dismissed_at.as_deref(), Some(at.as_str()));

        let err = inner_dismiss_handover("2026-11-04", &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Handover not found");
    }

    #[tokio::test]
    async fn saving_to_a_captured_date_is_independent_of_the_current_date() {
        let pool = crate::test_db().await;
        // The editor captured an old date (e.g. it was left open across midnight): the save
        // lands on that date, never on the machine's current one.
        let captured = "2020-01-02";
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        let saved = inner_save_handover(captured, "late edit", &pool)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.date, captured);
        assert_eq!(inner_get_handover(&today, &pool).await.unwrap(), None);
        // And a future date is stored as given too.
        inner_save_handover("2099-12-31", "far", &pool)
            .await
            .unwrap();
        let dates: Vec<String> = inner_list_handovers(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.date)
            .collect();
        assert_eq!(dates, ["2099-12-31", captured]);
    }

    #[tokio::test]
    async fn handover_text_stays_out_of_notes_search_and_link_indexes() {
        let pool = crate::test_db().await;
        let n = notes::inner_create_note_titled("Visible", None, &pool)
            .await
            .unwrap();
        inner_save_handover("2026-11-05", "zebracorn secret #handtag [[Visible]]", &pool)
            .await
            .unwrap();
        for query in ["zebracorn", "secret", "#handtag"] {
            let hits = notes::inner_search_notes(query, None, &pool).await.unwrap();
            assert!(hits.is_empty(), "{query} must not hit the handover");
        }
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refs, 0);
        assert!(notes::inner_list_backlinks(n.id, &pool)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn save_as_note_creates_an_indexed_root_note_and_keeps_the_handover() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        sqlx::raw_sql(
            "INSERT INTO folders (id, Name) VALUES (1, 'Work');
             INSERT INTO notes (FolderId, Title, CreatedAt, UpdatedAt) VALUES (1, 'Other', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        state.drain(&pool).await;
        let body = "# Quokka plan\n- [ ] see [[Other]] #ideas\n";
        inner_save_handover("2026-11-05", body, &pool)
            .await
            .unwrap();
        let before = inner_get_handover("2026-11-05", &pool).await.unwrap();

        let note = save_handover_as_note_and_queue("2026-11-05", &pool, &state)
            .await
            .unwrap();
        assert_eq!(note.title, "Handover 2026-11-05");
        assert_eq!(note.body, body);
        assert_eq!(note.folder_id, None, "lands at the root");
        assert_eq!(state.queued(), vec![note.id], "queued for embedding");

        let hits = notes::inner_search_notes("quokka", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1, "in the search index");
        assert_eq!(hits[0].note.id, note.id);
        let other: i64 = sqlx::query_scalar("SELECT id FROM notes WHERE Title = 'Other'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(
            notes::inner_list_backlinks(other, &pool)
                .await
                .unwrap()
                .len(),
            1,
            "links indexed"
        );
        assert_eq!(
            inner_get_handover("2026-11-05", &pool).await.unwrap(),
            before,
            "the handover stays"
        );
    }

    #[tokio::test]
    async fn save_as_note_with_a_taken_title_errors_and_changes_nothing() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        notes::inner_create_note_titled("handover 2026-11-05", None, &pool)
            .await
            .unwrap();
        state.drain(&pool).await;
        inner_save_handover("2026-11-05", "text", &pool)
            .await
            .unwrap();
        let err = save_handover_as_note_and_queue("2026-11-05", &pool, &state)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "A note named \"Handover 2026-11-05\" already exists"
        );
        let notes: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(notes, 1);
        assert!(state.queued().is_empty());
        assert_eq!(
            inner_get_handover("2026-11-05", &pool)
                .await
                .unwrap()
                .unwrap()
                .body,
            "text"
        );

        let err = save_handover_as_note_and_queue("2026-11-04", &pool, &state)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Handover not found");
    }
}
