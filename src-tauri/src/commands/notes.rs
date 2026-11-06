use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

/// Maximum title length in characters (not bytes).
pub const MAX_TITLE_CHARS: usize = 200;
/// Maximum excerpt length in characters (not bytes).
pub const MAX_EXCERPT_CHARS: usize = 120;
/// Characters that would break `[[wikilinks]]` and `#tags`.
const FORBIDDEN_TITLE_CHARS: [char; 4] = ['[', ']', '|', '#'];
const UNTITLED: &str = "Untitled";

/// A full note, including its body.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Note {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub title: String,
    pub body: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// A note for the list: the body is replaced by a short plain-text excerpt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteSummary {
    pub id: i64,
    pub folder_id: Option<i64>,
    pub title: String,
    pub excerpt: String,
    pub pinned: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl From<Note> for NoteSummary {
    fn from(n: Note) -> Self {
        NoteSummary {
            excerpt: excerpt(&n.body, &n.title),
            id: n.id,
            folder_id: n.folder_id,
            title: n.title,
            pinned: n.pinned,
            created_at: n.created_at,
            updated_at: n.updated_at,
        }
    }
}

const SELECT_NOTES: &str = "SELECT id,
        FolderId  AS folder_id,
        Title     AS title,
        Body      AS body,
        Pinned    AS pinned,
        CreatedAt AS created_at,
        UpdatedAt AS updated_at
 FROM notes";

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found() -> AppError {
    AppError::Custom("Note not found".to_string())
}

fn duplicate(title: &str) -> AppError {
    AppError::Custom(format!("A note named \"{title}\" already exists"))
}

fn now() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f")
        .to_string()
}

/// Maps a unique-index violation (a race past the Rust-side check) to the duplicate message.
fn map_unique(err: sqlx::Error, title: &str) -> AppError {
    match &err {
        sqlx::Error::Database(db) if db.is_unique_violation() => duplicate(title),
        _ => AppError::Database(err),
    }
}

fn clean_title(title: &str) -> Result<String, AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(AppError::Custom("Title is required".to_string()));
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(AppError::Custom("Title is too long".to_string()));
    }
    if t.contains(FORBIDDEN_TITLE_CHARS) {
        return Err(AppError::Custom(
            "Title can't contain [ ] | or #".to_string(),
        ));
    }
    Ok(t.to_string())
}

/// Lowercased titles of all notes except `except_id`. Rust's `to_lowercase` folds
/// non-ASCII letters too (`Å` → `å`), which SQLite's NOCASE does not.
async fn lowercase_titles(
    except_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<std::collections::HashSet<String>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT Title FROM notes WHERE id IS NOT ?")
        .bind(except_id)
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|(t,)| t.to_lowercase()).collect())
}

/// First free title in `Untitled`, `Untitled 2`, `Untitled 3`, …
fn first_free_untitled(taken: &std::collections::HashSet<String>) -> String {
    if !taken.contains(&UNTITLED.to_lowercase()) {
        return UNTITLED.to_string();
    }
    (2..)
        .map(|n| format!("{UNTITLED} {n}"))
        .find(|t| !taken.contains(&t.to_lowercase()))
        .unwrap_or_else(|| UNTITLED.to_string())
}

/// Strips one leading markdown marker (heading, quote, list, task, table pipe).
fn strip_marker(line: &str) -> Option<&str> {
    let line = line.trim_start();
    let after_word = |rest: &str| -> Option<usize> {
        // marker must be followed by whitespace or end of line
        match rest.chars().next() {
            None => Some(0),
            Some(c) if c.is_whitespace() => Some(c.len_utf8()),
            _ => None,
        }
    };
    if line.starts_with('#') {
        let hashes = line.chars().take_while(|&c| c == '#').count();
        if hashes <= 6 {
            if let Some(skip) = after_word(&line[hashes..]) {
                return Some(&line[hashes + skip..]);
            }
        }
        return None;
    }
    if let Some(rest) = line.strip_prefix('>') {
        return Some(rest);
    }
    if let Some(rest) = line.strip_prefix('|') {
        return Some(rest);
    }
    for marker in ["- ", "* ", "+ "] {
        if let Some(rest) = line.strip_prefix(marker) {
            return Some(rest);
        }
    }
    if matches!(line, "-" | "*" | "+") {
        return Some("");
    }
    for marker in ["[ ]", "[x]", "[X]"] {
        if let Some(rest) = line.strip_prefix(marker) {
            if let Some(skip) = after_word(rest) {
                return Some(&rest[skip..]);
            }
        }
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if (1..=9).contains(&digits) {
        let rest = &line[digits..];
        if let Some(rest) = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')')) {
            if let Some(skip) = after_word(rest) {
                return Some(&rest[skip..]);
            }
        }
    }
    None
}

fn is_fence(line: &str) -> bool {
    line.starts_with("```") || line.starts_with("~~~")
}

/// A GFM table delimiter row (`| --- | :-: |`) or a thematic break made of `-`.
fn is_delimiter_row(line: &str) -> bool {
    line.contains('-')
        && line
            .chars()
            .all(|c| matches!(c, '-' | ':' | '|') || c.is_whitespace())
}

/// A setext H1 underline (`===`); `---` is already skipped as a delimiter row.
fn is_setext_underline(line: &str) -> bool {
    !line.is_empty() && line.chars().all(|c| c == '=')
}

/// Plain-text preview: the first non-empty body line that is not a code fence, table
/// delimiter row or setext underline, with leading markers stripped, at most 120 chars.
/// A leading heading that repeats the title is skipped, so the list doesn't show it twice.
pub fn excerpt(body: &str, title: &str) -> String {
    let lines: Vec<&str> = body.lines().map(str::trim).collect();
    let mut first = true;
    for (i, &line) in lines.iter().enumerate() {
        if line.is_empty() || is_fence(line) || is_delimiter_row(line) || is_setext_underline(line)
        {
            continue;
        }
        let is_heading = line.starts_with('#')
            || lines
                .get(i + 1)
                .is_some_and(|next| is_setext_underline(next) || is_delimiter_row(next));
        let mut text = line;
        while let Some(rest) = strip_marker(text) {
            text = rest.trim_start();
        }
        let text = text.strip_suffix('|').unwrap_or(text).trim();
        if text.is_empty() {
            continue;
        }
        let repeats_title = first && is_heading && text.to_lowercase() == title.to_lowercase();
        first = false;
        if repeats_title {
            continue;
        }
        let truncated: String = text.chars().take(MAX_EXCERPT_CHARS).collect();
        return truncated.trim_end().to_string();
    }
    String::new()
}

/// Writes a title without the Rust-side duplicate check; the unique index is the backstop.
async fn store_title(id: i64, title: &str, pool: &Pool<Sqlite>) -> Result<u64, AppError> {
    let result =
        sqlx::query("UPDATE notes SET Title = ?, UpdatedAt = ? WHERE id = ? AND Title IS NOT ?")
            .bind(title)
            .bind(now())
            .bind(id)
            .bind(title)
            .execute(pool)
            .await
            .map_err(|e| map_unique(e, title))?;
    Ok(result.rows_affected())
}

/// Inserts a note without the Rust-side duplicate check; the unique index is the backstop.
async fn insert_note(title: &str, pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    let ts = now();
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO notes (Title, Body, Pinned, CreatedAt, UpdatedAt) \
         VALUES (?, '', 0, ?, ?) RETURNING id",
    )
    .bind(title)
    .bind(&ts)
    .bind(&ts)
    .fetch_one(pool)
    .await
    .map_err(|e| map_unique(e, title))?;
    Ok(row.0)
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// All notes without bodies: pinned first, then most recently updated, then id.
pub async fn inner_list_notes(pool: &Pool<Sqlite>) -> Result<Vec<NoteSummary>, AppError> {
    let sql = format!("{SELECT_NOTES} ORDER BY Pinned DESC, UpdatedAt DESC, id");
    let notes = sqlx::query_as::<_, Note>(&sql).fetch_all(pool).await?;
    Ok(notes.into_iter().map(NoteSummary::from).collect())
}

pub async fn inner_get_note(id: i64, pool: &Pool<Sqlite>) -> Result<Note, AppError> {
    let sql = format!("{SELECT_NOTES} WHERE id = ?");
    sqlx::query_as::<_, Note>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(not_found)
}

async fn get_summary(id: i64, pool: &Pool<Sqlite>) -> Result<NoteSummary, AppError> {
    inner_get_note(id, pool).await.map(NoteSummary::from)
}

/// Creates an empty note titled with the first free `Untitled[ n]`.
pub async fn inner_create_note(pool: &Pool<Sqlite>) -> Result<Note, AppError> {
    let title = first_free_untitled(&lowercase_titles(None, pool).await?);
    let id = insert_note(&title, pool).await?;
    inner_get_note(id, pool).await
}

/// Renames a note. Titles are unique case-insensitively (including non-ASCII).
/// An unchanged title is a no-op that keeps `UpdatedAt`.
pub async fn inner_rename_note(
    id: i64,
    title: &str,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    let title = clean_title(title)?;
    inner_get_note(id, pool).await?;
    if lowercase_titles(Some(id), pool)
        .await?
        .contains(&title.to_lowercase())
    {
        return Err(duplicate(&title));
    }
    store_title(id, &title, pool).await?;
    get_summary(id, pool).await
}

/// Replaces the body. An identical body is a no-op that keeps `UpdatedAt`.
pub async fn inner_update_note_body(
    id: i64,
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    // 0 rows affected means unchanged or missing; get_summary tells them apart.
    sqlx::query("UPDATE notes SET Body = ?, UpdatedAt = ? WHERE id = ? AND Body IS NOT ?")
        .bind(body)
        .bind(now())
        .bind(id)
        .bind(body)
        .execute(pool)
        .await?;
    get_summary(id, pool).await
}

/// Pins or unpins a note without touching `UpdatedAt`.
pub async fn inner_set_note_pinned(
    id: i64,
    pinned: bool,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    let result = sqlx::query("UPDATE notes SET Pinned = ? WHERE id = ?")
        .bind(pinned)
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    get_summary(id, pool).await
}

pub async fn inner_delete_note(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM notes WHERE id = ?")
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
pub async fn list_notes(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<NoteSummary>, AppError> {
    inner_list_notes(pool.inner()).await
}

#[tauri::command]
pub async fn get_note(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<Note, AppError> {
    inner_get_note(id, pool.inner()).await
}

#[tauri::command]
pub async fn create_note(pool: State<'_, Pool<Sqlite>>) -> Result<Note, AppError> {
    inner_create_note(pool.inner()).await
}

#[tauri::command]
pub async fn rename_note(
    id: i64,
    title: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<NoteSummary, AppError> {
    inner_rename_note(id, &title, pool.inner()).await
}

#[tauri::command]
pub async fn update_note_body(
    id: i64,
    body: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<NoteSummary, AppError> {
    inner_update_note_body(id, &body, pool.inner()).await
}

#[tauri::command]
pub async fn set_note_pinned(
    id: i64,
    pinned: bool,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<NoteSummary, AppError> {
    inner_set_note_pinned(id, pinned, pool.inner()).await
}

#[tauri::command]
pub async fn delete_note(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_note(id, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const OLD: &str = "2000-01-01T00:00:00.000";

    fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
        r.unwrap_err().to_string()
    }

    async fn titles(pool: &Pool<Sqlite>) -> Vec<String> {
        inner_list_notes(pool)
            .await
            .unwrap()
            .into_iter()
            .map(|n| n.title)
            .collect()
    }

    async fn set_updated_at(id: i64, ts: &str, pool: &Pool<Sqlite>) {
        sqlx::query("UPDATE notes SET UpdatedAt = ? WHERE id = ?")
            .bind(ts)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn note_titled(title: &str, pool: &Pool<Sqlite>) -> Note {
        let n = inner_create_note(pool).await.unwrap();
        inner_rename_note(n.id, title, pool).await.unwrap();
        inner_get_note(n.id, pool).await.unwrap()
    }

    #[tokio::test]
    async fn create_numbers_untitled_and_refills_gaps() {
        let pool = crate::test_db().await;
        let a = inner_create_note(&pool).await.unwrap();
        let b = inner_create_note(&pool).await.unwrap();
        let c = inner_create_note(&pool).await.unwrap();
        assert_eq!(
            (a.title.as_str(), b.title.as_str(), c.title.as_str()),
            ("Untitled", "Untitled 2", "Untitled 3")
        );
        assert_eq!(a.body, "");
        assert!(!a.pinned);
        assert_eq!(a.folder_id, None);
        assert_eq!(a.created_at, a.updated_at);
        assert_eq!(a.created_at.len(), "2026-01-01T00:00:00.000".len());

        inner_delete_note(b.id, &pool).await.unwrap();
        let d = inner_create_note(&pool).await.unwrap();
        assert_eq!(d.title, "Untitled 2", "first free number is reused");

        // case-insensitive: a note named "untitled 4" blocks "Untitled 4"
        inner_rename_note(d.id, "untitled 4", &pool).await.unwrap();
        let e = inner_create_note(&pool).await.unwrap();
        assert_eq!(e.title, "Untitled 2");
        let f = inner_create_note(&pool).await.unwrap();
        assert_eq!(f.title, "Untitled 5");
    }

    #[tokio::test]
    async fn rename_validates_title() {
        let pool = crate::test_db().await;
        let n = inner_create_note(&pool).await.unwrap();
        let rename = |t: String| {
            let pool = pool.clone();
            async move { inner_rename_note(n.id, &t, &pool).await }
        };
        assert_eq!(err_msg(rename("".into()).await), "Title is required");
        assert_eq!(err_msg(rename("  \t ".into()).await), "Title is required");
        assert_eq!(err_msg(rename("x".repeat(201)).await), "Title is too long");
        // 200 multi-byte chars are fine (limit counts chars, not bytes)
        let long = "å".repeat(200);
        assert_eq!(rename(long.clone()).await.unwrap().title, long);
        for bad in ["a[b", "a]b", "a|b", "#tag"] {
            assert_eq!(
                err_msg(rename(bad.into()).await),
                "Title can't contain [ ] | or #",
                "{bad}"
            );
        }
        let ok = rename("  Meeting notes 🎉  ".into()).await.unwrap();
        assert_eq!(ok.title, "Meeting notes 🎉");
    }

    #[tokio::test]
    async fn rename_rejects_case_insensitive_duplicates_including_non_ascii() {
        let pool = crate::test_db().await;
        note_titled("Åsa", &pool).await;
        note_titled("Plan", &pool).await;
        let n = inner_create_note(&pool).await.unwrap();

        assert_eq!(
            err_msg(inner_rename_note(n.id, "åsa", &pool).await),
            "A note named \"åsa\" already exists"
        );
        assert_eq!(
            err_msg(inner_rename_note(n.id, " PLAN ", &pool).await),
            "A note named \"PLAN\" already exists"
        );
        assert_eq!(titles(&pool).await.len(), 3);
    }

    #[tokio::test]
    async fn rename_to_own_title_with_case_change_succeeds() {
        let pool = crate::test_db().await;
        let n = note_titled("åsa", &pool).await;
        set_updated_at(n.id, OLD, &pool).await;

        let same = inner_rename_note(n.id, "åsa", &pool).await.unwrap();
        assert_eq!(same.updated_at, OLD, "unchanged title must not bump");

        let renamed = inner_rename_note(n.id, "Åsa", &pool).await.unwrap();
        assert_eq!(renamed.title, "Åsa");
        assert_ne!(renamed.updated_at, OLD, "title change bumps UpdatedAt");
    }

    #[tokio::test]
    async fn unique_violation_race_maps_to_duplicate_message() {
        let pool = crate::test_db().await;
        let a = note_titled("Plan", &pool).await;
        let b = inner_create_note(&pool).await.unwrap();

        // Skip the Rust-side check, as a concurrent writer would: the index catches it.
        assert_eq!(
            err_msg(store_title(b.id, "plan", &pool).await),
            "A note named \"plan\" already exists"
        );
        assert_eq!(
            err_msg(insert_note("PLAN", &pool).await),
            "A note named \"PLAN\" already exists"
        );
        assert_eq!(inner_get_note(a.id, &pool).await.unwrap().title, "Plan");
    }

    #[tokio::test]
    async fn update_body_bumps_updated_at_and_returns_excerpt() {
        let pool = crate::test_db().await;
        let n = inner_create_note(&pool).await.unwrap();
        set_updated_at(n.id, OLD, &pool).await;

        let s = inner_update_note_body(n.id, "\n\n  # Hello *world*\nmore", &pool)
            .await
            .unwrap();
        assert_ne!(s.updated_at, OLD);
        assert_eq!(s.excerpt, "Hello *world*");
        assert_eq!(s.title, "Untitled");
        assert_eq!(
            inner_get_note(n.id, &pool).await.unwrap().body,
            "\n\n  # Hello *world*\nmore"
        );
    }

    #[tokio::test]
    async fn identical_body_leaves_updated_at() {
        let pool = crate::test_db().await;
        let n = inner_create_note(&pool).await.unwrap();
        inner_update_note_body(n.id, "same", &pool).await.unwrap();
        set_updated_at(n.id, OLD, &pool).await;

        let s = inner_update_note_body(n.id, "same", &pool).await.unwrap();
        assert_eq!(s.updated_at, OLD);
        assert_eq!(s.excerpt, "same");
    }

    #[tokio::test]
    async fn pinning_does_not_bump_updated_at() {
        let pool = crate::test_db().await;
        let n = inner_create_note(&pool).await.unwrap();
        set_updated_at(n.id, OLD, &pool).await;

        let s = inner_set_note_pinned(n.id, true, &pool).await.unwrap();
        assert!(s.pinned);
        assert_eq!(s.updated_at, OLD);
        let s = inner_set_note_pinned(n.id, false, &pool).await.unwrap();
        assert!(!s.pinned);
        assert_eq!(s.updated_at, OLD);
    }

    #[tokio::test]
    async fn list_orders_pinned_then_updated_desc_then_id() {
        let pool = crate::test_db().await;
        let a = note_titled("a", &pool).await;
        let b = note_titled("b", &pool).await;
        let c = note_titled("c", &pool).await;
        let d = note_titled("d", &pool).await;
        set_updated_at(a.id, "2026-01-01T00:00:00.000", &pool).await;
        set_updated_at(b.id, "2026-03-01T00:00:00.000", &pool).await;
        set_updated_at(c.id, "2026-02-01T00:00:00.000", &pool).await;
        set_updated_at(d.id, "2026-02-01T00:00:00.000", &pool).await;
        inner_set_note_pinned(a.id, true, &pool).await.unwrap();

        assert_eq!(titles(&pool).await, vec!["a", "b", "c", "d"]);
        let list = inner_list_notes(&pool).await.unwrap();
        assert!(list[0].pinned);
    }

    #[tokio::test]
    async fn missing_note_reports_not_found() {
        let pool = crate::test_db().await;
        let nf = "Note not found";
        assert_eq!(err_msg(inner_get_note(42, &pool).await), nf);
        assert_eq!(err_msg(inner_rename_note(42, "x", &pool).await), nf);
        assert_eq!(err_msg(inner_update_note_body(42, "x", &pool).await), nf);
        assert_eq!(err_msg(inner_set_note_pinned(42, true, &pool).await), nf);
        assert_eq!(err_msg(inner_delete_note(42, &pool).await), nf);
    }

    #[tokio::test]
    async fn delete_removes_note() {
        let pool = crate::test_db().await;
        let n = inner_create_note(&pool).await.unwrap();
        inner_delete_note(n.id, &pool).await.unwrap();
        assert!(inner_list_notes(&pool).await.unwrap().is_empty());
    }

    #[test]
    fn excerpt_skips_blank_fence_and_delimiter_lines() {
        assert_eq!(excerpt("", ""), "");
        assert_eq!(excerpt("\n  \n\t\n", ""), "");
        assert_eq!(excerpt("\n\nfirst\nsecond", ""), "first");
        assert_eq!(excerpt("```rust\nlet x = 1;\n```", ""), "let x = 1;");
        assert_eq!(excerpt("~~~\ncode\n~~~", ""), "code");
        assert_eq!(excerpt("|---|:-:|\n| a | b |", ""), "a | b");
        assert_eq!(excerpt("---\nafter rule", ""), "after rule");
    }

    #[test]
    fn excerpt_strips_leading_markers() {
        assert_eq!(excerpt("# Heading", ""), "Heading");
        assert_eq!(excerpt("###### Deep", ""), "Deep");
        assert_eq!(excerpt("#tag stays", ""), "#tag stays");
        assert_eq!(excerpt("> quoted", ""), "quoted");
        assert_eq!(excerpt("> > nested", ""), "nested");
        assert_eq!(excerpt("- item", ""), "item");
        assert_eq!(excerpt("* item", ""), "item");
        assert_eq!(excerpt("+ item", ""), "item");
        assert_eq!(excerpt("1. first", ""), "first");
        assert_eq!(excerpt("12) twelfth", ""), "twelfth");
        assert_eq!(excerpt("- [ ] todo", ""), "todo");
        assert_eq!(excerpt("- [x] done", ""), "done");
        assert_eq!(excerpt("> - [X] quoted task", ""), "quoted task");
        assert_eq!(excerpt("| Name | Age |", ""), "Name | Age");
        assert_eq!(
            excerpt("-\n#\n> \nreal", ""),
            "real",
            "marker-only lines skip"
        );
        assert_eq!(excerpt("2024 was a year", ""), "2024 was a year");
        assert_eq!(excerpt("-dash text", ""), "-dash text");
    }

    #[test]
    fn excerpt_skips_leading_heading_that_repeats_the_title() {
        assert_eq!(
            excerpt("# Project kickoff\nNotes here", "Project kickoff"),
            "Notes here"
        );
        assert_eq!(
            excerpt("# project KICKOFF\nNotes", "Project kickoff"),
            "Notes"
        );
        assert_eq!(
            excerpt("Kickoff\n===\nBody", "Kickoff"),
            "Body",
            "setext H1"
        );
        assert_eq!(
            excerpt("Kickoff\n---\nBody", "Kickoff"),
            "Body",
            "setext H2"
        );
        assert_eq!(
            excerpt("# Other\nBody", "Kickoff"),
            "Other",
            "different heading kept"
        );
        assert_eq!(
            excerpt("Kickoff\nBody", "Kickoff"),
            "Kickoff",
            "plain line kept"
        );
        assert_eq!(
            excerpt("Intro\n# Kickoff", "Kickoff"),
            "Intro",
            "only the first line"
        );
        assert_eq!(excerpt("# Kickoff", "Kickoff"), "", "heading only");
    }

    #[test]
    fn excerpt_truncates_at_120_chars_on_char_boundary() {
        let ascii = "a".repeat(200);
        assert_eq!(excerpt(&ascii, ""), "a".repeat(120));
        let multi = "å🎉".repeat(100); // multi-byte chars around the cut
        let e = excerpt(&multi, "");
        assert_eq!(e.chars().count(), 120);
        assert_eq!(e, "å🎉".repeat(60));
        let short = "é".repeat(120);
        assert_eq!(excerpt(&short, ""), short);
    }
}
