//! Apple Reminders sync commands: status, the switch, access, sync now, recreate the list,
//! the confirmed "Delete in Badger anyway" and dismissing the old-lists note. The sync itself
//! lives in `crate::apple_sync`.

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, State};

use crate::app_settings::{
    dismiss_apple_sync_old_lists_note as store_dismiss_note, get_apple_sync_settings,
    set_apple_sync_enabled as store_enabled, set_apple_sync_list_id,
};
use crate::apple_sync::badger_writes::{delete_all_pairings, reset_missed};
use crate::apple_sync::engine::PassMode;
use crate::apple_sync::runner::{emit, pass_now, SyncHandle};
use crate::apple_sync::{
    list_name, old_list_names, AccessState, ReminderStore, SkippedItem, STATUS_EVENT,
};
use crate::AppError;

/// System Settings → Privacy & Security → Reminders.
const PRIVACY_PANE: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_Reminders";

/// What Settings shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AppleSyncStatus {
    /// False off macOS.
    pub available: bool,
    pub enabled: bool,
    pub access: AccessState,
    pub last_sync_at: Option<String>,
    pub last_error: Option<String>,
    /// `access`, `list_missing` or `empty_fetch` while the mass-delete guard holds sync.
    pub paused: Option<String>,
    pub skipped: Vec<SkippedItem>,
    /// The synced list's name ("Badger").
    pub list: String,
    /// Show the one-time note that the old lists (`old_lists`) can be deleted on the phone.
    pub old_lists_note: bool,
    pub old_lists: Vec<String>,
}

// ── Inner functions (testable with the fake store) ────────────────────────────

pub async fn inner_get_status<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
) -> Result<AppleSyncStatus, AppError> {
    let s = get_apple_sync_settings(pool).await?;
    let access = store.access().await;
    Ok(AppleSyncStatus {
        available: access != AccessState::Unsupported,
        enabled: s.enabled,
        access,
        last_sync_at: s.last_sync_at,
        last_error: s.last_error,
        paused: s.paused,
        skipped: s
            .skipped
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
        list: list_name().to_string(),
        old_lists_note: s.old_lists_note,
        old_lists: old_list_names().iter().map(|n| n.to_string()).collect(),
    })
}

/// Turns sync on or off. Turning it on asks for access first if it was never asked; lists
/// and pairings are kept when it's turned off, so turning it on again resumes (with the
/// missed-pass counts started over).
pub async fn inner_set_enabled<S: ReminderStore>(
    enabled: bool,
    pool: &Pool<Sqlite>,
    store: &S,
) -> Result<AppleSyncStatus, AppError> {
    if enabled {
        match store.access().await {
            AccessState::Unsupported => {
                return Err(crate::apple_sync::StoreError::Unsupported.into())
            }
            AccessState::NotDetermined => {
                store.request_access().await?;
            }
            _ => {}
        }
    }
    if enabled {
        reset_missed(pool).await?;
    }
    store_enabled(pool, enabled).await?;
    inner_get_status(pool, store).await
}

/// Asks for access; returns the resulting state.
pub async fn inner_request_access<S: ReminderStore>(store: &S) -> Result<AccessState, AppError> {
    Ok(store.request_access().await?)
}

/// Creates a new "Badger" list when the stored id no longer resolves, and drops every
/// pairing, so the next pass pushes the tasks again. Returns whether it was recreated.
pub async fn inner_recreate_lists<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
) -> Result<bool, AppError> {
    let s = get_apple_sync_settings(pool).await?;
    let exists = match s.list_id.as_deref() {
        Some(id) => store.list_exists(id).await?,
        None => false,
    };
    if exists {
        return Ok(false);
    }
    let id = store.create_list(list_name()).await?;
    set_apple_sync_list_id(pool, Some(&id)).await?;
    delete_all_pairings(pool).await?;
    Ok(true)
}

/// Hides the old-lists note for good.
pub async fn inner_dismiss_old_lists_note<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
) -> Result<AppleSyncStatus, AppError> {
    store_dismiss_note(pool).await?;
    inner_get_status(pool, store).await
}

async fn require_enabled(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    if get_apple_sync_settings(pool).await?.enabled {
        Ok(())
    } else {
        Err(AppError::Custom(
            "Turn on Apple Reminders sync first".to_string(),
        ))
    }
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_apple_sync_status(
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    inner_get_status(pool.inner(), sync.store.as_ref()).await
}

#[tauri::command]
pub async fn set_apple_sync_enabled(
    enabled: bool,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    let status = inner_set_enabled(enabled, pool.inner(), sync.store.as_ref()).await?;
    if enabled {
        sync.nudge();
    }
    emit(&app, STATUS_EVENT);
    Ok(status)
}

/// Asks for access; when it's denied (macOS asks only once), opens the Reminders privacy
/// pane in System Settings instead.
#[tauri::command]
pub async fn request_apple_sync_access(
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    let access = inner_request_access(sync.store.as_ref()).await?;
    match access {
        AccessState::Denied | AccessState::Restricted => {
            if let Err(e) = tauri_plugin_opener::open_url(PRIVACY_PANE, None::<&str>) {
                tracing::error!("Apple Reminders sync: couldn't open System Settings: {}", e);
            }
        }
        AccessState::Granted => sync.nudge(),
        _ => {}
    }
    emit(&app, STATUS_EVENT);
    inner_get_status(pool.inner(), sync.store.as_ref()).await
}

#[tauri::command]
pub async fn sync_apple_reminders_now(
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    require_enabled(pool.inner()).await?;
    pass_now(&app, pool.inner(), sync.inner(), PassMode::Normal).await;
    inner_get_status(pool.inner(), sync.store.as_ref()).await
}

#[tauri::command]
pub async fn recreate_apple_sync_lists(
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    require_enabled(pool.inner()).await?;
    {
        let _pass = sync.lock.lock().await;
        inner_recreate_lists(pool.inner(), sync.store.as_ref()).await?;
    }
    pass_now(&app, pool.inner(), sync.inner(), PassMode::Normal).await;
    inner_get_status(pool.inner(), sync.store.as_ref()).await
}

#[tauri::command]
pub async fn dismiss_apple_sync_old_lists_note(
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    let status = inner_dismiss_old_lists_note(pool.inner(), sync.store.as_ref()).await?;
    emit(&app, STATUS_EVENT);
    Ok(status)
}

/// The confirmed override of the mass-delete guard: one pass that applies the deletions.
/// The engine honours it only while paused with `empty_fetch` (checked under the sync lock);
/// otherwise this is an ordinary pass.
#[tauri::command]
pub async fn apple_sync_delete_anyway(
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
    sync: State<'_, SyncHandle>,
) -> Result<AppleSyncStatus, AppError> {
    require_enabled(pool.inner()).await?;
    pass_now(&app, pool.inner(), sync.inner(), PassMode::DeleteAnyway).await;
    inner_get_status(pool.inner(), sync.store.as_ref()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple_sync::engine::{run_pass, Outcome};
    use crate::apple_sync::fake::FakeStore;
    use crate::apple_sync::{UnsupportedStore, PAUSED_LIST_MISSING};
    use crate::commands::tasks::{inner_create_task, inner_get_task, TaskInput};
    use crate::recurrence::parse_timestamp;

    fn now() -> chrono::NaiveDateTime {
        parse_timestamp("2026-11-19T12:00").unwrap()
    }

    async fn task(title: &str, pool: &Pool<Sqlite>) -> i64 {
        let input = TaskInput {
            title: title.to_string(),
            ..Default::default()
        };
        inner_create_task(&input, &[], now(), pool)
            .await
            .unwrap()
            .id
    }

    #[tokio::test]
    async fn status_is_off_by_default_and_unavailable_off_macos() {
        let pool = crate::test_db().await;
        let status = inner_get_status(&pool, &UnsupportedStore).await.unwrap();
        assert!(!status.available);
        assert!(!status.enabled);
        assert_eq!(status.access, AccessState::Unsupported);
        assert_eq!(status.list, list_name());
        assert!(!status.old_lists_note);
        let err = inner_set_enabled(true, &pool, &UnsupportedStore)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Apple Reminders sync is only available on macOS"
        );
        assert!(!get_apple_sync_settings(&pool).await.unwrap().enabled);
    }

    #[tokio::test]
    async fn enabling_asks_for_access_once_and_status_reports_the_pass() {
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        store.set_access(AccessState::NotDetermined);
        let status = inner_set_enabled(true, &pool, &store).await.unwrap();
        assert!(status.enabled && status.available);
        assert_eq!(status.access, AccessState::Granted);

        task("T", &pool).await;
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let list = store.list_id(list_name()).unwrap();
        store.add(&list, crate::apple_sync::fake::phone("   "));
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let status = inner_get_status(&pool, &store).await.unwrap();
        assert!(status.last_sync_at.is_some());
        assert_eq!(status.skipped.len(), 1);
        assert_eq!(status.skipped[0].reason, "needs a title");
        assert_eq!(status.skipped[0].list, list_name());

        // Off keeps the list and pairings.
        let status = inner_set_enabled(false, &pool, &store).await.unwrap();
        assert!(!status.enabled);
        assert_eq!(store.items(&list).len(), 2);
    }

    #[tokio::test]
    async fn recreate_list_replaces_a_missing_list_and_pushes_again() {
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        inner_set_enabled(true, &pool, &store).await.unwrap();
        let t = task("T", &pool).await;
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let old = store.list_id(list_name()).unwrap();
        store.remove_list(&old);
        let report = run_pass(&pool, &store, now(), PassMode::Normal).await;
        assert!(matches!(
            report.outcome,
            Outcome::Paused {
                kind: PAUSED_LIST_MISSING,
                ..
            }
        ));

        assert!(inner_recreate_lists(&pool, &store).await.unwrap());
        let new = store.list_id(list_name()).unwrap();
        assert_ne!(new, old);
        let report = run_pass(&pool, &store, now(), PassMode::Normal).await;
        assert_eq!(report.outcome, Outcome::Synced);
        assert_eq!(store.items(&new).len(), 1);
        assert_eq!(store.items(&new)[0].title, "T");
        assert!(inner_get_task(t, &pool).await.is_ok());
        assert!(!inner_recreate_lists(&pool, &store).await.unwrap());
    }

    #[tokio::test]
    async fn enabling_starts_the_missed_pass_counts_over() {
        use crate::apple_sync::badger_writes::{load_pairings, set_pairing_missed};
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        inner_set_enabled(true, &pool, &store).await.unwrap();
        task("T", &pool).await;
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let pairing = load_pairings(&pool).await.unwrap()[0].id;
        set_pairing_missed(pairing, 1, &pool).await.unwrap();
        inner_set_enabled(false, &pool, &store).await.unwrap();
        assert_eq!(load_pairings(&pool).await.unwrap()[0].missed, 1);
        inner_set_enabled(true, &pool, &store).await.unwrap();
        assert_eq!(load_pairings(&pool).await.unwrap()[0].missed, 0);
    }

    #[tokio::test]
    async fn the_old_lists_note_shows_until_dismissed_and_old_lists_are_never_touched() {
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        let old_todos = store.create_list(old_list_names()[0]).await.unwrap();
        store.add(&old_todos, crate::apple_sync::fake::phone("Old todo"));
        sqlx::query(
            "INSERT INTO app_settings (Key, Value) VALUES ('apple_sync_old_lists_note', 'true')",
        )
        .execute(&pool)
        .await
        .unwrap();
        let status = inner_set_enabled(true, &pool, &store).await.unwrap();
        assert!(status.old_lists_note);
        assert_eq!(status.old_lists.len(), 2);
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        assert_eq!(store.items(&old_todos).len(), 1, "untouched");
        assert!(
            store.list_id(list_name()).is_some(),
            "the new list is created"
        );
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tasks")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "nothing imported from the old lists");

        let status = inner_dismiss_old_lists_note(&pool, &store).await.unwrap();
        assert!(!status.old_lists_note);
        assert!(
            !inner_get_status(&pool, &store)
                .await
                .unwrap()
                .old_lists_note
        );
    }

    #[tokio::test]
    async fn sync_now_needs_sync_to_be_on() {
        let pool = crate::test_db().await;
        assert_eq!(
            require_enabled(&pool).await.unwrap_err().to_string(),
            "Turn on Apple Reminders sync first"
        );
    }
}
