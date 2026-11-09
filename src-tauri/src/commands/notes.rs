use crate::note_links::{self, RefKind};
use crate::semantic::SemanticState;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection, SqliteExecutor};
use tauri::State;

/// Maximum title length in characters (not bytes).
pub const MAX_TITLE_CHARS: usize = 200;
/// Maximum excerpt length in characters (not bytes).
pub const MAX_EXCERPT_CHARS: usize = 120;
/// Characters that would break `[[wikilinks]]` and `#tags`.
/// A backtick could pair with another and turn a rewritten `[[link]]` into code.
const FORBIDDEN_TITLE_CHARS: [char; 5] = ['[', ']', '|', '#', '`'];
const UNTITLED: &str = "Untitled";
/// Version of the link extraction rules. Bump it when they change: the next start
/// rebuilds `note_refs` (see `db::backfill_note_refs`).
pub const NOTE_REFS_VERSION: &str = "1";
pub const NOTE_REFS_VERSION_KEY: &str = "note_refs_version";

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
    /// Manual order within the folder (display: pinned first, then `position`, then id).
    pub position: i64,
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
    pub position: i64,
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
            position: n.position,
        }
    }
}

const SELECT_NOTES: &str = "SELECT id,
        FolderId  AS folder_id,
        Title     AS title,
        Body      AS body,
        Pinned    AS pinned,
        CreatedAt AS created_at,
        UpdatedAt AS updated_at,
        Position  AS position
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
            "Title can't contain [ ] | # or `".to_string(),
        ));
    }
    Ok(t.to_string())
}

/// Lowercased titles of all notes except `except_id`. Rust's `to_lowercase` folds
/// non-ASCII letters too (`Å` → `å`), which SQLite's NOCASE does not.
async fn lowercase_titles(
    except_id: Option<i64>,
    executor: impl SqliteExecutor<'_>,
) -> Result<std::collections::HashSet<String>, AppError> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT Title FROM notes WHERE id IS NOT ?")
        .bind(except_id)
        .fetch_all(executor)
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

/// Writes a title and retargets `[[old title]]` links in all notes (this one included)
/// at it, in one transaction. Skips the Rust-side duplicate check; the unique index is
/// the backstop. Returns the ids of the notes whose bodies were rewritten.
async fn store_title(
    id: i64,
    old_title: &str,
    title: &str,
    pool: &Pool<Sqlite>,
) -> Result<Vec<i64>, AppError> {
    let mut tx = pool.begin().await?;
    // The write comes first, so the transaction holds the write lock before it reads.
    let result =
        sqlx::query("UPDATE notes SET Title = ?, UpdatedAt = ? WHERE id = ? AND Title IS NOT ?")
            .bind(title)
            .bind(now())
            .bind(id)
            .bind(title)
            .execute(&mut *tx)
            .await
            .map_err(|e| map_unique(e, title))?;
    let mut rewritten = Vec::new();
    if result.rows_affected() > 0 {
        index_note(id, &mut tx).await?;
        rewritten = rewrite_link_refs(&old_title.to_lowercase(), title, &mut tx).await?;
    }
    tx.commit().await?;
    Ok(rewritten)
}

/// Stores a body rewritten by a rename (not a user edit, so `UpdatedAt` stays) and
/// re-indexes it.
async fn store_rewritten_body(
    id: i64,
    body: &str,
    conn: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE notes SET Body = ? WHERE id = ?")
        .bind(body)
        .bind(id)
        .execute(&mut *conn)
        .await?;
    index_note(id, conn).await
}

/// Retargets wikilinks to `old_target` (lowercased title) at `new_title` in every note.
/// Returns the ids of the rewritten notes.
async fn rewrite_link_refs(
    old_target: &str,
    new_title: &str,
    conn: &mut SqliteConnection,
) -> Result<Vec<i64>, sqlx::Error> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT NoteId FROM note_refs WHERE Kind = 'link' AND Target = ? ORDER BY NoteId",
    )
    .bind(old_target)
    .fetch_all(&mut *conn)
    .await?;
    let mut rewritten = Vec::new();
    for id in ids {
        let body: String = sqlx::query_scalar("SELECT Body FROM notes WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if let Some(body) = note_links::rewrite_links(&body, old_target, new_title) {
            store_rewritten_body(id, &body, conn).await?;
            rewritten.push(id);
        }
    }
    Ok(rewritten)
}

/// Rewrites tags at or below `old_path` (e.g. `acme` or `acme/web`) in every note so the
/// path's last segment becomes `new_segment`. For customer/project renames; the caller
/// owns the transaction and has already written the rename. Returns the rewritten ids.
pub(crate) async fn rewrite_tag_refs(
    old_path: &str,
    new_segment: &str,
    conn: &mut SqliteConnection,
) -> Result<Vec<i64>, sqlx::Error> {
    let len = i64::try_from(old_path.chars().count()).unwrap_or(i64::MAX - 1);
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT DISTINCT NoteId FROM note_refs WHERE Kind = 'tag' \
         AND (Target = ? OR substr(Target, 1, ?) = ? || '/') ORDER BY NoteId",
    )
    .bind(old_path)
    .bind(len + 1)
    .bind(old_path)
    .fetch_all(&mut *conn)
    .await?;
    let mut rewritten = Vec::new();
    for id in ids {
        let body: String = sqlx::query_scalar("SELECT Body FROM notes WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *conn)
            .await?;
        if let Some(body) = note_links::rewrite_tags(&body, old_path, new_segment) {
            store_rewritten_body(id, &body, conn).await?;
            rewritten.push(id);
        }
    }
    Ok(rewritten)
}

/// The notes in `folder_id` (root = None) other than `except_id`, in display order, as
/// (pinned, unpinned) id lists.
pub(crate) async fn note_groups(
    folder_id: Option<i64>,
    except_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<(Vec<i64>, Vec<i64>), sqlx::Error> {
    let rows: Vec<(i64, bool)> = sqlx::query_as(
        "SELECT id, Pinned FROM notes WHERE FolderId IS ? AND id IS NOT ? \
         ORDER BY Pinned DESC, Position, id",
    )
    .bind(folder_id)
    .bind(except_id)
    .fetch_all(conn)
    .await?;
    let (pinned, unpinned): (Vec<_>, Vec<_>) = rows.into_iter().partition(|(_, p)| *p);
    Ok((
        pinned.into_iter().map(|(id, _)| id).collect(),
        unpinned.into_iter().map(|(id, _)| id).collect(),
    ))
}

/// Numbers the given notes 0..n in order (one folder's notes, pinned group first).
pub(crate) async fn write_note_positions(
    ids: &[i64],
    conn: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    for (position, id) in (0_i64..).zip(ids) {
        sqlx::query("UPDATE notes SET Position = ? WHERE id = ?")
            .bind(position)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Inserts an empty note without the Rust-side duplicate check; the unique index is the
/// backstop.
async fn insert_note(
    title: &str,
    folder_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<i64, AppError> {
    let mut tx = pool.begin().await?;
    let id = insert_note_in(title, "", folder_id, &mut tx).await?;
    tx.commit().await?;
    Ok(id)
}

/// Inserts a note with `body` and indexes it (search and links) inside the caller's
/// transaction, without the Rust-side duplicate check. The note goes to the top of its
/// folder (pinned notes still show above it).
async fn insert_note_in(
    title: &str,
    body: &str,
    folder_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<i64, AppError> {
    let ts = now();
    super::folders::ensure_folder_exists(folder_id, &mut *conn).await?;
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO notes (FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt, Position) \
         VALUES (?, ?, ?, 0, ?, ?, \
                 (SELECT COALESCE(MIN(Position) - 1, 0) FROM notes WHERE FolderId IS ?)) \
         RETURNING id",
    )
    .bind(folder_id)
    .bind(title)
    .bind(body)
    .bind(&ts)
    .bind(&ts)
    .bind(folder_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| map_unique(e, title))?;
    index_note(row.0, conn).await?;
    Ok(row.0)
}

/// Creates a note titled `title` with `body` inside the caller's transaction: the title
/// is validated and must be unique case-insensitively (non-ASCII too). Returns its id.
/// The caller queues it for the semantic index after committing.
pub(crate) async fn insert_titled_note(
    title: &str,
    body: &str,
    folder_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<i64, AppError> {
    let title = clean_title(title)?;
    if lowercase_titles(None, &mut *conn)
        .await?
        .contains(&title.to_lowercase())
    {
        return Err(duplicate(&title));
    }
    insert_note_in(&title, body, folder_id, conn).await
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// All notes without bodies: pinned first, then manual position, then id (the tree order
/// within each folder).
pub async fn inner_list_notes(pool: &Pool<Sqlite>) -> Result<Vec<NoteSummary>, AppError> {
    let sql = format!("{SELECT_NOTES} ORDER BY Pinned DESC, Position, id");
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

/// Creates an empty note in `folder_id` (root = None) titled with the first free
/// `Untitled[ n]`.
pub async fn inner_create_note(
    folder_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Note, AppError> {
    let title = first_free_untitled(&lowercase_titles(None, pool).await?);
    let id = insert_note(&title, folder_id, pool).await?;
    inner_get_note(id, pool).await
}

/// Creates an empty note with `title` in `folder_id` (following an unresolved
/// `[[wikilink]]`; the frontend passes the linking note's folder).
pub async fn inner_create_note_titled(
    title: &str,
    folder_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Note, AppError> {
    let mut tx = pool.begin().await?;
    let id = insert_titled_note(title, "", folder_id, &mut tx).await?;
    tx.commit().await?;
    inner_get_note(id, pool).await
}

/// `rename_note` result: the renamed note and the notes whose `[[links]]` to the old
/// title were rewritten (the renamed note itself included when it links to itself).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RenamedNote {
    pub note: NoteSummary,
    pub rewritten_ids: Vec<i64>,
}

/// Renames a note and retargets wikilinks to its old title, keeping aliases. Titles are
/// unique case-insensitively (including non-ASCII). An unchanged title is a no-op that
/// keeps `UpdatedAt`.
pub async fn inner_rename_note(
    id: i64,
    title: &str,
    pool: &Pool<Sqlite>,
) -> Result<RenamedNote, AppError> {
    let title = clean_title(title)?;
    let old = inner_get_note(id, pool).await?;
    if lowercase_titles(Some(id), pool)
        .await?
        .contains(&title.to_lowercase())
    {
        return Err(duplicate(&title));
    }
    let rewritten_ids = store_title(id, &old.title, &title, pool).await?;
    Ok(RenamedNote {
        note: get_summary(id, pool).await?,
        rewritten_ids,
    })
}

/// Replaces the body. An identical body is a no-op that keeps `UpdatedAt`.
pub async fn inner_update_note_body(
    id: i64,
    body: &str,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    // 0 rows affected means unchanged or missing; get_summary tells them apart.
    let mut tx = pool.begin().await?;
    let result =
        sqlx::query("UPDATE notes SET Body = ?, UpdatedAt = ? WHERE id = ? AND Body IS NOT ?")
            .bind(body)
            .bind(now())
            .bind(id)
            .bind(body)
            .execute(&mut *tx)
            .await?;
    if result.rows_affected() > 0 {
        index_note(id, &mut tx).await?;
    }
    tx.commit().await?;
    get_summary(id, pool).await
}

/// Pins or unpins a note without touching `UpdatedAt`. It lands on the boundary between
/// the groups: pinning puts it at the bottom of the pinned notes, unpinning at the top of
/// the unpinned ones. Its folder's positions are renumbered densely.
pub async fn inner_set_note_pinned(
    id: i64,
    pinned: bool,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    let mut tx = pool.begin().await?;
    let row: Option<(Option<i64>, bool)> =
        sqlx::query_as("SELECT FolderId, Pinned FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
    let Some((folder_id, was_pinned)) = row else {
        return Err(not_found());
    };
    if was_pinned != pinned {
        sqlx::query("UPDATE notes SET Pinned = ? WHERE id = ?")
            .bind(pinned)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        let (mut order, unpinned) = note_groups(folder_id, Some(id), &mut tx).await?;
        order.push(id);
        order.extend(unpinned);
        write_note_positions(&order, &mut tx).await?;
    }
    tx.commit().await?;
    get_summary(id, pool).await
}

pub async fn inner_delete_note(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query("DELETE FROM notes WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

// ── Search ────────────────────────────────────────────────────────────────────

/// Search result: a list row plus the title and a body snippet with hits wrapped in
/// `HIT_START`/`HIT_END`. `snippet` is empty when the body has no highlighted hit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NoteHit {
    #[serde(flatten)]
    pub note: NoteSummary,
    pub title_marked: String,
    pub snippet: String,
    /// A semantic-only hit (no keyword match): `snippet` is its best chunk, unmarked.
    #[serde(default)]
    pub related: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SearchIndexStats {
    pub notes: i64,
    pub bytes: i64,
}

/// Highlight delimiters (STX/ETX). Stripped from indexed text, so they only ever come
/// from FTS5 and user content can't fake a highlight.
pub const HIT_START: char = '\u{2}';
pub const HIT_END: char = '\u{3}';
/// The trigram tokenizer can't match terms shorter than this.
const MIN_TRIGRAM_CHARS: usize = 3;
/// Snippet length in tokens; trigram tokens are roughly one character each.
const SNIPPET_TOKENS: i64 = 64;
/// bm25 column weights: a title hit counts ten times a body hit.
const TITLE_WEIGHT: f64 = 10.0;

fn strip_hit_markers(text: &str) -> String {
    text.replace([HIT_START, HIT_END], "")
}

/// Writes (or rewrites) one note's index row from its stored title and body.
async fn index_note(id: i64, conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM notes_fts WHERE rowid = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT Title, Body FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
    if let Some((title, body)) = row {
        insert_index_row(id, &title, &body, conn).await?;
        index_refs(id, &body, conn).await?;
    }
    Ok(())
}

/// Replaces one note's rows in `note_refs` with the tags and links in `body`.
async fn index_refs(id: i64, body: &str, conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM note_refs WHERE NoteId = ?")
        .bind(id)
        .execute(&mut *conn)
        .await?;
    let mut seen = std::collections::HashSet::new();
    for r in note_links::extract(body) {
        if !seen.insert((r.kind, r.target.clone())) {
            continue;
        }
        sqlx::query("INSERT INTO note_refs (NoteId, Kind, Target) VALUES (?, ?, ?)")
            .bind(id)
            .bind(r.kind.as_str())
            .bind(&r.target)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Replaces the whole link index with rows built from `notes` and records its version.
pub(crate) async fn rebuild_refs(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM note_refs")
        .execute(&mut *conn)
        .await?;
    let notes: Vec<(i64, String)> = sqlx::query_as("SELECT id, Body FROM notes")
        .fetch_all(&mut *conn)
        .await?;
    for (id, body) in notes {
        index_refs(id, &body, conn).await?;
    }
    sqlx::query(
        "INSERT INTO app_settings (Key, Value) VALUES (?, ?)
         ON CONFLICT(Key) DO UPDATE SET Value = excluded.Value",
    )
    .bind(NOTE_REFS_VERSION_KEY)
    .bind(NOTE_REFS_VERSION)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn insert_index_row(
    id: i64,
    title: &str,
    body: &str,
    conn: &mut SqliteConnection,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO notes_fts (rowid, Title, Body) VALUES (?, ?, ?)")
        .bind(id)
        .bind(strip_hit_markers(title))
        .bind(strip_hit_markers(&super::markdown::to_plain_text(body)))
        .execute(conn)
        .await?;
    Ok(())
}

/// Replaces both derived indexes (search and links) with rows built from `notes`.
/// Callers own the transaction, so restore and seeding rebuild atomically with their writes.
pub(crate) async fn rebuild_index(conn: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM notes_fts")
        .execute(&mut *conn)
        .await?;
    let notes: Vec<(i64, String, String)> = sqlx::query_as("SELECT id, Title, Body FROM notes")
        .fetch_all(&mut *conn)
        .await?;
    for (id, title, body) in notes {
        insert_index_row(id, &title, &body, conn).await?;
    }
    rebuild_refs(conn).await
}

/// Quotes a term as an FTS5 string so operators and punctuation are literal.
fn fts_phrase(term: &str) -> String {
    format!("\"{}\"", term.replace('"', "\"\""))
}

#[derive(FromRow)]
struct HitRow {
    #[sqlx(flatten)]
    note: Note,
    plain_title: String,
    plain_body: String,
    title_marked: String,
    snippet: String,
}

/// Search over titles and plain-text bodies. A `#tag` term filters by tag (itself or
/// nested below it); every other whitespace-separated term must occur as a
/// case-insensitive substring. All filters must match. Terms of 3+ chars use the trigram
/// index (ranked, highlighted); shorter terms filter the rows in Rust. Without a 3+ char
/// term: no ranking or highlights, title matches first, then most recently updated.
/// A blank query returns nothing; `limit` caps the result count.
pub async fn inner_search_notes(
    query: &str,
    limit: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Vec<NoteHit>, AppError> {
    let mut tags: Vec<String> = Vec::new();
    let mut terms: Vec<String> = Vec::new();
    for term in query.split_whitespace() {
        match note_links::tag_term(term) {
            Some(tag) => tags.push(tag),
            None => terms.push(strip_hit_markers(term)),
        }
    }
    terms.retain(|t| !t.is_empty());
    if terms.is_empty() && tags.is_empty() {
        return Ok(Vec::new());
    }
    let (long, short): (Vec<&String>, Vec<&String>) = terms
        .iter()
        .partition(|t| t.chars().count() >= MIN_TRIGRAM_CHARS);

    let mut sql = String::from(
        "SELECT n.id, n.FolderId AS folder_id, n.Title AS title, n.Body AS body,
                n.Pinned AS pinned, n.CreatedAt AS created_at, n.UpdatedAt AS updated_at,
                n.Position AS position, notes_fts.Title AS plain_title, notes_fts.Body AS plain_body,",
    );
    if long.is_empty() {
        sql.push_str(" n.Title AS title_marked, '' AS snippet");
    } else {
        sql.push_str(&format!(
            " highlight(notes_fts, 0, char(2), char(3)) AS title_marked,
              snippet(notes_fts, 1, char(2), char(3), '…', {SNIPPET_TOKENS}) AS snippet"
        ));
    }
    sql.push_str(" FROM notes_fts JOIN notes n ON n.id = notes_fts.rowid");
    let mut filters: Vec<&str> = Vec::new();
    if !long.is_empty() {
        filters.push("notes_fts MATCH ?");
    }
    // Nested match without LIKE wildcards: the tag itself or `tag/…`.
    filters.extend(tags.iter().map(|_| {
        "n.id IN (SELECT NoteId FROM note_refs WHERE Kind = 'tag' \
         AND (Target = ? OR substr(Target, 1, ?) = ? || '/'))"
    }));
    if !filters.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&filters.join(" AND "));
    }
    if long.is_empty() {
        sql.push_str(" ORDER BY n.UpdatedAt DESC, n.id");
    } else {
        sql.push_str(&format!(
            " ORDER BY bm25(notes_fts, {TITLE_WEIGHT}, 1.0), n.UpdatedAt DESC, n.id"
        ));
    }
    // Short terms filter in Rust (Unicode case folding; SQLite's LIKE folds ASCII only),
    // so the limit applies after filtering.
    let sql_limit = if short.is_empty() { limit } else { None };
    sql.push_str(" LIMIT ?");

    let mut q = sqlx::query_as::<_, HitRow>(&sql);
    if !long.is_empty() {
        let phrases: Vec<String> = long.iter().map(|t| fts_phrase(t)).collect();
        q = q.bind(phrases.join(" "));
    }
    for tag in &tags {
        let len = i64::try_from(tag.chars().count()).unwrap_or(i64::MAX - 1);
        q = q.bind(tag).bind(len + 1).bind(tag);
    }
    q = q.bind(sql_limit.unwrap_or(-1));
    let rows = q.fetch_all(pool).await?;

    let short: Vec<String> = short.iter().map(|t| t.to_lowercase()).collect();
    let mut rows: Vec<(HitRow, String)> = rows
        .into_iter()
        .map(|r| {
            let title = r.plain_title.to_lowercase();
            (r, title)
        })
        .filter(|(r, title)| {
            let body = r.plain_body.to_lowercase();
            short.iter().all(|t| title.contains(t) || body.contains(t))
        })
        .collect();
    if let (true, Some(first)) = (long.is_empty(), short.first()) {
        // No ranking without MATCH: title matches first (stable, so most recent next).
        rows.sort_by_key(|(_, title)| !title.contains(first));
    }
    let cap = limit
        .and_then(|l| usize::try_from(l).ok())
        .unwrap_or(usize::MAX);
    Ok(rows
        .into_iter()
        .take(cap)
        .map(|(r, _)| NoteHit {
            note: NoteSummary::from(r.note),
            title_marked: r.title_marked,
            snippet: if r.snippet.contains(HIT_START) {
                r.snippet
            } else {
                String::new()
            },
            related: false,
        })
        .collect())
}

/// Maximum length of a semantic (chunk) snippet in characters.
const SEMANTIC_SNIPPET_CHARS: usize = 160;

/// Shortens chunk text for a snippet: whole words, `…` when cut.
fn semantic_snippet(text: &str) -> String {
    let text = strip_hit_markers(text);
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= SEMANTIC_SNIPPET_CHARS {
        return text;
    }
    let cut: String = text.chars().take(SEMANTIC_SNIPPET_CHARS).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > SEMANTIC_SNIPPET_CHARS / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end())
}

/// Ids among `ids` that are still notes, in the given order.
async fn existing_note_ids(ids: Vec<i64>, pool: &Pool<Sqlite>) -> Result<Vec<i64>, AppError> {
    if ids.is_empty() {
        return Ok(ids);
    }
    let json = format!(
        "[{}]",
        ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",")
    );
    let existing: std::collections::HashSet<i64> =
        sqlx::query_scalar("SELECT id FROM notes WHERE id IN (SELECT value FROM json_each(?))")
            .bind(json)
            .fetch_all(pool)
            .await?
            .into_iter()
            .collect();
    Ok(ids.into_iter().filter(|id| existing.contains(id)).collect())
}

/// Ids among `ids` tagged with every tag in `tags` (or below it), like the keyword filter.
async fn filter_by_tags(
    ids: Vec<i64>,
    tags: &[String],
    pool: &Pool<Sqlite>,
) -> Result<Vec<i64>, AppError> {
    if tags.is_empty() || ids.is_empty() {
        return Ok(ids);
    }
    let mut keep: std::collections::HashSet<i64> = ids.iter().copied().collect();
    for tag in tags {
        let len = i64::try_from(tag.chars().count()).unwrap_or(i64::MAX - 1);
        let tagged: std::collections::HashSet<i64> = sqlx::query_scalar(
            "SELECT DISTINCT NoteId FROM note_refs WHERE Kind = 'tag' \
             AND (Target = ? OR substr(Target, 1, ?) = ? || '/')",
        )
        .bind(tag)
        .bind(len + 1)
        .bind(tag)
        .fetch_all(pool)
        .await?
        .into_iter()
        .collect();
        keep.retain(|id| tagged.contains(id));
    }
    Ok(ids.into_iter().filter(|id| keep.contains(id)).collect())
}

/// Keyword search fused with semantic search (RRF, k = 60, depth 50 per leg). Falls back
/// to exactly `inner_search_notes` when semantic search isn't ready, the query (without
/// `#tag` terms) is under 4 characters, or the query can't be embedded. `#tag` terms
/// filter both legs. Semantic-only hits carry `related` and their best chunk as snippet;
/// keyword hits past the fusion depth follow the fused list in keyword order.
pub async fn inner_search_notes_hybrid(
    query: &str,
    limit: Option<i64>,
    pool: &Pool<Sqlite>,
    semantic: &crate::semantic::SemanticState,
) -> Result<Vec<NoteHit>, AppError> {
    use crate::semantic::{embedder::query_text, fusion};

    let text = match fusion::semantic_query(&strip_hit_markers(query)) {
        Some(text) if semantic.is_ready() => text,
        _ => return inner_search_notes(query, limit, pool).await,
    };
    let cap = limit
        .and_then(|l| usize::try_from(l).ok())
        .unwrap_or(usize::MAX);
    let keyword_limit = limit.map(|l| l.max(fusion::DEPTH as i64));
    let keyword = inner_search_notes(query, keyword_limit, pool).await?;

    let vector = match semantic.embed(vec![query_text(&text)]).await {
        Ok(mut v) if !v.is_empty() => v.swap_remove(0),
        Ok(_) => return Ok(keyword.into_iter().take(cap).collect()),
        Err(e) => {
            tracing::error!("Semantic search: couldn't embed the query: {e}");
            return Ok(keyword.into_iter().take(cap).collect());
        }
    };
    // The floor is relative to the median over every indexed note, so it is taken before
    // the tag filter and the depth cut.
    let nearest = semantic.nearest(&vector);
    let best_chunk: std::collections::HashMap<i64, i64> =
        nearest.iter().map(|(id, chunk, _)| (*id, *chunk)).collect();
    let tags: Vec<String> = query
        .split_whitespace()
        .filter_map(note_links::tag_term)
        .collect();
    let nearest_ids =
        existing_note_ids(nearest.iter().map(|(id, _, _)| *id).collect(), pool).await?;
    if nearest_ids.len() < nearest.len() {
        // Cached vectors of a note deleted under the worker: drop them so they don't take
        // a slot within the fusion depth or count as indexed.
        let kept: std::collections::HashSet<i64> = nearest_ids.iter().copied().collect();
        for (id, _, _) in nearest.iter().filter(|(id, _, _)| !kept.contains(id)) {
            semantic.evict_cache(*id);
        }
    }
    let mut semantic_ids = filter_by_tags(nearest_ids, &tags, pool).await?;
    semantic_ids.truncate(fusion::DEPTH);

    let keyword_ids: Vec<i64> = keyword.iter().map(|h| h.note.id).collect();
    let mut order = fusion::rrf(&keyword_ids, &semantic_ids);
    let fused: std::collections::HashSet<i64> = order.iter().copied().collect();
    order.extend(keyword_ids.iter().filter(|id| !fused.contains(id)));

    let mut by_id: std::collections::HashMap<i64, NoteHit> =
        keyword.into_iter().map(|h| (h.note.id, h)).collect();
    let mut hits = Vec::new();
    for id in order {
        if hits.len() >= cap {
            break;
        }
        if let Some(hit) = by_id.remove(&id) {
            hits.push(hit);
            continue;
        }
        // Semantic only: the note may have been deleted since it was embedded.
        let Ok(note) = inner_get_note(id, pool).await else {
            continue;
        };
        let chunk: Option<String> = sqlx::query_scalar(
            "SELECT ChunkText FROM note_embeddings WHERE NoteId = ? AND ChunkIndex = ?",
        )
        .bind(id)
        .bind(best_chunk.get(&id).copied().unwrap_or(0))
        .fetch_optional(pool)
        .await?;
        hits.push(NoteHit {
            title_marked: strip_hit_markers(&note.title),
            snippet: semantic_snippet(chunk.as_deref().unwrap_or("")),
            note: NoteSummary::from(note),
            related: true,
        });
    }
    Ok(hits)
}

pub async fn inner_search_index_stats(pool: &Pool<Sqlite>) -> Result<SearchIndexStats, AppError> {
    let (notes,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes_fts")
        .fetch_one(pool)
        .await?;
    let (bytes,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat WHERE name LIKE 'notes\\_fts%' ESCAPE '\\'",
    )
    .fetch_one(pool)
    .await?;
    Ok(SearchIndexStats { notes, bytes })
}

pub async fn inner_rebuild_search_index(pool: &Pool<Sqlite>) -> Result<SearchIndexStats, AppError> {
    let mut tx = pool.begin().await?;
    rebuild_index(&mut tx).await?;
    tx.commit().await?;
    inner_search_index_stats(pool).await
}

// ── Links ─────────────────────────────────────────────────────────────────────

/// A `#` autocomplete entry. `tag` is what follows the `#` when inserted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TagSuggestion {
    /// `tag` (in use in some note), `customer`, `project`, `issue` or `todo`.
    pub kind: String,
    pub tag: String,
    /// Display text: the tag, the customer or project name, the issue as
    /// "title — customer / project", or the todo title.
    pub label: String,
    /// Projects: the customer name.
    pub detail: Option<String>,
    /// Todos: `not_started` or `in_progress`.
    pub status: Option<String>,
}

/// A note that links to another, with the line that links.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Backlink {
    pub id: i64,
    pub title: String,
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct NoteTarget {
    pub id: i64,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct IssueTarget {
    pub id: i64,
    pub title: String,
    /// "Customer / Project".
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromRow)]
pub struct TodoTarget {
    pub id: i64,
    pub title: String,
    pub status: String,
}

/// What wikilinks and `#issue/<id>` / `#todo/<id>` chips resolve to (editor live preview).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LinkTargets {
    pub notes: Vec<NoteTarget>,
    pub issues: Vec<IssueTarget>,
    pub todos: Vec<TodoTarget>,
}

/// Customer and project tags by slug path. A clashing slug belongs to the lowest id;
/// a project has a tag only when its customer owns the customer slug.
#[derive(Debug, Default)]
pub(crate) struct EntityTags {
    /// `cslug` → (customer id, name).
    pub customers: std::collections::HashMap<String, (i64, String)>,
    /// `cslug/pslug` → (project id, customer name, project name).
    pub projects: std::collections::HashMap<String, (i64, String, String)>,
}

pub(crate) async fn load_entity_tags(
    conn: &mut SqliteConnection,
) -> Result<EntityTags, sqlx::Error> {
    let mut tags = EntityTags::default();
    let customers: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, Name FROM customers ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    let mut customer_slugs = std::collections::HashMap::new();
    for (id, name) in customers {
        if let Some(s) = note_links::customer_slug(&name) {
            if let std::collections::hash_map::Entry::Vacant(e) = tags.customers.entry(s.clone()) {
                customer_slugs.insert(id, (s, name.clone()));
                e.insert((id, name));
            }
        }
    }
    let projects: Vec<(i64, i64, String)> =
        sqlx::query_as("SELECT id, CustomerId, Name FROM projects ORDER BY id")
            .fetch_all(&mut *conn)
            .await?;
    for (id, customer_id, name) in projects {
        let (Some((cslug, cname)), Some(pslug)) =
            (customer_slugs.get(&customer_id), note_links::slug(&name))
        else {
            continue;
        };
        tags.projects
            .entry(format!("{cslug}/{pslug}"))
            .or_insert((id, cname.clone(), name));
    }
    Ok(tags)
}

/// Everything `#` autocomplete offers: tags in use (not already an entity's tag),
/// customers, projects, all issues and the todos that aren't done.
pub async fn inner_list_tag_suggestions(
    pool: &Pool<Sqlite>,
) -> Result<Vec<TagSuggestion>, AppError> {
    let mut conn = pool.acquire().await?;
    let entities = load_entity_tags(&mut conn).await?;
    let suggestion = |kind: &str, tag: String, label: String| TagSuggestion {
        kind: kind.to_string(),
        tag,
        label,
        detail: None,
        status: None,
    };

    let mut customers: Vec<TagSuggestion> = entities
        .customers
        .iter()
        .map(|(tag, (_, name))| suggestion("customer", tag.clone(), name.clone()))
        .collect();
    customers.sort_by_key(|s| s.label.to_lowercase());
    let mut projects: Vec<TagSuggestion> = entities
        .projects
        .iter()
        .map(|(tag, (_, customer, name))| TagSuggestion {
            detail: Some(customer.clone()),
            ..suggestion("project", tag.clone(), name.clone())
        })
        .collect();
    projects.sort_by_key(|s| {
        (
            s.detail.clone().unwrap_or_default().to_lowercase(),
            s.label.to_lowercase(),
        )
    });

    let issues: Vec<(i64, String, String, String)> = sqlx::query_as(
        "SELECT i.id, i.Title, c.Name, p.Name FROM issues i
         JOIN projects p ON p.id = i.ProjectId
         JOIN customers c ON c.id = p.CustomerId
         ORDER BY c.Name, p.Name, i.Title, i.id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let issues = issues.into_iter().map(|(id, title, customer, project)| {
        suggestion(
            "issue",
            format!("issue/{id}"),
            format!("{title} — {customer} / {project}"),
        )
    });
    let todos: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT id, Title, Status FROM todos WHERE Status != 'done' ORDER BY Status DESC, Position, id",
    )
    .fetch_all(&mut *conn)
    .await?;
    let todos = todos.into_iter().map(|(id, title, status)| TagSuggestion {
        status: Some(status),
        ..suggestion("todo", format!("todo/{id}"), title)
    });

    let in_use: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT Target FROM note_refs WHERE Kind = 'tag' ORDER BY Target",
    )
    .fetch_all(&mut *conn)
    .await?;
    let in_use = in_use
        .into_iter()
        .filter(|t| {
            !entities.customers.contains_key(t)
                && !entities.projects.contains_key(t)
                && note_links::entity_tag(t).is_none()
        })
        .map(|t| suggestion("tag", t.clone(), t));

    Ok(in_use
        .chain(customers)
        .chain(projects)
        .chain(issues)
        .chain(todos)
        .collect())
}

/// Note titles, issues and todos for resolving wikilinks and chips in the editor.
pub async fn inner_get_link_targets(pool: &Pool<Sqlite>) -> Result<LinkTargets, AppError> {
    let notes = sqlx::query_as::<_, NoteTarget>("SELECT id, Title AS title FROM notes ORDER BY id")
        .fetch_all(pool)
        .await?;
    let issues = sqlx::query_as::<_, IssueTarget>(
        "SELECT i.id, i.Title AS title, c.Name || ' / ' || p.Name AS context FROM issues i
         JOIN projects p ON p.id = i.ProjectId
         JOIN customers c ON c.id = p.CustomerId
         ORDER BY i.id",
    )
    .fetch_all(pool)
    .await?;
    let todos = sqlx::query_as::<_, TodoTarget>(
        "SELECT id, Title AS title, Status AS status FROM todos ORDER BY id",
    )
    .fetch_all(pool)
    .await?;
    Ok(LinkTargets {
        notes,
        issues,
        todos,
    })
}

/// The line of `body` around its first wikilink to `target`: markers stripped, at most
/// `MAX_EXCERPT_CHARS` characters, starting a little before the link when cut.
fn backlink_snippet(body: &str, target: &str) -> String {
    let Some(at) = note_links::extract(body)
        .into_iter()
        .find(|r| r.kind == RefKind::Link && r.target == target)
        .map(|r| r.range.start)
    else {
        return String::new();
    };
    let line_start = body[..at].rfind('\n').map_or(0, |p| p + 1);
    let line_end = body[at..].find('\n').map_or(body.len(), |p| at + p);
    let mut text = body[line_start..line_end].trim();
    while let Some(rest) = strip_marker(text) {
        text = rest.trim_start();
    }
    let text = text.strip_suffix('|').unwrap_or(text).trim();
    if text.chars().count() <= MAX_EXCERPT_CHARS {
        return text.to_string();
    }
    // `text` is a subslice of `body`: start up to 40 chars before the link.
    let text_start = text.as_ptr() as usize - body.as_ptr() as usize;
    let link_char = text[..at.saturating_sub(text_start).min(text.len())]
        .chars()
        .count();
    let skip = link_char.saturating_sub(40);
    let window: String = text.chars().skip(skip).take(MAX_EXCERPT_CHARS).collect();
    let mut out = String::new();
    if skip > 0 {
        out.push('…');
    }
    out.push_str(window.trim());
    if skip + MAX_EXCERPT_CHARS < text.chars().count() {
        out.push('…');
    }
    out
}

/// Notes that link to note `id` (title matched case-insensitively), excluding itself,
/// most recently updated first.
pub async fn inner_list_backlinks(id: i64, pool: &Pool<Sqlite>) -> Result<Vec<Backlink>, AppError> {
    let note = inner_get_note(id, pool).await?;
    let target = note.title.to_lowercase();
    let rows: Vec<(i64, String, String)> = sqlx::query_as(
        "SELECT n.id, n.Title, n.Body FROM notes n
         JOIN note_refs r ON r.NoteId = n.id
         WHERE r.Kind = 'link' AND r.Target = ? AND n.id != ?
         ORDER BY n.UpdatedAt DESC, n.id",
    )
    .bind(&target)
    .bind(id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, title, body)| Backlink {
            id,
            title,
            snippet: backlink_snippet(&body, &target),
        })
        .collect())
}

// ── Semantic index hooks ──────────────────────────────────────────────────────
// Note writes that change embedded text queue the note(s) after their transaction
// commits; delete evicts the cached vectors (the rows go with the note via cascade).

pub async fn create_note_and_queue(
    folder_id: Option<i64>,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<Note, AppError> {
    let note = inner_create_note(folder_id, pool).await?;
    semantic.enqueue([note.id]);
    Ok(note)
}

pub async fn create_note_titled_and_queue(
    title: &str,
    folder_id: Option<i64>,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<Note, AppError> {
    let note = inner_create_note_titled(title, folder_id, pool).await?;
    semantic.enqueue([note.id]);
    Ok(note)
}

/// Queues the renamed note and every note whose links were rewritten.
pub async fn rename_note_and_queue(
    id: i64,
    title: &str,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<RenamedNote, AppError> {
    let renamed = inner_rename_note(id, title, pool).await?;
    semantic.enqueue(std::iter::once(id).chain(renamed.rewritten_ids.iter().copied()));
    Ok(renamed)
}

pub async fn update_note_body_and_queue(
    id: i64,
    body: &str,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<NoteSummary, AppError> {
    let summary = inner_update_note_body(id, body, pool).await?;
    semantic.enqueue([id]);
    Ok(summary)
}

pub async fn delete_note_and_evict(
    id: i64,
    pool: &Pool<Sqlite>,
    semantic: &SemanticState,
) -> Result<(), AppError> {
    inner_delete_note(id, pool).await?;
    semantic.evict(id);
    Ok(())
}

/// After a customer or project rename that may have rewritten `#tags` in note bodies:
/// queue every note (unchanged ones are skipped by hash). The rename has committed, so
/// a failure here is logged, not returned.
pub async fn queue_all_after_tag_rename(pool: &Pool<Sqlite>, semantic: &SemanticState) {
    if !semantic.is_enabled() {
        return;
    }
    if let Err(e) = semantic.enqueue_all(pool).await {
        tracing::error!("Semantic search: couldn't queue notes after a rename: {e}");
    }
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
pub async fn create_note(
    folder_id: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<Note, AppError> {
    create_note_and_queue(folder_id, pool.inner(), semantic.inner()).await
}

#[tauri::command]
pub async fn create_note_titled(
    title: String,
    folder_id: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<Note, AppError> {
    create_note_titled_and_queue(&title, folder_id, pool.inner(), semantic.inner()).await
}

#[tauri::command]
pub async fn rename_note(
    id: i64,
    title: String,
    app: tauri::AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<RenamedNote, AppError> {
    let renamed = rename_note_and_queue(id, &title, pool.inner(), semantic.inner()).await?;
    // Linked reminders show the note's title.
    crate::reminders_scheduler::reminders_changed(&app, pool.inner()).await;
    Ok(renamed)
}

#[tauri::command]
pub async fn update_note_body(
    id: i64,
    body: String,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<NoteSummary, AppError> {
    update_note_body_and_queue(id, &body, pool.inner(), semantic.inner()).await
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
pub async fn delete_note(
    id: i64,
    app: tauri::AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<(), AppError> {
    delete_note_and_evict(id, pool.inner(), semantic.inner()).await?;
    // Its reminders cascade away.
    crate::reminders_scheduler::reminders_changed(&app, pool.inner()).await;
    Ok(())
}

/// `hybrid` (default false) adds semantic hits when semantic search is ready.
#[tauri::command]
pub async fn search_notes(
    query: String,
    limit: Option<i64>,
    hybrid: Option<bool>,
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, SemanticState>,
) -> Result<Vec<NoteHit>, AppError> {
    if hybrid.unwrap_or(false) {
        inner_search_notes_hybrid(&query, limit, pool.inner(), semantic.inner()).await
    } else {
        inner_search_notes(&query, limit, pool.inner()).await
    }
}

#[tauri::command]
pub async fn search_index_stats(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SearchIndexStats, AppError> {
    inner_search_index_stats(pool.inner()).await
}

#[tauri::command]
pub async fn rebuild_search_index(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SearchIndexStats, AppError> {
    inner_rebuild_search_index(pool.inner()).await
}

#[tauri::command]
pub async fn list_tag_suggestions(
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<TagSuggestion>, AppError> {
    inner_list_tag_suggestions(pool.inner()).await
}

#[tauri::command]
pub async fn get_link_targets(pool: State<'_, Pool<Sqlite>>) -> Result<LinkTargets, AppError> {
    inner_get_link_targets(pool.inner()).await
}

#[tauri::command]
pub async fn list_backlinks(
    id: i64,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<Backlink>, AppError> {
    inner_list_backlinks(id, pool.inner()).await
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
        let n = inner_create_note(None, pool).await.unwrap();
        inner_rename_note(n.id, title, pool).await.unwrap();
        inner_get_note(n.id, pool).await.unwrap()
    }

    #[tokio::test]
    async fn create_numbers_untitled_and_refills_gaps() {
        let pool = crate::test_db().await;
        let a = inner_create_note(None, &pool).await.unwrap();
        let b = inner_create_note(None, &pool).await.unwrap();
        let c = inner_create_note(None, &pool).await.unwrap();
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
        let d = inner_create_note(None, &pool).await.unwrap();
        assert_eq!(d.title, "Untitled 2", "first free number is reused");

        // case-insensitive: a note named "untitled 4" blocks "Untitled 4"
        inner_rename_note(d.id, "untitled 4", &pool).await.unwrap();
        let e = inner_create_note(None, &pool).await.unwrap();
        assert_eq!(e.title, "Untitled 2");
        let f = inner_create_note(None, &pool).await.unwrap();
        assert_eq!(f.title, "Untitled 5");
    }

    #[tokio::test]
    async fn rename_validates_title() {
        let pool = crate::test_db().await;
        let n = inner_create_note(None, &pool).await.unwrap();
        let rename = |t: String| {
            let pool = pool.clone();
            async move { inner_rename_note(n.id, &t, &pool).await }
        };
        assert_eq!(err_msg(rename("".into()).await), "Title is required");
        assert_eq!(err_msg(rename("  \t ".into()).await), "Title is required");
        assert_eq!(err_msg(rename("x".repeat(201)).await), "Title is too long");
        // 200 multi-byte chars are fine (limit counts chars, not bytes)
        let long = "å".repeat(200);
        assert_eq!(rename(long.clone()).await.unwrap().note.title, long);
        for bad in ["a[b", "a]b", "a|b", "#tag", "a`b"] {
            assert_eq!(
                err_msg(rename(bad.into()).await),
                "Title can't contain [ ] | # or `",
                "{bad}"
            );
        }
        let ok = rename("  Meeting notes 🎉  ".into()).await.unwrap().note;
        assert_eq!(ok.title, "Meeting notes 🎉");
    }

    #[tokio::test]
    async fn rename_rejects_case_insensitive_duplicates_including_non_ascii() {
        let pool = crate::test_db().await;
        note_titled("Åsa", &pool).await;
        note_titled("Plan", &pool).await;
        let n = inner_create_note(None, &pool).await.unwrap();

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

        let same = inner_rename_note(n.id, "åsa", &pool).await.unwrap().note;
        assert_eq!(same.updated_at, OLD, "unchanged title must not bump");

        let renamed = inner_rename_note(n.id, "Åsa", &pool).await.unwrap().note;
        assert_eq!(renamed.title, "Åsa");
        assert_ne!(renamed.updated_at, OLD, "title change bumps UpdatedAt");
    }

    #[tokio::test]
    async fn unique_violation_race_maps_to_duplicate_message() {
        let pool = crate::test_db().await;
        let a = note_titled("Plan", &pool).await;
        let b = inner_create_note(None, &pool).await.unwrap();

        // Skip the Rust-side check, as a concurrent writer would: the index catches it.
        assert_eq!(
            err_msg(store_title(b.id, &b.title, "plan", &pool).await),
            "A note named \"plan\" already exists"
        );
        assert_eq!(
            err_msg(insert_note("PLAN", None, &pool).await),
            "A note named \"PLAN\" already exists"
        );
        assert_eq!(inner_get_note(a.id, &pool).await.unwrap().title, "Plan");
    }

    #[tokio::test]
    async fn update_body_bumps_updated_at_and_returns_excerpt() {
        let pool = crate::test_db().await;
        let n = inner_create_note(None, &pool).await.unwrap();
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
        let n = inner_create_note(None, &pool).await.unwrap();
        inner_update_note_body(n.id, "same", &pool).await.unwrap();
        set_updated_at(n.id, OLD, &pool).await;

        let s = inner_update_note_body(n.id, "same", &pool).await.unwrap();
        assert_eq!(s.updated_at, OLD);
        assert_eq!(s.excerpt, "same");
    }

    #[tokio::test]
    async fn pinning_does_not_bump_updated_at() {
        let pool = crate::test_db().await;
        let n = inner_create_note(None, &pool).await.unwrap();
        set_updated_at(n.id, OLD, &pool).await;

        let s = inner_set_note_pinned(n.id, true, &pool).await.unwrap();
        assert!(s.pinned);
        assert_eq!(s.updated_at, OLD);
        let s = inner_set_note_pinned(n.id, false, &pool).await.unwrap();
        assert!(!s.pinned);
        assert_eq!(s.updated_at, OLD);
    }

    #[tokio::test]
    async fn list_orders_pinned_then_position_then_id() {
        let pool = crate::test_db().await;
        let a = note_titled("a", &pool).await;
        let b = note_titled("b", &pool).await;
        let c = note_titled("c", &pool).await;
        let d = note_titled("d", &pool).await;
        // Update times don't matter; new notes go on top, pinning to the pinned group.
        set_updated_at(b.id, "2026-03-01T00:00:00.000", &pool).await;
        inner_set_note_pinned(a.id, true, &pool).await.unwrap();
        assert_eq!(titles(&pool).await, vec!["a", "d", "c", "b"]);
        let list = inner_list_notes(&pool).await.unwrap();
        assert!(list[0].pinned);

        // Equal positions fall back to id.
        sqlx::query("UPDATE notes SET Position = 5 WHERE id IN (?, ?)")
            .bind(c.id)
            .bind(d.id)
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(titles(&pool).await, vec!["a", "b", "c", "d"]);
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
        let n = inner_create_note(None, &pool).await.unwrap();
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

    // ── Search ────────────────────────────────────────────────────────────────

    async fn note_with(title: &str, body: &str, pool: &Pool<Sqlite>) -> i64 {
        let n = note_titled(title, pool).await;
        inner_update_note_body(n.id, body, pool).await.unwrap();
        n.id
    }

    async fn hit_ids(query: &str, pool: &Pool<Sqlite>) -> Vec<i64> {
        inner_search_notes(query, None, pool)
            .await
            .unwrap()
            .into_iter()
            .map(|h| h.note.id)
            .collect()
    }

    #[tokio::test]
    async fn search_follows_create_rename_update_and_delete() {
        let pool = crate::test_db().await;
        let n = inner_create_note(None, &pool).await.unwrap();
        assert_eq!(hit_ids("untitled", &pool).await, vec![n.id]);

        inner_rename_note(n.id, "Kickoff", &pool).await.unwrap();
        assert!(hit_ids("untitled", &pool).await.is_empty());
        assert_eq!(hit_ids("kickoff", &pool).await, vec![n.id]);

        inner_update_note_body(n.id, "agenda: budget", &pool)
            .await
            .unwrap();
        assert_eq!(hit_ids("budget", &pool).await, vec![n.id]);
        inner_update_note_body(n.id, "agenda: hiring", &pool)
            .await
            .unwrap();
        assert!(hit_ids("budget", &pool).await.is_empty());
        assert_eq!(hit_ids("hiring", &pool).await, vec![n.id]);

        inner_delete_note(n.id, &pool).await.unwrap();
        assert!(hit_ids("kickoff", &pool).await.is_empty());
        let stats = inner_search_index_stats(&pool).await.unwrap();
        assert_eq!(stats.notes, 0);
    }

    #[tokio::test]
    async fn search_matches_substrings_case_insensitively_but_keeps_diacritics() {
        let pool = crate::test_db().await;
        let a = note_with("Tidrapport", "Veckans TIMMAR", &pool).await;
        let b = note_with("Mötet", "det är bra", &pool).await;
        assert_eq!(hit_ids("rapport", &pool).await, vec![a]);
        assert_eq!(hit_ids("timmar", &pool).await, vec![a]);
        assert_eq!(hit_ids("möt", &pool).await, vec![b]);
        assert!(hit_ids("mot", &pool).await.is_empty(), "ö stays distinct");
        assert!(
            hit_ids("ar bra", &pool).await.is_empty(),
            "ä stays distinct"
        );
        assert_eq!(hit_ids("är bra", &pool).await, vec![b]);
    }

    #[tokio::test]
    async fn search_ignores_markdown_syntax_and_link_urls() {
        let pool = crate::test_db().await;
        let id = note_with(
            "Links",
            "**bold** see [docs](https://secret.example.com/path) and `code here`",
            &pool,
        )
        .await;
        assert!(hit_ids("secret.example", &pool).await.is_empty());
        assert!(hit_ids("**bold", &pool).await.is_empty());
        assert_eq!(hit_ids("bold see docs", &pool).await, vec![id]);
        assert_eq!(hit_ids("code here", &pool).await, vec![id]);
    }

    #[tokio::test]
    async fn search_requires_all_terms_and_treats_operators_as_text() {
        let pool = crate::test_db().await;
        let a = note_with("Alpha", "apples and pears", &pool).await;
        note_with("Beta", "apples only", &pool).await;
        assert_eq!(hit_ids("apples pears", &pool).await, vec![a]);
        for q in [
            "\"",
            "apples\"",
            "*",
            "-x",
            "OR",
            "pears OR",
            "(",
            "a:b",
            "NEAR(x y)",
            "%",
            "_",
        ] {
            inner_search_notes(q, None, &pool).await.unwrap();
        }
        assert!(
            hit_ids("apples OR beta", &pool).await.is_empty(),
            "OR is literal"
        );
        assert!(hit_ids("   ", &pool).await.is_empty());
        assert!(hit_ids("", &pool).await.is_empty());
    }

    #[tokio::test]
    async fn short_terms_use_like_without_highlights() {
        let pool = crate::test_db().await;
        let body_hit = note_with("Zeta", "xy in body", &pool).await;
        let title_hit = note_with("Axy", "nothing", &pool).await;
        note_with("Other", "none", &pool).await;
        note_with("Pct", "100_0", &pool).await;
        let hits = inner_search_notes("xy", None, &pool).await.unwrap();
        let ids: Vec<i64> = hits.iter().map(|h| h.note.id).collect();
        assert_eq!(ids, vec![title_hit, body_hit], "title matches first");
        assert!(hits
            .iter()
            .all(|h| h.snippet.is_empty() && !h.title_marked.contains(HIT_START)));
        // `_` is literal, not a wildcard
        assert!(hit_ids("y_", &pool).await.is_empty());
        // Unicode case folding for short terms too
        let a = note_with("Ärende", "x", &pool).await;
        assert_eq!(hit_ids("är", &pool).await, vec![a]);
        assert_eq!(hit_ids("ÄR", &pool).await, vec![a]);
        // mixed: long term via MATCH, short term filters
        assert_eq!(hit_ids("body xy", &pool).await, vec![body_hit]);
        assert!(hit_ids("body qq", &pool).await.is_empty());
    }

    #[tokio::test]
    async fn title_hits_rank_above_body_hits_and_are_highlighted() {
        let pool = crate::test_db().await;
        let body = note_with("Notes", "talked about the budget at length", &pool).await;
        let title = note_with("Budget", "numbers", &pool).await;
        let hits = inner_search_notes("budget", None, &pool).await.unwrap();
        assert_eq!(
            hits.iter().map(|h| h.note.id).collect::<Vec<_>>(),
            vec![title, body]
        );
        assert_eq!(hits[0].title_marked, "\u{2}Budget\u{3}");
        assert!(hits[0].snippet.is_empty(), "no body hit → no snippet");
        assert!(hits[1].snippet.contains("\u{2}budget\u{3}"));
        assert_eq!(hits[1].title_marked, "Notes");

        let limited = inner_search_notes("budget", Some(1), &pool).await.unwrap();
        assert_eq!(limited.len(), 1);
    }

    #[tokio::test]
    async fn marker_chars_in_content_cannot_fake_highlights() {
        let pool = crate::test_db().await;
        let id = note_with("Plain", "fake \u{2}mark\u{3} text", &pool).await;
        let hits = inner_search_notes("text", None, &pool).await.unwrap();
        assert_eq!(hits[0].note.id, id);
        assert_eq!(hits[0].snippet.matches(HIT_START).count(), 1);
        assert!(hits[0].snippet.contains("fake mark \u{2}text\u{3}"));
    }

    // ── Link index ────────────────────────────────────────────────────────────

    async fn refs_of(id: i64, pool: &Pool<Sqlite>) -> Vec<(String, String)> {
        sqlx::query_as("SELECT Kind, Target FROM note_refs WHERE NoteId = ? ORDER BY Kind, Target")
            .bind(id)
            .fetch_all(pool)
            .await
            .unwrap()
    }

    fn rows(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, t)| (k.to_string(), t.to_string()))
            .collect()
    }

    #[tokio::test]
    async fn saving_a_body_updates_its_refs_and_delete_cascades() {
        let pool = crate::test_db().await;
        let id = note_with(
            "Refs",
            "#Acme #acme/web [[Plan]] [[plan|again]] `#code` #acme",
            &pool,
        )
        .await;
        assert_eq!(
            refs_of(id, &pool).await,
            rows(&[("link", "plan"), ("tag", "acme"), ("tag", "acme/web")])
        );

        inner_update_note_body(id, "only #other", &pool)
            .await
            .unwrap();
        assert_eq!(refs_of(id, &pool).await, rows(&[("tag", "other")]));
        inner_rename_note(id, "Renamed", &pool).await.unwrap();
        assert_eq!(refs_of(id, &pool).await, rows(&[("tag", "other")]));

        inner_delete_note(id, &pool).await.unwrap();
        let (left,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM note_refs")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(left, 0, "refs cascade with the note");
    }

    #[tokio::test]
    async fn rebuild_restores_both_indexes() {
        let pool = crate::test_db().await;
        let id = note_with("Kept", "searchable #tag [[Other]]", &pool).await;
        sqlx::raw_sql("DELETE FROM notes_fts; DELETE FROM note_refs;")
            .execute(&pool)
            .await
            .unwrap();
        inner_rebuild_search_index(&pool).await.unwrap();
        assert_eq!(hit_ids("searchable", &pool).await, vec![id]);
        assert_eq!(
            refs_of(id, &pool).await,
            rows(&[("link", "other"), ("tag", "tag")])
        );
        let version: String = sqlx::query_scalar("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(NOTE_REFS_VERSION_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, NOTE_REFS_VERSION);
    }

    // ── Rename rewriting ──────────────────────────────────────────────────────

    async fn body_of(id: i64, pool: &Pool<Sqlite>) -> String {
        inner_get_note(id, pool).await.unwrap().body
    }

    #[tokio::test]
    async fn rename_rewrites_links_in_all_notes_but_not_in_code() {
        let pool = crate::test_db().await;
        let target = note_with("Old plan", "self [[old plan]]", &pool).await;
        let a = note_with(
            "A",
            "[[Old plan]] and [[old PLAN|the plan]] `[[Old plan]]`\n\n```\n[[Old plan]]\n```\n[[Old planning]]",
            &pool,
        )
        .await;
        let b = note_with("B", "[[Other]]", &pool).await;
        for id in [target, a, b] {
            set_updated_at(id, OLD, &pool).await;
        }

        let renamed = inner_rename_note(target, "New plan", &pool).await.unwrap();
        assert_eq!(renamed.note.title, "New plan");
        assert_eq!(renamed.rewritten_ids, vec![target, a]);
        assert_eq!(
            body_of(a, &pool).await,
            "[[New plan]] and [[New plan|the plan]] `[[Old plan]]`\n\n```\n[[Old plan]]\n```\n[[Old planning]]"
        );
        assert_eq!(body_of(target, &pool).await, "self [[New plan]]");
        assert_eq!(body_of(b, &pool).await, "[[Other]]");
        let a_note = inner_get_note(a, &pool).await.unwrap();
        assert_eq!(a_note.updated_at, OLD, "a rewrite is not a user edit");
        // refs and search follow the rewrite
        assert_eq!(
            refs_of(a, &pool).await,
            rows(&[("link", "new plan"), ("link", "old planning")])
        );
        let backlinks = inner_list_backlinks(target, &pool).await.unwrap();
        assert_eq!(backlinks.iter().map(|l| l.id).collect::<Vec<_>>(), vec![a]);
        assert_eq!(hit_ids("the plan", &pool).await, vec![a]);

        // A case-only rename still rewrites; an unchanged title rewrites nothing.
        let recased = inner_rename_note(target, "new Plan", &pool).await.unwrap();
        assert_eq!(recased.rewritten_ids, vec![target, a]);
        assert!(body_of(a, &pool)
            .await
            .starts_with("[[new Plan]] and [[new Plan|the plan]]"));
        let same = inner_rename_note(target, "new Plan", &pool).await.unwrap();
        assert!(same.rewritten_ids.is_empty());
    }

    #[tokio::test]
    async fn failed_rename_rewrites_nothing() {
        let pool = crate::test_db().await;
        let target = note_with("Old", "", &pool).await;
        let a = note_with("A", "[[Old]]", &pool).await;
        let b = note_titled("Taken", &pool).await;
        // Past the Rust-side check, the unique index rejects the title: nothing is written.
        assert!(store_title(target, "Old", "taken", &pool).await.is_err());
        assert_eq!(body_of(a, &pool).await, "[[Old]]");
        assert_eq!(inner_get_note(b.id, &pool).await.unwrap().title, "Taken");
    }

    // ── Link commands ─────────────────────────────────────────────────────────

    /// Customers 1 Acme, 2 "ACME!" (slug clash: 1 owns `acme`), 3 "123" (no slug);
    /// projects 1 Acme/Web App, 2 Acme/web-app (clash), 3 ACME!/Other (customer has no
    /// tag); issue 7 in project 1; todos 4 (in progress), 5 (done).
    async fn seed_entities(pool: &Pool<Sqlite>) {
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Acme'), (2, 'ACME!'), (3, '123');
             INSERT INTO projects (id, CustomerId, Name) VALUES
                 (1, 1, 'Web App'), (2, 1, 'web-app'), (3, 2, 'Other');
             INSERT INTO issues (id, ProjectId, Title) VALUES (7, 1, 'Login bug');
             INSERT INTO todos (id, Title, Status, CreatedAt) VALUES
                 (4, 'Call Åsa', 'in_progress', 'x'), (5, 'Finished', 'done', 'x');",
        )
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn tag_suggestions_cover_tags_in_use_and_entities() {
        let pool = crate::test_db().await;
        seed_entities(&pool).await;
        note_with(
            "Tagged",
            "#Meeting #acme #acme/web-app #issue/7 #todo/5 #issue/999",
            &pool,
        )
        .await;
        type Row = (String, String, String, Option<String>, Option<String>);
        let got: Vec<Row> = inner_list_tag_suggestions(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| (s.kind, s.tag, s.label, s.detail, s.status))
            .collect();
        let row = |k: &str, t: &str, l: &str, d: Option<&str>, st: Option<&str>| {
            (
                k.to_string(),
                t.to_string(),
                l.to_string(),
                d.map(String::from),
                st.map(String::from),
            )
        };
        assert_eq!(
            got,
            vec![
                // in use, minus entity tags and `issue/<id>`/`todo/<id>` shapes
                row("tag", "meeting", "meeting", None, None),
                row("customer", "acme", "Acme", None, None),
                row("project", "acme/web-app", "Web App", Some("Acme"), None),
                row("issue", "issue/7", "Login bug — Acme / Web App", None, None),
                row("todo", "todo/4", "Call Åsa", None, Some("in_progress")),
            ]
        );
    }

    #[tokio::test]
    async fn tag_suggestions_skip_reserved_customer_slugs() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Issue'), (2, 'todo'), (3, 'Acme');
             INSERT INTO projects (id, CustomerId, Name) VALUES (1, 1, 'Web'), (2, 2, 'API');",
        )
        .execute(&pool)
        .await
        .unwrap();
        let got: Vec<(String, String)> = inner_list_tag_suggestions(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|s| (s.kind, s.tag))
            .collect();
        assert_eq!(got, vec![("customer".to_string(), "acme".to_string())]);
    }

    #[tokio::test]
    async fn link_targets_list_notes_issues_and_all_todos() {
        let pool = crate::test_db().await;
        seed_entities(&pool).await;
        let n = note_titled("Plan", &pool).await;
        let t = inner_get_link_targets(&pool).await.unwrap();
        assert_eq!(
            t.notes,
            vec![NoteTarget {
                id: n.id,
                title: "Plan".into()
            }]
        );
        assert_eq!(
            t.issues,
            vec![IssueTarget {
                id: 7,
                title: "Login bug".into(),
                context: "Acme / Web App".into()
            }]
        );
        let todos: Vec<(i64, &str)> = t.todos.iter().map(|t| (t.id, t.status.as_str())).collect();
        assert_eq!(todos, vec![(4, "in_progress"), (5, "done")]);
    }

    #[tokio::test]
    async fn create_note_titled_validates_and_rejects_duplicates() {
        let pool = crate::test_db().await;
        note_titled("Åsa", &pool).await;
        let n = inner_create_note_titled("  New idea ", None, &pool)
            .await
            .unwrap();
        assert_eq!((n.title.as_str(), n.body.as_str()), ("New idea", ""));
        assert_eq!(hit_ids("idea", &pool).await, vec![n.id]);
        assert_eq!(
            err_msg(inner_create_note_titled("åsa", None, &pool).await),
            "A note named \"åsa\" already exists"
        );
        assert_eq!(
            err_msg(inner_create_note_titled("a|b", None, &pool).await),
            "Title can't contain [ ] | # or `"
        );
        assert_eq!(
            err_msg(inner_create_note_titled(" ", None, &pool).await),
            "Title is required"
        );
    }

    #[tokio::test]
    async fn backlinks_match_titles_case_insensitively_and_exclude_self() {
        let pool = crate::test_db().await;
        let target = note_with("Åsa plan", "self link [[åsa plan]]", &pool).await;
        let a = note_with(
            "A",
            "intro\n- see [[ÅSA PLAN|the plan]] today\nmore [[Åsa plan]]",
            &pool,
        )
        .await;
        let b = note_with("B", "`[[Åsa plan]]` in code only", &pool).await;
        let c = note_with("C", "[[Åsa plan]]", &pool).await;
        set_updated_at(a.max(c), "2030-01-01T00:00:00.000", &pool).await;
        let links = inner_list_backlinks(target, &pool).await.unwrap();
        let got: Vec<(i64, &str)> = links.iter().map(|l| (l.id, l.snippet.as_str())).collect();
        assert_eq!(
            got,
            vec![(c, "[[Åsa plan]]"), (a, "see [[ÅSA PLAN|the plan]] today")]
        );
        assert!(!got.iter().any(|(id, _)| *id == b));
        assert_eq!(
            err_msg(inner_list_backlinks(999, &pool).await),
            "Note not found"
        );
    }

    #[test]
    fn backlink_snippet_windows_long_lines_around_the_link() {
        let body = format!("{} [[Target]] {}", "a".repeat(150), "b".repeat(150));
        let s = backlink_snippet(&body, "target");
        assert!(s.starts_with('…') && s.ends_with('…'), "{s}");
        assert!(s.contains("[[Target]]"), "{s}");
        assert_eq!(backlink_snippet("no link", "target"), "");
    }

    #[tokio::test]
    async fn hash_terms_filter_by_tag_including_nested() {
        let pool = crate::test_db().await;
        let acme = note_with("Acme notes", "budget #Acme", &pool).await;
        let web = note_with("Web", "budget #acme/web", &pool).await;
        let co = note_with("Acmeco", "budget #acmeco", &pool).await;
        let code = note_with("Code", "budget `#acme`", &pool).await;
        let mut ids = hit_ids("#acme", &pool).await;
        ids.sort();
        assert_eq!(ids, vec![acme, web], "nested, not partial or in code");
        assert_eq!(hit_ids("#ACME/web", &pool).await, vec![web]);
        assert_eq!(hit_ids("#acmeco", &pool).await, vec![co]);
        // AND with text terms (long via FTS, short in Rust) and other tags
        assert_eq!(hit_ids("#acme/web budget", &pool).await, vec![web]);
        assert_eq!(hit_ids("notes #acme", &pool).await, vec![acme]);
        assert_eq!(hit_ids("#acme we", &pool).await, vec![web]);
        assert!(hit_ids("#acme #acmeco", &pool).await.is_empty());
        assert!(hit_ids("#missing", &pool).await.is_empty());
        // `#` alone or digits only is text, not a tag
        assert!(hit_ids("#123", &pool).await.is_empty());
        let mut all_budget = hit_ids("budget", &pool).await;
        all_budget.sort();
        assert_eq!(all_budget, vec![acme, web, co, code]);
        let limited = inner_search_notes("#acme", Some(1), &pool).await.unwrap();
        assert_eq!(limited.len(), 1);
        assert!(limited[0].snippet.is_empty());
    }

    #[tokio::test]
    async fn rebuild_restores_a_drifted_index_and_reports_stats() {
        let pool = crate::test_db().await;
        let id = note_with("Kept", "searchable words", &pool).await;
        sqlx::query("DELETE FROM notes_fts")
            .execute(&pool)
            .await
            .unwrap();
        assert!(hit_ids("searchable", &pool).await.is_empty());
        let stats = inner_rebuild_search_index(&pool).await.unwrap();
        assert_eq!(stats.notes, 1);
        assert!(stats.bytes > 0);
        assert_eq!(hit_ids("searchable", &pool).await, vec![id]);
    }

    // ── Hybrid search ─────────────────────────────────────────────────────────

    /// Test embedder with known geometry: one axis per concept (Swedish and English words
    /// alike); words outside every concept add nothing, so such texts embed to zero.
    struct ConceptEmbedder;

    const CONCEPTS: [&[&str]; 2] = [
        &["budget", "money", "pengar", "kostnad", "costs"],
        &["garden", "tomato", "trädgård", "tomater"],
    ];

    impl crate::semantic::embedder::Embedder for ConceptEmbedder {
        fn model_id(&self) -> &str {
            "concepts"
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
            Ok(texts
                .iter()
                .map(|t| {
                    let mut v = vec![0f32; CONCEPTS.len()];
                    for w in t.split(|c: char| !c.is_alphanumeric()) {
                        let w = w.to_lowercase();
                        for (axis, words) in CONCEPTS.iter().enumerate() {
                            if words.contains(&w.as_str()) {
                                v[axis] += 1.0;
                            }
                        }
                    }
                    crate::semantic::embedder::l2_normalize(&mut v);
                    v
                })
                .collect())
        }
        fn count_tokens(&self, t: &str) -> usize {
            t.split_whitespace().count()
        }
    }

    async fn semantic_ready(pool: &Pool<Sqlite>) -> SemanticState {
        let state = SemanticState::new("/nonexistent".into());
        state.set_enabled(true);
        state.set_embedder(Some(std::sync::Arc::new(ConceptEmbedder)));
        state.enqueue_all(pool).await.unwrap();
        state.drain(pool).await;
        state
    }

    async fn hybrid(q: &str, state: &SemanticState, pool: &Pool<Sqlite>) -> Vec<(i64, bool)> {
        inner_search_notes_hybrid(q, None, pool, state)
            .await
            .unwrap()
            .into_iter()
            .map(|h| (h.note.id, h.related))
            .collect()
    }

    #[tokio::test]
    async fn hybrid_adds_semantic_only_hits_with_marker_and_chunk_snippet() {
        let pool = crate::test_db().await;
        let budget = note_with("Budget", "More money for travel next year.", &pool).await;
        note_with("Garden", "Plant tomato seedlings.", &pool).await;
        let state = semantic_ready(&pool).await;

        // "pengar" (Swedish) matches no word but the money concept.
        let hits = inner_search_notes_hybrid("pengar", None, &pool, &state)
            .await
            .unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].note.id, budget);
        assert!(hits[0].related);
        assert_eq!(hits[0].snippet, "More money for travel next year.");
        assert_eq!(hits[0].title_marked, "Budget", "no highlight markers");
    }

    #[tokio::test]
    async fn hybrid_fuses_with_rrf_keyword_and_semantic() {
        let pool = crate::test_db().await;
        // Keyword-only: "costs" as a word in a garden note (semantically garden-heavy).
        let mixed = note_with("Garden costs", "tomato tomato garden trädgård costs", &pool).await;
        // Both legs: keyword "costs" and the money concept.
        let both = note_with("Costs", "costs and money", &pool).await;
        // Semantic only.
        let sem = note_with("Budget", "money pengar", &pool).await;
        let state = semantic_ready(&pool).await;

        let keyword: Vec<i64> = hit_ids("costs", &pool).await;
        assert_eq!(keyword.len(), 2);
        let fused = hybrid("costs", &state, &pool).await;
        // `both` is in both lists → first; then the better single ranks.
        assert_eq!(fused[0], (both, false));
        assert_eq!(fused.len(), 3);
        assert!(fused.contains(&(sem, true)));
        assert!(fused.contains(&(mixed, false)));
    }

    #[tokio::test]
    async fn hybrid_excludes_hits_below_the_fixed_floor_in_small_indexes() {
        let pool = crate::test_db().await;
        let pure = note_with("Money", "money", &pool).await;
        // Half money, half garden: cosine ≈ 0.71 with a money query, under the floor.
        note_with("Mixed", "pengar tomater", &pool).await;
        let state = semantic_ready(&pool).await;
        assert!(crate::semantic::fusion::FIXED_FLOOR > 0.71);
        assert_eq!(hybrid("kostnad", &state, &pool).await, vec![(pure, true)]);
    }

    /// A note with `money` money words and `garden` garden words (neutral title): its
    /// cosine with a money query is money / sqrt(money² + garden²).
    async fn concept_note(money: usize, garden: usize, pool: &Pool<Sqlite>) -> i64 {
        concept_note_with(money, garden, "", pool).await
    }

    async fn concept_note_with(
        money: usize,
        garden: usize,
        extra: &str,
        pool: &Pool<Sqlite>,
    ) -> i64 {
        let body = format!(
            "{}{}{extra}",
            "money ".repeat(money),
            "tomato ".repeat(garden)
        );
        let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
            .fetch_one(pool)
            .await
            .unwrap();
        note_with(&format!("Note {count}"), body.trim(), pool).await
    }

    fn sorted(mut hits: Vec<(i64, bool)>) -> Vec<(i64, bool)> {
        hits.sort();
        hits
    }

    #[tokio::test]
    async fn hybrid_floor_is_relative_to_the_median_with_an_absolute_minimum() {
        use crate::semantic::fusion::{ABSOLUTE_FLOOR, FIXED_FLOOR, MARGIN};
        let pool = crate::test_db().await;
        for _ in 0..8 {
            concept_note(0, 1, &pool).await; // 0.0
        }
        let a = concept_note(3, 2, &pool).await; // 0.832
        let b = concept_note(4, 3, &pool).await; // 0.800
        let c = concept_note(5, 4, &pool).await; // 0.781
        concept_note(7, 6, &pool).await; // 0.759
        let state = semantic_ready(&pool).await;
        // Median 0 → the absolute floor decides; it keeps notes the fixed floor would drop.
        assert!(MARGIN < ABSOLUTE_FLOOR && ABSOLUTE_FLOOR > 0.7593 && ABSOLUTE_FLOOR <= 0.78);
        assert!(FIXED_FLOOR > 0.80);
        assert_eq!(
            sorted(hybrid("kostnad", &state, &pool).await),
            vec![(a, true), (b, true), (c, true)]
        );
    }

    #[tokio::test]
    async fn hybrid_floor_rises_with_the_median() {
        use crate::semantic::fusion::{FIXED_FLOOR, MARGIN};
        let pool = crate::test_db().await;
        for _ in 0..8 {
            concept_note(2, 1, &pool).await; // 0.894, above the fixed floor
        }
        let x = concept_note(5, 1, &pool).await; // 0.981
        let y = concept_note(1, 0, &pool).await; // 1.0
        let state = semantic_ready(&pool).await;
        // Median 0.894: only notes clearly above the typical note pass.
        assert!(0.894 > FIXED_FLOOR && 0.8945 + MARGIN < 0.98);
        assert_eq!(
            sorted(hybrid("kostnad", &state, &pool).await),
            vec![(x, true), (y, true)]
        );
    }

    #[tokio::test]
    async fn hybrid_uses_the_fixed_floor_below_eight_indexed_notes() {
        use crate::semantic::fusion::{ABSOLUTE_FLOOR, FIXED_FLOOR, MIN_NOTES_FOR_RELATIVE};
        assert_eq!(MIN_NOTES_FOR_RELATIVE, 8);
        assert!(ABSOLUTE_FLOOR <= 0.80 && FIXED_FLOOR > 0.80);
        let pool = crate::test_db().await;
        for _ in 0..5 {
            concept_note(0, 1, &pool).await; // 0.0
        }
        let a = concept_note(2, 1, &pool).await; // 0.894
        let b = concept_note(4, 3, &pool).await; // 0.800
        let state = semantic_ready(&pool).await;
        assert_eq!(hybrid("kostnad", &state, &pool).await, vec![(a, true)]);

        // An eighth note switches to the relative rule (median 0 → absolute floor).
        concept_note(0, 1, &pool).await;
        let state = semantic_ready(&pool).await;
        assert_eq!(
            sorted(hybrid("kostnad", &state, &pool).await),
            vec![(a, true), (b, true)]
        );
    }

    #[tokio::test]
    async fn hybrid_takes_the_median_before_the_tag_filter() {
        use crate::semantic::fusion::{FIXED_FLOOR, MARGIN};
        let pool = crate::test_db().await;
        for _ in 0..7 {
            concept_note(2, 1, &pool).await; // 0.894, untagged
        }
        concept_note_with(5, 2, " #work", &pool).await; // 0.928, under median + margin
        let top = concept_note_with(1, 0, " #work", &pool).await; // 1.0
        let state = semantic_ready(&pool).await;
        // Over the two tagged notes alone the fixed floor would keep both; over all nine
        // the median is 0.894, so only the top note passes.
        assert!(0.928 > FIXED_FLOOR && 0.8944 + MARGIN > 0.929);
        assert_eq!(
            hybrid("#work kostnad", &state, &pool).await,
            vec![(top, true)]
        );
    }

    #[tokio::test]
    async fn hybrid_skips_semantic_leg_for_short_and_tag_only_queries() {
        let pool = crate::test_db().await;
        note_with("Budget", "money #work", &pool).await;
        note_with("Plan", "pengar", &pool).await;
        let state = semantic_ready(&pool).await;
        for q in ["mon", "#work", "#work mon", "pen"] {
            let keyword = inner_search_notes(q, None, &pool).await.unwrap();
            let hybrid = inner_search_notes_hybrid(q, None, &pool, &state)
                .await
                .unwrap();
            assert_eq!(hybrid, keyword, "query {q:?}");
            assert!(hybrid.iter().all(|h| !h.related));
        }
    }

    #[tokio::test]
    async fn hybrid_tag_terms_filter_semantic_hits() {
        let pool = crate::test_db().await;
        let tagged = note_with("Budget", "money #work", &pool).await;
        let nested = note_with("Costs", "kostnad #work/acme", &pool).await;
        note_with("Private", "pengar", &pool).await;
        let state = semantic_ready(&pool).await;
        let mut hits = hybrid("#work pengar", &state, &pool).await;
        hits.sort();
        assert_eq!(hits, vec![(tagged, true), (nested, true)]);
    }

    #[tokio::test]
    async fn hybrid_equals_keyword_search_when_not_ready() {
        let pool = crate::test_db().await;
        note_with("Budget", "money plan for 2027", &pool).await;
        note_with("Garden", "tomato", &pool).await;
        let off = SemanticState::new("/nonexistent".into());
        let enabled_not_loaded = SemanticState::new("/nonexistent".into());
        enabled_not_loaded.set_enabled(true);
        for state in [&off, &enabled_not_loaded] {
            for q in ["budget", "pengar", "plan 2027", "#x", ""] {
                assert_eq!(
                    inner_search_notes_hybrid(q, Some(5), &pool, state)
                        .await
                        .unwrap(),
                    inner_search_notes(q, Some(5), &pool).await.unwrap(),
                    "query {q:?}"
                );
            }
        }
    }

    #[tokio::test]
    async fn hybrid_keeps_keyword_hits_past_the_fusion_depth_and_applies_limit() {
        let pool = crate::test_db().await;
        for i in 0..55 {
            note_with(&format!("Alpha {i}"), "alpha words", &pool).await;
        }
        let state = semantic_ready(&pool).await;
        let keyword = hit_ids("alpha", &pool).await;
        let fused: Vec<i64> = hybrid("alpha", &state, &pool)
            .await
            .into_iter()
            .map(|(id, _)| id)
            .collect();
        assert_eq!(
            fused, keyword,
            "no semantic hits: keyword order, nothing dropped"
        );
        let limited = inner_search_notes_hybrid("alpha", Some(3), &pool, &state)
            .await
            .unwrap();
        assert_eq!(limited.len(), 3);
    }

    #[tokio::test]
    async fn semantic_snippet_trims_long_chunks_at_a_word() {
        let long = vec!["ord"; 100].join(" ");
        let s = semantic_snippet(&long);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= SEMANTIC_SNIPPET_CHARS + 1);
        assert!(!s.contains("or…"), "cut between words");
        assert_eq!(semantic_snippet("  short\n text "), "short text");
    }

    #[tokio::test]
    async fn hybrid_drops_cached_vectors_of_deleted_notes_before_the_depth_cut() {
        let pool = crate::test_db().await;
        let mut ids = Vec::new();
        // Semantic-only for "pengar"; equal scores, so the lowest ids rank first.
        for i in 0..(crate::semantic::fusion::DEPTH + 15) {
            ids.push(note_with(&format!("Note {i}"), "money", &pool).await);
        }
        // Unrelated notes keep the median low, so the relative floor is the absolute one.
        for i in 0..(crate::semantic::fusion::DEPTH + 16) {
            note_with(&format!("Garden {i}"), "tomato", &pool).await;
        }
        let state = semantic_ready(&pool).await;
        // A delete that raced the worker leaves vectors cached without rows or note.
        let ghosts = &ids[..10];
        for id in ghosts {
            inner_delete_note(*id, &pool).await.unwrap();
        }
        let garden = crate::semantic::fusion::DEPTH + 16;
        assert_eq!(
            state.indexed_notes(),
            ids.len() + garden,
            "stale entries cached"
        );

        let hits = hybrid("pengar", &state, &pool).await;
        assert_eq!(
            hits.len(),
            crate::semantic::fusion::DEPTH,
            "depth filled by live notes"
        );
        assert!(hits
            .iter()
            .all(|(id, related)| *related && !ghosts.contains(id)));
        assert_eq!(
            state.indexed_notes(),
            ids.len() + garden - ghosts.len(),
            "ghosts evicted"
        );
    }

    // ── Semantic index hooks ──────────────────────────────────────────────────

    #[tokio::test]
    async fn note_writes_queue_for_the_semantic_index() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;

        let a = create_note_and_queue(None, &pool, &state).await.unwrap();
        let b = create_note_titled_and_queue("Old plan", None, &pool, &state)
            .await
            .unwrap();
        assert_eq!(state.queued(), vec![a.id, b.id], "create");
        state.drain(&pool).await;

        update_note_body_and_queue(a.id, "See [[Old plan]].", &pool, &state)
            .await
            .unwrap();
        assert_eq!(state.queued(), vec![a.id], "body update");
        state.drain(&pool).await;

        // Rename queues the note and the notes whose links were rewritten.
        rename_note_and_queue(b.id, "New plan", &pool, &state)
            .await
            .unwrap();
        assert_eq!(state.queued(), vec![b.id, a.id], "rename + link rewrite");

        // A failed write queues nothing.
        state.drain(&pool).await;
        assert!(update_note_body_and_queue(999, "x", &pool, &state)
            .await
            .is_err());
        assert!(state.queued().is_empty());
    }

    #[tokio::test]
    async fn note_delete_evicts_cached_vectors_and_queue_entry() {
        let pool = crate::test_db().await;
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        let a = create_note_and_queue(None, &pool, &state).await.unwrap();
        let b = create_note_and_queue(None, &pool, &state).await.unwrap();
        state.drain(&pool).await;
        assert_eq!(state.indexed_notes(), 2);

        delete_note_and_evict(a.id, &pool, &state).await.unwrap();
        assert_eq!(state.indexed_notes(), 1);
        // Queued then deleted: the queue entry goes too.
        state.enqueue([b.id]);
        delete_note_and_evict(b.id, &pool, &state).await.unwrap();
        assert_eq!(state.indexed_notes(), 0);
        assert!(state.queued().is_empty());
        let rows: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_embeddings")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(rows, 0, "cascade");
    }

    #[tokio::test]
    async fn tag_rename_catch_up_queues_all_notes_and_only_logs_failures() {
        let pool = crate::test_db().await;
        let a = note_with("A", "#acme", &pool).await;
        let b = note_with("B", "plain", &pool).await;
        let off = SemanticState::new("/nonexistent".into());
        queue_all_after_tag_rename(&pool, &off).await;
        assert!(off.queued().is_empty(), "disabled: nothing queued");

        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        queue_all_after_tag_rename(&pool, &state).await;
        let mut queued = state.queued();
        queued.sort();
        assert_eq!(queued, vec![a, b]);

        // The query fails (closed pool): logged, no panic, nothing queued.
        let (state, _) = crate::semantic::index::tests::ready_state(&pool).await;
        pool.close().await;
        queue_all_after_tag_rename(&pool, &state).await;
        assert!(state.queued().is_empty());
    }
}
