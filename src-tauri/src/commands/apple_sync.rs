//! Apple Reminders sync commands: status, the switch, access, sync now, recreate lists and
//! the confirmed "Delete in Badger anyway". The sync itself lives in `crate::apple_sync`.

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::{AppHandle, State};

use crate::app_settings::{
    get_apple_sync_settings, set_apple_sync_enabled as store_enabled, set_apple_sync_list_id,
};
use crate::apple_sync::badger_writes::{delete_pairings_of, reset_missed};
use crate::apple_sync::engine::PassMode;
use crate::apple_sync::runner::{emit, pass_now, SyncHandle};
use crate::apple_sync::{AccessState, ListKind, ReminderStore, SkippedItem, STATUS_EVENT};
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
    pub reminders_list: String,
    pub todos_list: String,
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
        reminders_list: ListKind::Reminders.list_name().to_string(),
        todos_list: ListKind::Todos.list_name().to_string(),
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

/// Creates a new list for each one whose stored id no longer resolves, and drops that
/// list's pairings, so the next pass pushes its Badger items again. Returns how many lists
/// were recreated.
pub async fn inner_recreate_lists<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
) -> Result<usize, AppError> {
    let s = get_apple_sync_settings(pool).await?;
    let mut recreated = 0;
    for kind in ListKind::ALL {
        let stored = match kind {
            ListKind::Reminders => s.reminders_list_id.as_deref(),
            ListKind::Todos => s.todos_list_id.as_deref(),
        };
        let exists = match stored {
            Some(id) => store.list_exists(id).await?,
            None => false,
        };
        if exists {
            continue;
        }
        let id = store.create_list(kind.list_name()).await?;
        set_apple_sync_list_id(pool, kind.settings_key(), Some(&id)).await?;
        delete_pairings_of(kind, pool).await?;
        recreated += 1;
    }
    Ok(recreated)
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
    use crate::commands::reminders::parse_timestamp;
    use crate::commands::todos::inner_create_todo;

    fn now() -> chrono::NaiveDateTime {
        parse_timestamp("2026-11-19T12:00").unwrap()
    }

    #[tokio::test]
    async fn status_is_off_by_default_and_unavailable_off_macos() {
        let pool = crate::test_db().await;
        let status = inner_get_status(&pool, &UnsupportedStore).await.unwrap();
        assert!(!status.available);
        assert!(!status.enabled);
        assert_eq!(status.access, AccessState::Unsupported);
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

        inner_create_todo("T", None, None, "not_started", None, &pool)
            .await
            .unwrap();
        let tl = {
            run_pass(&pool, &store, now(), PassMode::Normal).await;
            store.list_id(ListKind::Todos.list_name()).unwrap()
        };
        store.add(&tl, crate::apple_sync::fake::phone("   "));
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let status = inner_get_status(&pool, &store).await.unwrap();
        assert!(status.last_sync_at.is_some());
        assert_eq!(status.skipped.len(), 1);
        assert_eq!(status.skipped[0].reason, "needs a title");

        // Off keeps lists and pairings.
        let status = inner_set_enabled(false, &pool, &store).await.unwrap();
        assert!(!status.enabled);
        assert_eq!(store.items(&tl).len(), 2);
    }

    #[tokio::test]
    async fn recreate_lists_replaces_a_missing_list_and_pushes_again() {
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        inner_set_enabled(true, &pool, &store).await.unwrap();
        let t = inner_create_todo("T", None, None, "not_started", None, &pool)
            .await
            .unwrap();
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let old = store.list_id(ListKind::Todos.list_name()).unwrap();
        let reminders = store.list_id(ListKind::Reminders.list_name()).unwrap();
        store.remove_list(&old);
        let report = run_pass(&pool, &store, now(), PassMode::Normal).await;
        assert!(matches!(
            report.outcome,
            Outcome::Paused {
                kind: PAUSED_LIST_MISSING,
                ..
            }
        ));

        assert_eq!(inner_recreate_lists(&pool, &store).await.unwrap(), 1);
        let new = store.list_id(ListKind::Todos.list_name()).unwrap();
        assert_ne!(new, old);
        assert_eq!(
            store.list_id(ListKind::Reminders.list_name()),
            Some(reminders),
            "the intact list stays"
        );
        let report = run_pass(&pool, &store, now(), PassMode::Normal).await;
        assert_eq!(report.outcome, Outcome::Synced);
        assert_eq!(store.items(&new).len(), 1);
        assert_eq!(store.items(&new)[0].title, "T");
        assert!(crate::commands::todos::inner_get_todo(t.id, &pool)
            .await
            .is_ok());
        assert_eq!(inner_recreate_lists(&pool, &store).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn enabling_starts_the_missed_pass_counts_over() {
        use crate::apple_sync::badger_writes::{load_pairings, set_pairing_missed};
        let pool = crate::test_db().await;
        let store = FakeStore::default();
        inner_set_enabled(true, &pool, &store).await.unwrap();
        inner_create_todo("T", None, None, "not_started", None, &pool)
            .await
            .unwrap();
        run_pass(&pool, &store, now(), PassMode::Normal).await;
        let pairing = load_pairings(&pool).await.unwrap()[0].id;
        set_pairing_missed(pairing, 1, &pool).await.unwrap();
        inner_set_enabled(false, &pool, &store).await.unwrap();
        assert_eq!(load_pairings(&pool).await.unwrap()[0].missed, 1);
        inner_set_enabled(true, &pool, &store).await.unwrap();
        assert_eq!(load_pairings(&pool).await.unwrap()[0].missed, 0);
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
