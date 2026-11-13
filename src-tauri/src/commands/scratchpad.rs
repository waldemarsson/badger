//! The scratchpad: one persistent buffer for throwaway text. It lives in its own table,
//! so it never reaches the notes search index, the link index or the semantic index.
use crate::commands::notes::{self, Note};
use crate::semantic::SemanticState;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteExecutor};
use tauri::State;

/// The pad's text. `updated_at` is None until the first save.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Scratchpad {
    pub body: String,
    pub updated_at: Option<String>,
    /// `markdown` or `json`; kept when the pad is cleared or saved as a note.
    #[serde(default = "notes::default_kind")]
    pub kind: String,
}

fn now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

/// Writes the pad's body (creating the row on first save). An identical body is a no-op
/// that keeps `UpdatedAt`.
async fn store_body(body: &str, executor: impl SqliteExecutor<'_>) -> Result<(), AppError> {
    sqlx::query(
        "INSERT INTO scratchpad (id, Body, UpdatedAt) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET Body = excluded.Body, UpdatedAt = excluded.UpdatedAt
         WHERE Body IS NOT excluded.Body",
    )
    .bind(body)
    .bind(now())
    .execute(executor)
    .await?;
    Ok(())
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// The pad's stored kind (`markdown` before the row exists).
async fn stored_kind(executor: impl SqliteExecutor<'_>) -> Result<String, AppError> {
    let kind: Option<String> = sqlx::query_scalar("SELECT Kind FROM scratchpad WHERE id = 1")
        .fetch_optional(executor)
        .await?;
    Ok(kind.unwrap_or_else(notes::default_kind))
}

/// The pad; empty before its first save.
pub async fn inner_get_scratchpad(pool: &Pool<Sqlite>) -> Result<Scratchpad, AppError> {
    let pad = sqlx::query_as::<_, Scratchpad>(
        "SELECT Body AS body, UpdatedAt AS updated_at, Kind AS kind FROM scratchpad WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(pad.unwrap_or(Scratchpad {
        body: String::new(),
        updated_at: None,
        kind: notes::default_kind(),
    }))
}

/// Replaces the pad's text (an empty body clears it; the kind stays). Like note bodies:
/// no length limit in markdown, [`notes::MAX_JSON_BYTES`] in JSON.
pub async fn inner_set_scratchpad_body(
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<Scratchpad, AppError> {
    // The limit is in the statement (as in `inner_update_note_body`), so a concurrent kind
    // switch can't slip an oversized body past it. A new row is markdown: no limit. 0 rows
    // affected means unchanged or over the limit; the kind check tells them apart.
    let over_json_limit = body.len() > notes::MAX_JSON_BYTES;
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "INSERT INTO scratchpad (id, Body, UpdatedAt) VALUES (1, ?, ?)
         ON CONFLICT(id) DO UPDATE SET Body = excluded.Body, UpdatedAt = excluded.UpdatedAt
         WHERE Body IS NOT excluded.Body AND NOT (? AND Kind = 'json')",
    )
    .bind(body)
    .bind(now())
    .bind(over_json_limit)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 && over_json_limit {
        notes::check_json_size(&stored_kind(&mut *tx).await?, body)?;
    }
    tx.commit().await?;
    inner_get_scratchpad(pool).await
}

/// Switches the pad between markdown and JSON, first storing `body` (the editor's pending
/// text) when given. The text is never converted. Creates the row if the pad was never
/// written. Switching to JSON checks the size limit.
pub async fn inner_set_scratchpad_kind(
    kind: &str,
    body: Option<&str>,
    pool: &Pool<Sqlite>,
) -> Result<Scratchpad, AppError> {
    notes::check_kind(kind)?;
    let mut tx = pool.begin().await?;
    if let Some(body) = body {
        store_body(body, &mut *tx).await?;
    }
    sqlx::query(
        "INSERT INTO scratchpad (id, Body, UpdatedAt, Kind) VALUES (1, '', ?, ?)
         ON CONFLICT(id) DO UPDATE SET Kind = excluded.Kind",
    )
    .bind(now())
    .bind(kind)
    .execute(&mut *tx)
    .await?;
    let bytes: i64 =
        sqlx::query_scalar("SELECT length(CAST(Body AS BLOB)) FROM scratchpad WHERE id = 1")
            .fetch_one(&mut *tx)
            .await?;
    if kind == notes::KIND_JSON
        && usize::try_from(bytes).unwrap_or(usize::MAX) > notes::MAX_JSON_BYTES
    {
        return Err(AppError::Custom(notes::JSON_TOO_LARGE.to_string()));
    }
    tx.commit().await?;
    inner_get_scratchpad(pool).await
}

/// Turns the pad into a note: a root note of the pad's kind titled `title` whose body is
/// `body` (the pad's live text, unchanged), then clears the pad (keeping its kind), all
/// in one transaction. The title follows the notes rules (required, length, characters,
/// unique) and a JSON body the size limit; on any error nothing changes.
pub async fn inner_save_scratchpad_as_note(
    title: &str,
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<Note, AppError> {
    let mut tx = pool.begin().await?;
    let kind = stored_kind(&mut *tx).await?;
    let id = notes::insert_titled_note(title, body, &kind, None, &mut tx).await?;
    store_body("", &mut *tx).await?;
    tx.commit().await?;
    notes::inner_get_note(id, pool).await
}

/// `inner_save_scratchpad_as_note`, then queues the new note for the semantic index
/// (markdown only: JSON notes are never embedded).
pub async fn save_scratchpad_as_note_and_queue(
    title: &str,
    body: &str,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<Note, AppError> {
    let note = inner_save_scratchpad_as_note(title, body, pool).await?;
    if note.kind != notes::KIND_JSON {
        semantic.enqueue([note.id]);
    }
    Ok(note)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_scratchpad(pool: State<'_, Pool<Sqlite>>) -> Result<Scratchpad, AppError> {
    inner_get_scratchpad(pool.inner()).await
}

#[tauri::command]
pub async fn set_scratchpad_body(
    body: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Scratchpad, AppError> {
    inner_set_scratchpad_body(&body, pool.inner()).await
}

/// `body`: the pad's unsaved text, stored in the same transaction.
#[tauri::command]
pub async fn set_scratchpad_kind(
    kind: String,
    body: Option<String>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Scratchpad, AppError> {
    inner_set_scratchpad_kind(&kind, body.as_deref(), pool.inner()).await
}

#[tauri::command]
pub async fn save_scratchpad_as_note(
    title: String,
    body: String,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<Note, AppError> {
    save_scratchpad_as_note_and_queue(&title, &body, pool.inner(), semantic.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn get_on_an_empty_db_returns_an_empty_pad() {
        let pool = crate::test_db().await;
        let pad = inner_get_scratchpad(&pool).await.unwrap();
        assert_eq!(pad.body, "");
        assert_eq!(pad.updated_at, None);
    }

    #[tokio::test]
    async fn set_then_get_round_trips_and_unchanged_keeps_updated_at() {
        let pool = crate::test_db().await;
        let body = "# Call notes\r\n\n- [ ] ring Åsa 📞\n";
        let saved = inner_set_scratchpad_body(body, &pool).await.unwrap();
        assert_eq!(saved.body, body);
        assert!(saved.updated_at.is_some());
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap(), saved);

        sqlx::query("UPDATE scratchpad SET UpdatedAt = 'old'")
            .execute(&pool)
            .await
            .unwrap();
        let same = inner_set_scratchpad_body(body, &pool).await.unwrap();
        assert_eq!(same.updated_at.as_deref(), Some("old"), "no-op write");

        let cleared = inner_set_scratchpad_body("", &pool).await.unwrap();
        assert_eq!(cleared.body, "");
        assert_ne!(cleared.updated_at.as_deref(), Some("old"));
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM scratchpad")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 1, "single row");
    }

    #[tokio::test]
    async fn pad_text_stays_out_of_notes_search_and_link_indexes() {
        let pool = crate::test_db().await;
        let n = notes::inner_create_note_titled("Visible", None, &pool)
            .await
            .unwrap();
        notes::inner_update_note_body(n.id, "plain text", &pool)
            .await
            .unwrap();
        inner_set_scratchpad_body("zebracorn secret #padtag [[Visible]]", &pool)
            .await
            .unwrap();

        for query in ["zebracorn", "secret", "#padtag"] {
            let hits = notes::inner_search_notes(query, None, &pool).await.unwrap();
            assert!(hits.is_empty(), "{query} must not hit the pad");
        }
        let fts: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notes_fts WHERE notes_fts MATCH 'zebracorn'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(fts, 0);
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refs, 0, "no tags or links indexed from the pad");
        assert!(notes::inner_list_backlinks(n.id, &pool)
            .await
            .unwrap()
            .is_empty());
        let embedded: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_embeddings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(embedded, 0);
        let tags = notes::inner_list_tag_suggestions(&pool).await.unwrap();
        assert!(tags.iter().all(|t| !t.tag.contains("padtag")));
    }

    async fn note_count(pool: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM notes")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn save_as_note_creates_an_indexed_root_note_and_clears_the_pad() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        sqlx::raw_sql(
            "INSERT INTO folders (id, Name) VALUES (1, 'Work');
             INSERT INTO notes (FolderId, Title, CreatedAt, UpdatedAt) VALUES (1, 'Other', 'x', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        inner_set_scratchpad_body("stale saved text", &pool)
            .await
            .unwrap();
        // The live text (not yet autosaved) is what becomes the note, byte for byte.
        let body = "\n  # Quokka plan\r\n- see [[Other]] #ideas\n\n";

        let note = save_scratchpad_as_note_and_queue("  Quokka plan  ", body, &pool, &state)
            .await
            .unwrap();
        assert_eq!(note.title, "Quokka plan");
        assert_eq!(note.body, body);
        assert_eq!(note.folder_id, None, "lands at the root");
        assert_eq!(state.queued(), vec![note.id], "queued for embedding");

        let hits = notes::inner_search_notes("quokka", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note.id, note.id);
        let other: i64 = sqlx::query_scalar("SELECT id FROM notes WHERE Title = 'Other'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let backlinks = notes::inner_list_backlinks(other, &pool).await.unwrap();
        assert_eq!(backlinks.len(), 1, "links indexed");

        let pad = inner_get_scratchpad(&pool).await.unwrap();
        assert_eq!(pad.body, "", "pad cleared");
    }

    #[tokio::test]
    async fn save_as_note_rejects_bad_titles_and_leaves_the_pad_alone() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        for title in ["Plan", "Åsa"] {
            notes::inner_create_note_titled(title, None, &pool)
                .await
                .unwrap();
        }
        state.drain(&pool).await;
        inner_set_scratchpad_body("keep me", &pool).await.unwrap();
        let before = inner_get_scratchpad(&pool).await.unwrap();
        let long = "x".repeat(notes::MAX_TITLE_CHARS + 1);

        let cases = [
            ("", "Title is required".to_string()),
            ("   ", "Title is required".to_string()),
            (long.as_str(), "Title is too long".to_string()),
            ("a [b]", "Title can't contain [ ] | # or `".to_string()),
            ("a | b", "Title can't contain [ ] | # or `".to_string()),
            ("#tag", "Title can't contain [ ] | # or `".to_string()),
            ("co`de", "Title can't contain [ ] | # or `".to_string()),
            ("plan", "A note named \"plan\" already exists".to_string()),
            (" ÅSA ", "A note named \"ÅSA\" already exists".to_string()),
        ];
        for (title, message) in cases {
            let err = save_scratchpad_as_note_and_queue(title, "unsaved live text", &pool, &state)
                .await
                .unwrap_err();
            assert_eq!(err.to_string(), message, "title {title:?}");
            assert_eq!(inner_get_scratchpad(&pool).await.unwrap(), before);
            assert_eq!(note_count(&pool).await, 2);
            assert!(state.queued().is_empty(), "nothing queued for {title:?}");
        }
        // Exactly the maximum length is fine.
        let ok = "y".repeat(notes::MAX_TITLE_CHARS);
        assert!(inner_save_scratchpad_as_note(&ok, "x", &pool).await.is_ok());
    }

    #[tokio::test]
    async fn kind_persists_upserts_and_survives_clear() {
        let pool = crate::test_db().await;
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap().kind, "markdown");
        assert_eq!(
            inner_set_scratchpad_kind("yaml", None, &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Unknown note kind \"yaml\""
        );
        // Never written: the switch creates the row.
        let pad = inner_set_scratchpad_kind("json", None, &pool)
            .await
            .unwrap();
        assert_eq!((pad.body.as_str(), pad.kind.as_str()), ("", "json"));
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap(), pad);

        // The pending body is stored with the switch and never converted.
        let pad = inner_set_scratchpad_kind("markdown", Some("not json"), &pool)
            .await
            .unwrap();
        assert_eq!(
            (pad.body.as_str(), pad.kind.as_str()),
            ("not json", "markdown")
        );
        inner_set_scratchpad_kind("json", None, &pool)
            .await
            .unwrap();
        // Clear keeps the mode.
        let cleared = inner_set_scratchpad_body("", &pool).await.unwrap();
        assert_eq!((cleared.body.as_str(), cleared.kind.as_str()), ("", "json"));
    }

    #[tokio::test]
    async fn json_pad_is_limited_to_5_mb() {
        let pool = crate::test_db().await;
        let exact = "x".repeat(notes::MAX_JSON_BYTES);
        let over = "x".repeat(notes::MAX_JSON_BYTES + 1);
        inner_set_scratchpad_body(&over, &pool).await.unwrap(); // markdown: no limit
        let err = inner_set_scratchpad_kind("json", None, &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), notes::JSON_TOO_LARGE);
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap().kind, "markdown");
        let err = inner_set_scratchpad_kind("json", Some(&over), &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), notes::JSON_TOO_LARGE);

        inner_set_scratchpad_kind("json", Some(&exact), &pool)
            .await
            .unwrap();
        let err = inner_set_scratchpad_body(&over, &pool).await.unwrap_err();
        assert_eq!(err.to_string(), notes::JSON_TOO_LARGE);
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap().body, exact);
        // Over the limit (restore): switching to markdown with the pending text works.
        sqlx::query("UPDATE scratchpad SET Body = ?")
            .bind(&over)
            .execute(&pool)
            .await
            .unwrap();
        let pad = inner_set_scratchpad_kind("markdown", Some(&over), &pool)
            .await
            .unwrap();
        assert_eq!(pad.kind, "markdown");
    }

    #[tokio::test]
    async fn json_pad_over_the_limit_rejects_even_an_unchanged_body_until_shortened() {
        let pool = crate::test_db().await;
        let over = "x".repeat(notes::MAX_JSON_BYTES + 1);
        inner_set_scratchpad_kind("json", Some("{}"), &pool)
            .await
            .unwrap();
        // A restore can leave a JSON pad over the limit.
        sqlx::query("UPDATE scratchpad SET Body = ?")
            .bind(&over)
            .execute(&pool)
            .await
            .unwrap();
        let err = inner_set_scratchpad_body(&over, &pool).await.unwrap_err();
        assert_eq!(err.to_string(), notes::JSON_TOO_LARGE);
        let pad = inner_set_scratchpad_body("[1]", &pool).await.unwrap();
        assert_eq!((pad.body.as_str(), pad.kind.as_str()), ("[1]", "json"));
        // An unchanged body under the limit is still a no-op, not an error.
        let again = inner_set_scratchpad_body("[1]", &pool).await.unwrap();
        assert_eq!(again, pad);
    }

    #[tokio::test]
    async fn json_pad_saves_as_an_unindexed_json_note_and_keeps_its_kind() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        notes::inner_create_note_titled("Other", None, &pool)
            .await
            .unwrap();
        inner_set_scratchpad_kind("json", None, &pool)
            .await
            .unwrap();
        let body = "{\"q\": \"zebracorn #padtag [[Other]]\"}";

        let note = save_scratchpad_as_note_and_queue("Payload", body, &pool, &state)
            .await
            .unwrap();
        assert_eq!((note.kind.as_str(), note.body.as_str()), ("json", body));
        assert!(state.queued().is_empty(), "JSON notes are not embedded");
        let hits = notes::inner_search_notes("zebracorn", None, &pool)
            .await
            .unwrap();
        assert!(hits.is_empty(), "title-only");
        let hits = notes::inner_search_notes("payload", None, &pool)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        let refs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(refs, 0, "no refs");
        let pad = inner_get_scratchpad(&pool).await.unwrap();
        assert_eq!((pad.body.as_str(), pad.kind.as_str()), ("", "json"));

        // The limit applies; nothing changes on error.
        let over = "x".repeat(notes::MAX_JSON_BYTES + 1);
        let err = inner_save_scratchpad_as_note("Huge", &over, &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), notes::JSON_TOO_LARGE);
        assert_eq!(note_count(&pool).await, 2);

        // Markdown pad: a markdown note, queued.
        inner_set_scratchpad_kind("markdown", None, &pool)
            .await
            .unwrap();
        let note = save_scratchpad_as_note_and_queue("Prose", "words", &pool, &state)
            .await
            .unwrap();
        assert_eq!(note.kind, "markdown");
        assert_eq!(state.queued(), vec![note.id]);
    }

    #[tokio::test]
    async fn save_as_note_is_atomic_when_clearing_the_pad_fails() {
        let pool = crate::test_db().await;
        inner_set_scratchpad_body("keep me", &pool).await.unwrap();
        sqlx::raw_sql(
            "CREATE TRIGGER pad_locked BEFORE UPDATE ON scratchpad
             BEGIN SELECT RAISE(ABORT, 'pad locked'); END;",
        )
        .execute(&pool)
        .await
        .unwrap();

        let err = inner_save_scratchpad_as_note("Fresh", "keep me", &pool)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("pad locked"), "{err}");
        assert_eq!(note_count(&pool).await, 0, "note insert rolled back");
        let fts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notes_fts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(fts, 0, "index row rolled back");
        assert_eq!(inner_get_scratchpad(&pool).await.unwrap().body, "keep me");
    }
}
