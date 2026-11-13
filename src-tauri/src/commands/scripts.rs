//! Playground scripts: saved JavaScript snippets run by the playground (`js_runner`). They
//! live in their own table, so they never reach the notes search index, the link index or
//! the semantic index. Titles are not unique.
use crate::commands::notes::now;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

/// Maximum title length in characters (not bytes).
pub const MAX_TITLE_CHARS: usize = 200;
const UNTITLED: &str = "Untitled";

/// A full script, including its body.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Script {
    pub id: i64,
    pub title: String,
    pub body: String,
    pub created_at: String,
    pub updated_at: String,
}

/// A script for the list (no body).
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct ScriptSummary {
    pub id: i64,
    pub title: String,
    pub updated_at: String,
}

const SELECT_SCRIPTS: &str = "SELECT id, Title AS title, Body AS body, \
     CreatedAt AS created_at, UpdatedAt AS updated_at FROM scripts";

const SELECT_SUMMARIES: &str = "SELECT id, Title AS title, UpdatedAt AS updated_at FROM scripts";

fn not_found() -> AppError {
    AppError::Custom("Script not found".to_string())
}

/// Trimmed, required, at most [`MAX_TITLE_CHARS`]; any characters (unlike note titles).
fn clean_script_title(title: &str) -> Result<String, AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError::Custom("Title is required".to_string()));
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::Custom("Title is too long".to_string()));
    }
    Ok(t.to_string())
}

async fn get_summary(id: i64, pool: &Pool<Sqlite>) -> Result<ScriptSummary, AppError> {
    let sql = format!("{SELECT_SUMMARIES} WHERE id = ?");
    sqlx::query_as::<_, ScriptSummary>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(not_found)
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// All scripts without bodies, most recently changed first.
pub async fn inner_list_scripts(pool: &Pool<Sqlite>) -> Result<Vec<ScriptSummary>, AppError> {
    let sql = format!("{SELECT_SUMMARIES} ORDER BY UpdatedAt DESC, id DESC");
    Ok(sqlx::query_as::<_, ScriptSummary>(&sql)
        .fetch_all(pool)
        .await?)
}

pub async fn inner_get_script(id: i64, pool: &Pool<Sqlite>) -> Result<Script, AppError> {
    let sql = format!("{SELECT_SCRIPTS} WHERE id = ?");
    sqlx::query_as::<_, Script>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(not_found)
}

/// Creates an empty script titled "Untitled".
pub async fn inner_create_script(pool: &Pool<Sqlite>) -> Result<Script, AppError> {
    let ts = now();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO scripts (Title, Body, CreatedAt, UpdatedAt) VALUES (?, '', ?, ?) RETURNING id",
    )
    .bind(UNTITLED)
    .bind(&ts)
    .bind(&ts)
    .fetch_one(pool)
    .await?;
    inner_get_script(id, pool).await
}

/// Renames a script. An unchanged title is a no-op that keeps `UpdatedAt`.
pub async fn inner_update_script_title(
    id: i64,
    title: &str,
    pool: &Pool<Sqlite>,
) -> Result<ScriptSummary, AppError> {
    let title = clean_script_title(title)?;
    sqlx::query("UPDATE scripts SET Title = ?, UpdatedAt = ? WHERE id = ? AND Title IS NOT ?")
        .bind(&title)
        .bind(now())
        .bind(id)
        .bind(&title)
        .execute(pool)
        .await?;
    get_summary(id, pool).await
}

/// Replaces the body (no length limit). An identical body is a no-op that keeps `UpdatedAt`.
pub async fn inner_update_script_body(
    id: i64,
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<ScriptSummary, AppError> {
    sqlx::query("UPDATE scripts SET Body = ?, UpdatedAt = ? WHERE id = ? AND Body IS NOT ?")
        .bind(body)
        .bind(now())
        .bind(id)
        .bind(body)
        .execute(pool)
        .await?;
    get_summary(id, pool).await
}

pub async fn inner_delete_script(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM scripts WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(())
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_scripts(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<ScriptSummary>, AppError> {
    inner_list_scripts(pool.inner()).await
}

#[tauri::command]
pub async fn get_script(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<Script, AppError> {
    inner_get_script(id, pool.inner()).await
}

#[tauri::command]
pub async fn create_script(pool: State<'_, Pool<Sqlite>>) -> Result<Script, AppError> {
    inner_create_script(pool.inner()).await
}

#[tauri::command]
pub async fn update_script_title(
    id: i64,
    title: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<ScriptSummary, AppError> {
    inner_update_script_title(id, &title, pool.inner()).await
}

#[tauri::command]
pub async fn update_script_body(
    id: i64,
    body: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<ScriptSummary, AppError> {
    inner_update_script_body(id, &body, pool.inner()).await
}

#[tauri::command]
pub async fn delete_script(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_script(id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    async fn set_updated(pool: &Pool<Sqlite>, id: i64, ts: &str) {
        sqlx::query("UPDATE scripts SET UpdatedAt = ? WHERE id = ?")
            .bind(ts)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_get_and_list() {
        let pool = crate::test_db().await;
        assert!(inner_list_scripts(&pool).await.unwrap().is_empty());
        let s = inner_create_script(&pool).await.unwrap();
        assert_eq!(s.title, "Untitled");
        assert_eq!(s.body, "");
        assert_eq!(s.created_at, s.updated_at);
        assert_eq!(inner_get_script(s.id, &pool).await.unwrap(), s);
        // Titles are not unique.
        let t = inner_create_script(&pool).await.unwrap();
        assert_eq!(t.title, "Untitled");
        assert_ne!(t.id, s.id);
        assert_eq!(
            inner_get_script(999, &pool).await.unwrap_err().to_string(),
            "Script not found"
        );
    }

    #[tokio::test]
    async fn list_is_most_recently_updated_first() {
        let pool = crate::test_db().await;
        let a = inner_create_script(&pool).await.unwrap();
        let b = inner_create_script(&pool).await.unwrap();
        let c = inner_create_script(&pool).await.unwrap();
        set_updated(&pool, a.id, "2026-01-03T00:00:00.000").await;
        set_updated(&pool, b.id, "2026-01-01T00:00:00.000").await;
        set_updated(&pool, c.id, "2026-01-02T00:00:00.000").await;
        let ids: Vec<i64> = inner_list_scripts(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, [a.id, c.id, b.id]);
        // Editing moves a script to the top.
        inner_update_script_body(b.id, "1", &pool).await.unwrap();
        assert_eq!(inner_list_scripts(&pool).await.unwrap()[0].id, b.id);
    }

    #[tokio::test]
    async fn title_is_trimmed_validated_and_takes_any_characters() {
        let pool = crate::test_db().await;
        let s = inner_create_script(&pool).await.unwrap();
        let renamed = inner_update_script_title(s.id, "  [[Tally]] #1 | `x` Åsa 🦡  ", &pool)
            .await
            .unwrap();
        assert_eq!(renamed.title, "[[Tally]] #1 | `x` Åsa 🦡");
        for blank in ["", "   \t"] {
            assert_eq!(
                inner_update_script_title(s.id, blank, &pool)
                    .await
                    .unwrap_err()
                    .to_string(),
                "Title is required"
            );
        }
        let max = "é".repeat(MAX_TITLE_CHARS);
        assert_eq!(
            inner_update_script_title(s.id, &max, &pool)
                .await
                .unwrap()
                .title,
            max
        );
        assert_eq!(
            inner_update_script_title(s.id, &format!("{max}x"), &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Title is too long"
        );
        assert_eq!(
            inner_update_script_title(999, "x", &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Script not found"
        );
    }

    #[tokio::test]
    async fn updated_at_moves_only_on_real_change() {
        let pool = crate::test_db().await;
        let s = inner_create_script(&pool).await.unwrap();
        inner_update_script_body(s.id, "1 + 2", &pool)
            .await
            .unwrap();
        inner_update_script_title(s.id, "Sum", &pool).await.unwrap();
        set_updated(&pool, s.id, "old").await;

        let same = inner_update_script_body(s.id, "1 + 2", &pool)
            .await
            .unwrap();
        assert_eq!(same.updated_at, "old", "identical body");
        let same = inner_update_script_title(s.id, " Sum ", &pool)
            .await
            .unwrap();
        assert_eq!(same.updated_at, "old", "identical title");

        let changed = inner_update_script_body(s.id, "1 + 3", &pool)
            .await
            .unwrap();
        assert_ne!(changed.updated_at, "old");
        set_updated(&pool, s.id, "old").await;
        let changed = inner_update_script_title(s.id, "Total", &pool)
            .await
            .unwrap();
        assert_ne!(changed.updated_at, "old");
        let full = inner_get_script(s.id, &pool).await.unwrap();
        assert_eq!(full.body, "1 + 3");
        assert_eq!(full.title, "Total");
        assert_eq!(full.created_at, s.created_at, "created stays");
        assert_eq!(
            inner_update_script_body(999, "x", &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Script not found"
        );
    }

    #[tokio::test]
    async fn delete_removes_and_missing_is_not_found() {
        let pool = crate::test_db().await;
        let a = inner_create_script(&pool).await.unwrap();
        let b = inner_create_script(&pool).await.unwrap();
        inner_delete_script(a.id, &pool).await.unwrap();
        let ids: Vec<i64> = inner_list_scripts(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| s.id)
            .collect();
        assert_eq!(ids, [b.id]);
        assert_eq!(
            inner_delete_script(a.id, &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Script not found"
        );
    }

    #[tokio::test]
    async fn script_text_stays_out_of_notes_search() {
        let pool = crate::test_db().await;
        let s = inner_create_script(&pool).await.unwrap();
        inner_update_script_body(s.id, "const zebracorn = '#tag [[Link]]'", &pool)
            .await
            .unwrap();
        inner_update_script_title(s.id, "zebracorn", &pool)
            .await
            .unwrap();
        let hits = crate::commands::notes::inner_search_notes("zebracorn", None, &pool)
            .await
            .unwrap();
        assert!(hits.is_empty());
    }
}
