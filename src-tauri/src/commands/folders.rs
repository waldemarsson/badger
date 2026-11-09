/// Note folders: a tree via `ParentId` (root = NULL). Names are unique among siblings,
/// case-insensitively; moves and deletes never fail on a clash but suffix the moved
/// folder (" 2", " 3", …) instead. Note titles stay globally unique, so notes never clash.
use crate::commands::notes::{inner_get_note, note_groups, write_note_positions, NoteSummary};
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection, SqliteExecutor};
use std::collections::{HashMap, HashSet};
use tauri::State;

/// Maximum folder name length in characters (not bytes).
pub const MAX_FOLDER_NAME_CHARS: usize = 100;
/// Base name of folders created without a name (`create_folder` with `name = None`).
pub const NEW_FOLDER_NAME: &str = "New folder";

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, PartialEq)]
pub struct Folder {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub name: String,
    /// Manual order among its siblings (display: `position`, then id).
    pub position: i64,
}

const SELECT_FOLDERS: &str =
    "SELECT id, ParentId AS parent_id, Name AS name, Position AS position FROM folders";

// ── Helpers ───────────────────────────────────────────────────────────────────

fn not_found() -> AppError {
    AppError::Custom("Folder not found".to_string())
}

fn duplicate(name: &str) -> AppError {
    AppError::Custom(format!("A folder named \"{name}\" already exists here"))
}

fn clean_name(name: &str) -> Result<String, AppError> {
    let n = name.trim();
    if n.is_empty() {
        return Err(AppError::Custom("Folder name is required".to_string()));
    }
    if n.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(AppError::Custom("Folder name is too long".to_string()));
    }
    if n.contains('/') {
        return Err(AppError::Custom("Folder name can't contain /".to_string()));
    }
    Ok(n.to_string())
}

/// `base` if free, else the first free `base 2`, `base 3`, … `taken` holds lowercased
/// names (Rust `to_lowercase`, so non-ASCII letters fold too).
pub(crate) fn first_free_name(base: &str, taken: &HashSet<String>) -> String {
    if !taken.contains(&base.to_lowercase()) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !taken.contains(&candidate.to_lowercase()))
        .expect("an unbounded range always yields a free name")
}

/// Lowercased names of the folders under `parent_id` (root = None), except `except_id`.
async fn sibling_names(
    parent_id: Option<i64>,
    except_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<HashSet<String>, AppError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT Name FROM folders WHERE ParentId IS ? AND id IS NOT ?")
            .bind(parent_id)
            .bind(except_id)
            .fetch_all(conn)
            .await?;
    Ok(rows.into_iter().map(|(n,)| n.to_lowercase()).collect())
}

/// `Folder not found` unless `folder_id` is None (root) or an existing folder.
pub(crate) async fn ensure_folder_exists(
    folder_id: Option<i64>,
    executor: impl SqliteExecutor<'_>,
) -> Result<(), AppError> {
    if let Some(id) = folder_id {
        let found: Option<(i64,)> = sqlx::query_as("SELECT id FROM folders WHERE id = ?")
            .bind(id)
            .fetch_optional(executor)
            .await?;
        if found.is_none() {
            return Err(not_found());
        }
    }
    Ok(())
}

async fn get_folder(id: i64, executor: impl SqliteExecutor<'_>) -> Result<Folder, AppError> {
    let sql = format!("{SELECT_FOLDERS} WHERE id = ?");
    sqlx::query_as::<_, Folder>(&sql)
        .bind(id)
        .fetch_optional(executor)
        .await?
        .ok_or_else(not_found)
}

/// Ids of the folders under `parent_id` (root = None) other than `except_id`, in order.
async fn sibling_ids(
    parent_id: Option<i64>,
    except_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<Vec<i64>, AppError> {
    Ok(sqlx::query_scalar(
        "SELECT id FROM folders WHERE ParentId IS ? AND id IS NOT ? ORDER BY Position, id",
    )
    .bind(parent_id)
    .bind(except_id)
    .fetch_all(conn)
    .await?)
}

/// Numbers the given folders 0..n in order (one parent's children).
async fn write_folder_positions(ids: &[i64], conn: &mut SqliteConnection) -> Result<(), AppError> {
    for (position, id) in (0_i64..).zip(ids) {
        sqlx::query("UPDATE folders SET Position = ? WHERE id = ?")
            .bind(position)
            .bind(id)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// `index` clamped to 0..=len (None: `default`).
fn clamp_index(index: Option<i64>, default: usize, len: usize) -> usize {
    index
        .map(|i| usize::try_from(i.max(0)).unwrap_or(usize::MAX))
        .unwrap_or(default)
        .min(len)
}

/// True when `target` is `id` or one of its descendants. Walks `ParentId` upward from
/// `target`, bounded by the folder count so corrupt (cyclic) data can't loop forever.
async fn is_self_or_descendant(
    id: i64,
    target: i64,
    conn: &mut SqliteConnection,
) -> Result<bool, AppError> {
    let rows: Vec<(i64, Option<i64>)> = sqlx::query_as("SELECT id, ParentId FROM folders")
        .fetch_all(conn)
        .await?;
    let parents: HashMap<i64, Option<i64>> = rows.into_iter().collect();
    let mut current = Some(target);
    for _ in 0..=parents.len() {
        match current {
            Some(c) if c == id => return Ok(true),
            Some(c) => current = parents.get(&c).copied().flatten(),
            None => return Ok(false),
        }
    }
    Ok(false)
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// All folders in manual order (`Position`, then id); grouped by parent, that is the tree.
pub async fn inner_list_folders(pool: &Pool<Sqlite>) -> Result<Vec<Folder>, AppError> {
    let sql = format!("{SELECT_FOLDERS} ORDER BY Position, id");
    Ok(sqlx::query_as::<_, Folder>(&sql).fetch_all(pool).await?)
}

/// Creates a folder at the end of `parent_id` (root = None). A given `name` is validated
/// and must be free among its siblings; `None` picks the first free `New folder[ n]`.
pub async fn inner_create_folder(
    name: Option<&str>,
    parent_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Folder, AppError> {
    let name = name.map(clean_name).transpose()?;
    let mut tx = pool.begin().await?;
    ensure_folder_exists(parent_id, &mut *tx).await?;
    let taken = sibling_names(parent_id, None, &mut tx).await?;
    let name = match name {
        Some(n) if taken.contains(&n.to_lowercase()) => return Err(duplicate(&n)),
        Some(n) => n,
        None => first_free_name(NEW_FOLDER_NAME, &taken),
    };
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO folders (ParentId, Name, Position) \
         VALUES (?, ?, (SELECT COALESCE(MAX(Position) + 1, 0) FROM folders WHERE ParentId IS ?)) \
         RETURNING id",
    )
    .bind(parent_id)
    .bind(&name)
    .bind(parent_id)
    .fetch_one(&mut *tx)
    .await?;
    let folder = get_folder(row.0, &mut *tx).await?;
    tx.commit().await?;
    Ok(folder)
}

/// Renames a folder. The name must be free among its siblings; a case change of its own
/// name is allowed.
pub async fn inner_rename_folder(
    id: i64,
    name: &str,
    pool: &Pool<Sqlite>,
) -> Result<Folder, AppError> {
    let name = clean_name(name)?;
    let mut tx = pool.begin().await?;
    let folder = get_folder(id, &mut *tx).await?;
    if sibling_names(folder.parent_id, Some(id), &mut tx)
        .await?
        .contains(&name.to_lowercase())
    {
        return Err(duplicate(&name));
    }
    sqlx::query("UPDATE folders SET Name = ? WHERE id = ?")
        .bind(&name)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let folder = get_folder(id, &mut *tx).await?;
    tx.commit().await?;
    Ok(folder)
}

/// Moves a folder under `parent_id` (root = None) at `index` among its new siblings
/// (clamped; None: the end), renumbering them densely. Rejects moving it into itself or a
/// descendant. On a name clash among the new siblings the folder gets the first free
/// ` 2`, ` 3`, … suffix. The current parent with `index` None is a no-op; with an index
/// it is a reorder.
pub async fn inner_move_folder(
    id: i64,
    parent_id: Option<i64>,
    index: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Folder, AppError> {
    let mut tx = pool.begin().await?;
    let folder = get_folder(id, &mut *tx).await?;
    let same_parent = folder.parent_id == parent_id;
    if same_parent && index.is_none() {
        return Ok(folder);
    }
    ensure_folder_exists(parent_id, &mut *tx).await?;
    if let Some(target) = parent_id {
        if is_self_or_descendant(id, target, &mut tx).await? {
            return Err(AppError::Custom(
                "Can't move a folder into itself".to_string(),
            ));
        }
    }
    let name = if same_parent {
        folder.name
    } else {
        first_free_name(
            &folder.name,
            &sibling_names(parent_id, Some(id), &mut tx).await?,
        )
    };
    sqlx::query("UPDATE folders SET ParentId = ?, Name = ? WHERE id = ?")
        .bind(parent_id)
        .bind(&name)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let mut order = sibling_ids(parent_id, Some(id), &mut tx).await?;
    let at = clamp_index(index, order.len(), order.len());
    order.insert(at, id);
    write_folder_positions(&order, &mut tx).await?;
    let folder = get_folder(id, &mut *tx).await?;
    tx.commit().await?;
    Ok(folder)
}

/// Deletes a folder. Its child folders (suffixed on a name clash, in order) and notes
/// move to the end of its parent's folders and notes, keeping their order, all in one
/// transaction. The deleted id is dropped from the stored expanded-folders set.
pub async fn inner_delete_folder(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let mut tx = pool.begin().await?;
    let folder = get_folder(id, &mut *tx).await?;
    let mut taken = sibling_names(folder.parent_id, Some(id), &mut tx).await?;
    let children: Vec<(i64, String)> =
        sqlx::query_as("SELECT id, Name FROM folders WHERE ParentId = ? ORDER BY Position, id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    let mut folder_order = sibling_ids(folder.parent_id, Some(id), &mut tx).await?;
    for (child_id, child_name) in children {
        let name = first_free_name(&child_name, &taken);
        taken.insert(name.to_lowercase());
        sqlx::query("UPDATE folders SET ParentId = ?, Name = ? WHERE id = ?")
            .bind(folder.parent_id)
            .bind(&name)
            .bind(child_id)
            .execute(&mut *tx)
            .await?;
        folder_order.push(child_id);
    }
    write_folder_positions(&folder_order, &mut tx).await?;
    // Parent's notes, then the deleted folder's, each in display order; the pinned ones
    // among them still show at the top.
    let (pinned, unpinned) = note_groups(folder.parent_id, None, &mut tx).await?;
    let (child_pinned, child_unpinned) = note_groups(Some(id), None, &mut tx).await?;
    sqlx::query("UPDATE notes SET FolderId = ? WHERE FolderId = ?")
        .bind(folder.parent_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let note_order: Vec<i64> = [pinned, unpinned, child_pinned, child_unpinned].concat();
    write_note_positions(&note_order, &mut tx).await?;
    sqlx::query("DELETE FROM folders WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    crate::app_settings::forget_expanded_folder(id, &mut tx).await?;
    tx.commit().await?;
    Ok(())
}

/// Moves a note into `folder_id` (root = None) at `index` within its own pin group there
/// (clamped; None: the top of the group), renumbering the folder's notes densely. The
/// same folder with an index is a reorder. `UpdatedAt` is untouched: moving is not an edit.
pub async fn inner_move_note(
    id: i64,
    folder_id: Option<i64>,
    index: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<NoteSummary, AppError> {
    let mut tx = pool.begin().await?;
    ensure_folder_exists(folder_id, &mut *tx).await?;
    let pinned: Option<bool> = sqlx::query_scalar("SELECT Pinned FROM notes WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(pinned) = pinned else {
        return Err(AppError::Custom("Note not found".to_string()));
    };
    sqlx::query("UPDATE notes SET FolderId = ? WHERE id = ?")
        .bind(folder_id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    let (mut pinned_ids, mut unpinned_ids) = note_groups(folder_id, Some(id), &mut tx).await?;
    let group = if pinned {
        &mut pinned_ids
    } else {
        &mut unpinned_ids
    };
    let at = clamp_index(index, 0, group.len());
    group.insert(at, id);
    pinned_ids.extend(unpinned_ids);
    write_note_positions(&pinned_ids, &mut tx).await?;
    tx.commit().await?;
    inner_get_note(id, pool).await.map(NoteSummary::from)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_folders(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<Folder>, AppError> {
    inner_list_folders(pool.inner()).await
}

#[tauri::command]
pub async fn create_folder(
    name: Option<String>,
    parent_id: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Folder, AppError> {
    inner_create_folder(name.as_deref(), parent_id, pool.inner()).await
}

#[tauri::command]
pub async fn rename_folder(
    id: i64,
    name: String,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Folder, AppError> {
    inner_rename_folder(id, &name, pool.inner()).await
}

#[tauri::command]
pub async fn move_folder(
    id: i64,
    parent_id: Option<i64>,
    index: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Folder, AppError> {
    inner_move_folder(id, parent_id, index, pool.inner()).await
}

#[tauri::command]
pub async fn delete_folder(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_folder(id, pool.inner()).await
}

#[tauri::command]
pub async fn move_note(
    id: i64,
    folder_id: Option<i64>,
    index: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<NoteSummary, AppError> {
    inner_move_note(id, folder_id, index, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::notes::{inner_create_note, inner_create_note_titled};

    const OLD: &str = "2000-01-01T00:00:00.000";

    fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
        r.unwrap_err().to_string()
    }

    async fn folder(name: &str, parent_id: Option<i64>, pool: &Pool<Sqlite>) -> Folder {
        inner_create_folder(Some(name), parent_id, pool)
            .await
            .unwrap()
    }

    async fn get(id: i64, pool: &Pool<Sqlite>) -> Folder {
        get_folder(id, pool).await.unwrap()
    }

    async fn note_in(folder_id: Option<i64>, pool: &Pool<Sqlite>) -> i64 {
        let id = inner_create_note(folder_id, pool).await.unwrap().id;
        sqlx::query("UPDATE notes SET UpdatedAt = ? WHERE id = ?")
            .bind(OLD)
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
        id
    }

    async fn note_folder(id: i64, pool: &Pool<Sqlite>) -> (Option<i64>, String) {
        let n = inner_get_note(id, pool).await.unwrap();
        (n.folder_id, n.updated_at)
    }

    #[test]
    fn first_free_name_suffixes_case_insensitively() {
        let taken: HashSet<String> = ["work", "work 2", "åsa"].map(String::from).into();
        assert_eq!(first_free_name("Home", &taken), "Home");
        assert_eq!(first_free_name("Work", &taken), "Work 3");
        assert_eq!(first_free_name("ÅSA", &taken), "ÅSA 2");
    }

    #[tokio::test]
    async fn create_validates_name() {
        let pool = crate::test_db().await;
        assert_eq!(
            err_msg(inner_create_folder(Some("  "), None, &pool).await),
            "Folder name is required"
        );
        assert_eq!(
            err_msg(inner_create_folder(Some(&"x".repeat(101)), None, &pool).await),
            "Folder name is too long"
        );
        assert_eq!(
            err_msg(inner_create_folder(Some("a/b"), None, &pool).await),
            "Folder name can't contain /"
        );
        let f = folder(&format!("  {}  ", "é".repeat(100)), None, &pool).await;
        assert_eq!(
            f.name,
            "é".repeat(100),
            "trimmed; 100 chars (not bytes) is fine"
        );
        assert_eq!(f.parent_id, None);
        assert_eq!(inner_list_folders(&pool).await.unwrap(), vec![f]);
    }

    #[tokio::test]
    async fn create_rejects_case_insensitive_sibling_duplicates_including_root() {
        let pool = crate::test_db().await;
        let work = folder("Work", None, &pool).await;
        folder("Åsa", None, &pool).await;
        assert_eq!(
            err_msg(inner_create_folder(Some("work"), None, &pool).await),
            "A folder named \"work\" already exists here"
        );
        assert_eq!(
            err_msg(inner_create_folder(Some(" åsa "), None, &pool).await),
            "A folder named \"åsa\" already exists here"
        );
        // Same name in another level is fine, and so is a clash-free sibling.
        let nested = folder("Work", Some(work.id), &pool).await;
        assert_eq!(nested.parent_id, Some(work.id));
        assert_eq!(
            err_msg(inner_create_folder(Some("WORK"), Some(work.id), &pool).await),
            "A folder named \"WORK\" already exists here"
        );
        folder("Home", None, &pool).await;
    }

    #[tokio::test]
    async fn create_without_name_picks_first_free_new_folder() {
        let pool = crate::test_db().await;
        let names = |pool: Pool<Sqlite>, parent: Option<i64>| async move {
            inner_create_folder(None, parent, &pool).await.unwrap().name
        };
        assert_eq!(names(pool.clone(), None).await, "New folder");
        assert_eq!(names(pool.clone(), None).await, "New folder 2");
        let p = folder("P", None, &pool).await;
        assert_eq!(names(pool.clone(), Some(p.id)).await, "New folder");
        folder("new folder 3", None, &pool).await;
        assert_eq!(names(pool.clone(), None).await, "New folder 4");
    }

    #[tokio::test]
    async fn folder_id_arguments_report_folder_not_found() {
        let pool = crate::test_db().await;
        let f = folder("F", None, &pool).await;
        let n = note_in(None, &pool).await;
        assert_eq!(
            err_msg(inner_create_folder(Some("X"), Some(99), &pool).await),
            "Folder not found"
        );
        assert_eq!(
            err_msg(inner_create_folder(None, Some(99), &pool).await),
            "Folder not found"
        );
        assert_eq!(
            err_msg(inner_create_note(Some(99), &pool).await),
            "Folder not found"
        );
        assert_eq!(
            err_msg(inner_create_note_titled("T", Some(99), &pool).await),
            "Folder not found"
        );
        assert_eq!(
            err_msg(inner_move_note(n, Some(99), None, &pool).await),
            "Folder not found"
        );
        assert_eq!(
            err_msg(inner_move_folder(f.id, Some(99), None, &pool).await),
            "Folder not found"
        );
        for missing in [
            err_msg(inner_rename_folder(99, "X", &pool).await),
            err_msg(inner_move_folder(99, None, None, &pool).await),
            err_msg(inner_delete_folder(99, &pool).await),
        ] {
            assert_eq!(missing, "Folder not found");
        }
        assert_eq!(
            err_msg(inner_move_note(999, Some(f.id), None, &pool).await),
            "Note not found"
        );
        let count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count.0, 1, "failed creates insert nothing");
    }

    #[tokio::test]
    async fn notes_are_created_in_the_given_folder() {
        let pool = crate::test_db().await;
        let f = folder("F", None, &pool).await;
        assert_eq!(
            inner_create_note(Some(f.id), &pool)
                .await
                .unwrap()
                .folder_id,
            Some(f.id)
        );
        assert_eq!(
            inner_create_note_titled("Linked", Some(f.id), &pool)
                .await
                .unwrap()
                .folder_id,
            Some(f.id)
        );
        assert_eq!(
            inner_create_note(None, &pool).await.unwrap().folder_id,
            None
        );
    }

    #[tokio::test]
    async fn rename_validates_and_checks_siblings_only() {
        let pool = crate::test_db().await;
        let a = folder("Alpha", None, &pool).await;
        folder("Beta", None, &pool).await;
        let inner = folder("Gamma", Some(a.id), &pool).await;

        assert_eq!(
            err_msg(inner_rename_folder(a.id, " ", &pool).await),
            "Folder name is required"
        );
        assert_eq!(
            err_msg(inner_rename_folder(a.id, &"x".repeat(101), &pool).await),
            "Folder name is too long"
        );
        assert_eq!(
            err_msg(inner_rename_folder(a.id, "x/y", &pool).await),
            "Folder name can't contain /"
        );
        assert_eq!(
            err_msg(inner_rename_folder(a.id, "beta", &pool).await),
            "A folder named \"beta\" already exists here"
        );
        assert_eq!(
            get(a.id, &pool).await.name,
            "Alpha",
            "failed rename keeps name"
        );

        // Own name with a case change, and a name used at another level, are fine.
        assert_eq!(
            inner_rename_folder(a.id, " ALPHA ", &pool)
                .await
                .unwrap()
                .name,
            "ALPHA"
        );
        assert_eq!(
            inner_rename_folder(inner.id, "Beta", &pool).await.unwrap(),
            Folder {
                id: inner.id,
                parent_id: Some(a.id),
                name: "Beta".into(),
                position: 0
            }
        );
    }

    #[tokio::test]
    async fn move_folder_rejects_self_and_descendants() {
        let pool = crate::test_db().await;
        let a = folder("A", None, &pool).await;
        let b = folder("B", Some(a.id), &pool).await;
        let c = folder("C", Some(b.id), &pool).await;
        for target in [a.id, b.id, c.id] {
            assert_eq!(
                err_msg(inner_move_folder(a.id, Some(target), None, &pool).await),
                "Can't move a folder into itself",
                "target {target}"
            );
        }
        assert_eq!(get(a.id, &pool).await.parent_id, None);
        // Upward and sideways moves are fine.
        assert_eq!(
            inner_move_folder(c.id, Some(a.id), None, &pool)
                .await
                .unwrap()
                .parent_id,
            Some(a.id)
        );
        assert_eq!(
            inner_move_folder(b.id, None, None, &pool)
                .await
                .unwrap()
                .parent_id,
            None
        );
        assert_eq!(
            inner_move_folder(a.id, Some(b.id), None, &pool)
                .await
                .unwrap()
                .parent_id,
            Some(b.id)
        );
    }

    #[tokio::test]
    async fn move_folder_suffixes_on_clash_and_same_parent_is_a_noop() {
        let pool = crate::test_db().await;
        let projects = folder("Projects", None, &pool).await;
        folder("Archive", None, &pool).await;
        folder("archive 2", None, &pool).await;
        let nested = folder("Archive", Some(projects.id), &pool).await;
        let solo = folder("Solo", Some(projects.id), &pool).await;

        let moved = inner_move_folder(nested.id, None, None, &pool)
            .await
            .unwrap();
        assert_eq!((moved.parent_id, moved.name.as_str()), (None, "Archive 3"));

        // Back down: no clash, the suffixed name is kept as is.
        let back = inner_move_folder(moved.id, Some(projects.id), None, &pool)
            .await
            .unwrap();
        assert_eq!(back.name, "Archive 3");

        // Same parent without an index: unchanged, no suffix.
        let before = get(solo.id, &pool).await;
        let same = inner_move_folder(solo.id, Some(projects.id), None, &pool)
            .await
            .unwrap();
        assert_eq!(same, before);
        // Same parent with an index: a reorder, still no suffix.
        let reordered = inner_move_folder(solo.id, Some(projects.id), Some(0), &pool)
            .await
            .unwrap();
        assert_eq!((reordered.name.as_str(), reordered.position), ("Solo", 0));
    }

    /// Folder names under `parent_id`, in list order.
    async fn folder_order(parent_id: Option<i64>, pool: &Pool<Sqlite>) -> Vec<String> {
        inner_list_folders(pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|f| f.parent_id == parent_id)
            .map(|f| f.name)
            .collect()
    }

    /// Raw (id, Position) of the folders under `parent_id`, in list order.
    async fn folder_positions(parent_id: Option<i64>, pool: &Pool<Sqlite>) -> Vec<i64> {
        inner_list_folders(pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|f| f.parent_id == parent_id)
            .map(|f| f.position)
            .collect()
    }

    /// Note ids in `folder_id`, in list order (pinned first, then position, then id).
    async fn note_order(folder_id: Option<i64>, pool: &Pool<Sqlite>) -> Vec<i64> {
        crate::commands::notes::inner_list_notes(pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.folder_id == folder_id)
            .map(|n| n.id)
            .collect()
    }

    async fn note_positions(folder_id: Option<i64>, pool: &Pool<Sqlite>) -> Vec<i64> {
        crate::commands::notes::inner_list_notes(pool)
            .await
            .unwrap()
            .into_iter()
            .filter(|n| n.folder_id == folder_id)
            .map(|n| n.position)
            .collect()
    }

    async fn pin(id: i64, pinned: bool, pool: &Pool<Sqlite>) {
        crate::commands::notes::inner_set_note_pinned(id, pinned, pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn folders_are_listed_in_manual_order_and_created_at_the_end() {
        let pool = crate::test_db().await;
        let zed = folder("Zed", None, &pool).await;
        folder("alpha", None, &pool).await;
        let p = folder("Mid", None, &pool).await;
        folder("b", Some(p.id), &pool).await;
        folder("a", Some(p.id), &pool).await;
        assert_eq!(folder_order(None, &pool).await, ["Zed", "alpha", "Mid"]);
        assert_eq!(folder_order(Some(p.id), &pool).await, ["b", "a"]);
        assert_eq!(folder_positions(None, &pool).await, [0, 1, 2]);
        // Renaming doesn't re-sort.
        inner_rename_folder(zed.id, "Aaa", &pool).await.unwrap();
        assert_eq!(folder_order(None, &pool).await, ["Aaa", "alpha", "Mid"]);
    }

    #[tokio::test]
    async fn move_folder_with_index_reorders_inserts_and_clamps() {
        let pool = crate::test_db().await;
        let a = folder("A", None, &pool).await;
        folder("B", None, &pool).await;
        let c = folder("C", None, &pool).await;
        let p = folder("P", None, &pool).await;
        folder("X", Some(p.id), &pool).await;
        folder("Y", Some(p.id), &pool).await;

        // Same parent: pure reorder, positions dense.
        inner_move_folder(c.id, None, Some(0), &pool).await.unwrap();
        assert_eq!(folder_order(None, &pool).await, ["C", "A", "B", "P"]);
        inner_move_folder(c.id, None, Some(2), &pool).await.unwrap();
        assert_eq!(folder_order(None, &pool).await, ["A", "B", "C", "P"]);
        assert_eq!(folder_positions(None, &pool).await, [0, 1, 2, 3]);
        // Clamped at both ends.
        inner_move_folder(a.id, None, Some(99), &pool)
            .await
            .unwrap();
        assert_eq!(folder_order(None, &pool).await, ["B", "C", "P", "A"]);
        inner_move_folder(a.id, None, Some(-5), &pool)
            .await
            .unwrap();
        assert_eq!(folder_order(None, &pool).await, ["A", "B", "C", "P"]);

        // Across parents at an index; default (None) is the end.
        inner_move_folder(a.id, Some(p.id), Some(1), &pool)
            .await
            .unwrap();
        assert_eq!(folder_order(Some(p.id), &pool).await, ["X", "A", "Y"]);
        assert_eq!(folder_positions(Some(p.id), &pool).await, [0, 1, 2]);
        inner_move_folder(c.id, Some(p.id), None, &pool)
            .await
            .unwrap();
        assert_eq!(folder_order(Some(p.id), &pool).await, ["X", "A", "Y", "C"]);
        assert_eq!(folder_order(None, &pool).await, ["B", "P"]);
    }

    #[tokio::test]
    async fn move_folder_with_index_keeps_cycle_and_name_rules() {
        let pool = crate::test_db().await;
        let a = folder("A", None, &pool).await;
        let b = folder("B", Some(a.id), &pool).await;
        folder("Dup", None, &pool).await;
        let dup = folder("dup", Some(a.id), &pool).await;
        assert_eq!(
            err_msg(inner_move_folder(a.id, Some(b.id), Some(0), &pool).await),
            "Can't move a folder into itself"
        );
        assert_eq!(
            err_msg(inner_move_folder(a.id, Some(a.id), Some(0), &pool).await),
            "Can't move a folder into itself"
        );
        let moved = inner_move_folder(dup.id, None, Some(0), &pool)
            .await
            .unwrap();
        assert_eq!((moved.name.as_str(), moved.position), ("dup 2", 0));
        assert_eq!(folder_order(None, &pool).await, ["dup 2", "A", "Dup"]);
    }

    #[tokio::test]
    async fn new_notes_go_to_the_top_of_the_unpinned_group() {
        let pool = crate::test_db().await;
        let f = folder("F", None, &pool).await;
        let first = note_in(Some(f.id), &pool).await;
        let pinned = note_in(Some(f.id), &pool).await;
        pin(pinned, true, &pool).await;
        let newest = note_in(Some(f.id), &pool).await;
        let root = note_in(None, &pool).await;
        assert_eq!(note_order(Some(f.id), &pool).await, [pinned, newest, first]);
        assert_eq!(note_order(None, &pool).await, [root]);
    }

    #[tokio::test]
    async fn move_note_with_index_reorders_within_its_pin_group_and_across_folders() {
        let pool = crate::test_db().await;
        let f = folder("F", None, &pool).await;
        let g = folder("G", None, &pool).await;
        // In F (top to bottom): p1 p2 | u1 u2 u3
        let u3 = note_in(Some(f.id), &pool).await;
        let u2 = note_in(Some(f.id), &pool).await;
        let u1 = note_in(Some(f.id), &pool).await;
        let p2 = note_in(Some(f.id), &pool).await;
        let p1 = note_in(Some(f.id), &pool).await;
        pin(p1, true, &pool).await;
        pin(p2, true, &pool).await;
        assert_eq!(note_order(Some(f.id), &pool).await, [p1, p2, u1, u2, u3]);

        // Same folder: pure reorder within the group, positions dense over the folder.
        inner_move_note(u3, Some(f.id), Some(0), &pool)
            .await
            .unwrap();
        assert_eq!(note_order(Some(f.id), &pool).await, [p1, p2, u3, u1, u2]);
        assert_eq!(note_positions(Some(f.id), &pool).await, [0, 1, 2, 3, 4]);
        // An index is relative to the pin group and clamps to it: a pinned note can't
        // land among unpinned ones, nor an unpinned one among pinned ones.
        inner_move_note(p1, Some(f.id), Some(99), &pool)
            .await
            .unwrap();
        assert_eq!(note_order(Some(f.id), &pool).await, [p2, p1, u3, u1, u2]);
        inner_move_note(u2, Some(f.id), Some(-3), &pool)
            .await
            .unwrap();
        assert_eq!(note_order(Some(f.id), &pool).await, [p2, p1, u2, u3, u1]);
        inner_move_note(u1, Some(f.id), Some(1), &pool)
            .await
            .unwrap();
        assert_eq!(note_order(Some(f.id), &pool).await, [p2, p1, u2, u1, u3]);

        // Across folders at an index; None is the top of the group (pinned too).
        let g1 = note_in(Some(g.id), &pool).await;
        let g0 = note_in(Some(g.id), &pool).await;
        inner_move_note(u2, Some(g.id), Some(1), &pool)
            .await
            .unwrap();
        assert_eq!(note_order(Some(g.id), &pool).await, [g0, u2, g1]);
        inner_move_note(u3, Some(g.id), None, &pool).await.unwrap();
        assert_eq!(note_order(Some(g.id), &pool).await, [u3, g0, u2, g1]);
        inner_move_note(p1, Some(g.id), None, &pool).await.unwrap();
        assert_eq!(note_order(Some(g.id), &pool).await, [p1, u3, g0, u2, g1]);
        assert_eq!(note_positions(Some(g.id), &pool).await, [0, 1, 2, 3, 4]);
        assert_eq!(note_order(Some(f.id), &pool).await, [p2, u1]);
        assert_eq!(note_folder(u2, &pool).await, (Some(g.id), OLD.to_string()));
    }

    #[tokio::test]
    async fn pinning_lands_on_the_group_boundary() {
        let pool = crate::test_db().await;
        let c = note_in(None, &pool).await;
        let b = note_in(None, &pool).await;
        let a = note_in(None, &pool).await;
        let p = note_in(None, &pool).await;
        pin(p, true, &pool).await;
        assert_eq!(note_order(None, &pool).await, [p, a, b, c]);
        // Pin → bottom of the pinned group.
        pin(c, true, &pool).await;
        assert_eq!(note_order(None, &pool).await, [p, c, a, b]);
        // Unpin → top of the unpinned group.
        pin(p, false, &pool).await;
        assert_eq!(note_order(None, &pool).await, [c, p, a, b]);
        assert_eq!(note_positions(None, &pool).await, [0, 1, 2, 3]);
        // Re-pinning an already pinned note keeps its place.
        pin(c, true, &pool).await;
        assert_eq!(note_order(None, &pool).await, [c, p, a, b]);
        assert_eq!(note_folder(c, &pool).await.1, OLD, "pinning is not an edit");
    }

    #[tokio::test]
    async fn editing_or_renaming_a_note_keeps_its_place() {
        let pool = crate::test_db().await;
        let b = note_in(None, &pool).await;
        let a = note_in(None, &pool).await;
        crate::commands::notes::inner_update_note_body(b, "changed", &pool)
            .await
            .unwrap();
        crate::commands::notes::inner_rename_note(b, "Renamed", &pool)
            .await
            .unwrap();
        assert_eq!(note_order(None, &pool).await, [a, b]);
    }

    #[tokio::test]
    async fn delete_folder_appends_children_to_the_parent_in_order() {
        let pool = crate::test_db().await;
        let top = folder("Top", None, &pool).await;
        folder("First", Some(top.id), &pool).await;
        let doomed = folder("Doomed", Some(top.id), &pool).await;
        folder("Last", Some(top.id), &pool).await;
        let z = folder("Z", Some(doomed.id), &pool).await;
        folder("Y", Some(doomed.id), &pool).await;
        inner_move_folder(z.id, Some(doomed.id), Some(1), &pool)
            .await
            .unwrap(); // Y, Z
        let kept = note_in(Some(top.id), &pool).await;
        let kept_pinned = note_in(Some(top.id), &pool).await;
        pin(kept_pinned, true, &pool).await;
        let child2 = note_in(Some(doomed.id), &pool).await;
        let child1 = note_in(Some(doomed.id), &pool).await;
        let child_pinned = note_in(Some(doomed.id), &pool).await;
        pin(child_pinned, true, &pool).await;

        inner_delete_folder(doomed.id, &pool).await.unwrap();

        assert_eq!(
            folder_order(Some(top.id), &pool).await,
            ["First", "Last", "Y", "Z"]
        );
        assert_eq!(folder_positions(Some(top.id), &pool).await, [0, 1, 2, 3]);
        assert_eq!(
            note_order(Some(top.id), &pool).await,
            [kept_pinned, child_pinned, kept, child1, child2]
        );
    }

    #[tokio::test]
    async fn move_note_changes_folder_without_bumping_updated_at() {
        let pool = crate::test_db().await;
        let f = folder("F", None, &pool).await;
        let n = note_in(None, &pool).await;

        let moved = inner_move_note(n, Some(f.id), None, &pool).await.unwrap();
        assert_eq!(moved.folder_id, Some(f.id));
        assert_eq!(note_folder(n, &pool).await, (Some(f.id), OLD.to_string()));

        inner_move_note(n, None, None, &pool).await.unwrap();
        assert_eq!(note_folder(n, &pool).await, (None, OLD.to_string()));
    }

    #[tokio::test]
    async fn delete_folder_moves_contents_to_parent_with_suffixes() {
        let pool = crate::test_db().await;
        let top = folder("Top", None, &pool).await;
        folder("Shared", Some(top.id), &pool).await;
        let doomed = folder("Doomed", Some(top.id), &pool).await;
        let shared = folder("shared", Some(doomed.id), &pool).await;
        // Legacy data may hold case-duplicate siblings; insert past the validation.
        let shared2: (i64,) = sqlx::query_as(
            "INSERT INTO folders (ParentId, Name) VALUES (?, 'Shared') RETURNING id",
        )
        .bind(doomed.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let shared2 = get(shared2.0, &pool).await;
        let unique = folder("Unique", Some(doomed.id), &pool).await;
        let grandchild = folder("Deep", Some(unique.id), &pool).await;
        let n = note_in(Some(doomed.id), &pool).await;
        let other = note_in(Some(top.id), &pool).await;

        inner_delete_folder(doomed.id, &pool).await.unwrap();

        assert_eq!(
            err_msg(get_folder(doomed.id, &pool).await),
            "Folder not found"
        );
        let lifted = |f: Folder| (f.parent_id, f.name);
        assert_eq!(
            lifted(get(shared.id, &pool).await),
            (Some(top.id), "shared 2".into())
        );
        assert_eq!(
            lifted(get(shared2.id, &pool).await),
            (Some(top.id), "Shared 3".into())
        );
        assert_eq!(
            lifted(get(unique.id, &pool).await),
            (Some(top.id), "Unique".into())
        );
        assert_eq!(get(grandchild.id, &pool).await.parent_id, Some(unique.id));
        assert_eq!(note_folder(n, &pool).await, (Some(top.id), OLD.to_string()));
        assert_eq!(note_folder(other, &pool).await.0, Some(top.id));
    }

    #[tokio::test]
    async fn delete_root_folder_moves_contents_to_root() {
        let pool = crate::test_db().await;
        folder("Child", None, &pool).await;
        let root = folder("Root", None, &pool).await;
        let child = folder("Child", Some(root.id), &pool).await;
        let n = note_in(Some(root.id), &pool).await;

        inner_delete_folder(root.id, &pool).await.unwrap();

        let c = get(child.id, &pool).await;
        assert_eq!((c.parent_id, c.name.as_str()), (None, "Child 2"));
        assert_eq!(note_folder(n, &pool).await.0, None);
        assert_eq!(inner_list_folders(&pool).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn delete_folder_drops_it_from_the_stored_expanded_set() {
        let pool = crate::test_db().await;
        let a = folder("A", None, &pool).await;
        let b = folder("B", None, &pool).await;
        crate::app_settings::set_expanded_folders(&pool, vec![a.id, b.id])
            .await
            .unwrap();

        inner_delete_folder(a.id, &pool).await.unwrap();

        let stored: (String,) = sqlx::query_as("SELECT Value FROM app_settings WHERE Key = ?")
            .bind(crate::app_settings::EXPANDED_FOLDERS_KEY)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored.0, format!("[{}]", b.id));
    }
}
