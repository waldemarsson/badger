//! Two-way sync of tasks with one Apple Reminders list ("Badger"; "Badger Dev" in debug
//! builds) through EventKit; iCloud carries it to the phone. Repeating and Mac-only tasks
//! stay off the phone. Badger owns reminders: each pushes as one absolute alarm.
//!
//! The engine ([`engine::run_pass`]) only sees the [`ReminderStore`] trait and the pool, so
//! all merge logic runs in `cargo test` on any OS against the in-memory fake. The EventKit
//! store is macOS-only; elsewhere [`UnsupportedStore`] reports that sync isn't available.
//! [`runner`] owns the background task, [`badger_writes`] the conditional Badger writes.

pub mod badger_writes;
pub mod engine;
#[cfg(target_os = "macos")]
pub mod eventkit;
#[cfg(test)]
pub mod fake;
pub mod reply;
pub mod runner;

use std::future::Future;

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::AppError;

/// The store the app uses on this platform.
#[cfg(target_os = "macos")]
pub type PlatformStore = eventkit::EventKitStore;
#[cfg(not(target_os = "macos"))]
pub type PlatformStore = UnsupportedStore;

/// The platform store; `on_change` is called (on any thread) when EventKit reports a
/// change.
#[cfg(target_os = "macos")]
pub fn platform_store(on_change: std::sync::Arc<dyn Fn() + Send + Sync>) -> PlatformStore {
    eventkit::EventKitStore::new(on_change)
}

#[cfg(not(target_os = "macos"))]
pub fn platform_store(_on_change: std::sync::Arc<dyn Fn() + Send + Sync>) -> PlatformStore {
    UnsupportedStore
}

/// The list's name when Badger creates it. Debug builds use a separate list, so a
/// development database never touches the real one. Found again by its stored id, so a
/// rename on the phone is fine.
pub fn list_name() -> &'static str {
    if cfg!(debug_assertions) {
        "Badger Dev"
    } else {
        "Badger"
    }
}

/// The lists synced before tasks; never touched, Settings says they can be deleted.
pub fn old_list_names() -> [&'static str; 2] {
    if cfg!(debug_assertions) {
        ["Badger Dev Todos", "Badger Dev Reminders"]
    } else {
        ["Badger Todos", "Badger Reminders"]
    }
}

/// A phone item's due: a floating local date, or a date and time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhoneDue {
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

impl PhoneDue {
    pub fn date(self) -> NaiveDate {
        match self {
            PhoneDue::Date(d) => d,
            PhoneDue::DateTime(t) => t.date(),
        }
    }
}

/// An item in the list, as the store reports it.
#[derive(Debug, Clone, PartialEq)]
pub struct PhoneItem {
    /// EventKit `calendarItemIdentifier` (device-local).
    pub ek_id: String,
    /// EventKit `calendarItemExternalIdentifier` (the server's id; may be missing).
    pub external_id: Option<String>,
    pub title: String,
    pub notes: Option<String>,
    pub due: Option<PhoneDue>,
    /// Local times of the item's absolute-date alarms.
    pub alarms: Vec<NaiveDateTime>,
    pub completed: bool,
    /// Whether the item has a repeat rule.
    pub recurring: bool,
}

/// What to write to a phone item. `None` fields are left as they are.
#[derive(Debug, Clone, PartialEq)]
pub struct PhoneDraft {
    pub title: String,
    pub notes: Option<Option<String>>,
    pub due: Option<Option<PhoneDue>>,
    /// The item's absolute alarms become exactly these (relative alarms stay).
    pub alarms: Option<Vec<NaiveDateTime>>,
    pub completed: bool,
    pub strip_recurrence: bool,
}

/// Access to Reminders, as EventKit reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccessState {
    NotDetermined,
    Granted,
    Denied,
    Restricted,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("Badger has no access to Reminders. Grant access in Settings.")]
    AccessDenied,
    #[error("The Badger list is missing in Reminders")]
    ListMissing,
    #[error("The reminder no longer exists on the phone")]
    ItemMissing,
    #[error("Apple Reminders sync is only available on macOS")]
    Unsupported,
    #[error("Reminders: {0}")]
    Other(String),
}

impl From<StoreError> for AppError {
    fn from(e: StoreError) -> Self {
        AppError::Custom(e.to_string())
    }
}

/// The phone side of the sync. Implementations must never panic: release is
/// `panic = "abort"`.
pub trait ReminderStore: Send + Sync + 'static {
    fn access(&self) -> impl Future<Output = AccessState> + Send;
    /// Asks for full access (prompts the first time); returns the resulting state.
    fn request_access(&self) -> impl Future<Output = Result<AccessState, StoreError>> + Send;
    fn list_exists(&self, list_id: &str) -> impl Future<Output = Result<bool, StoreError>> + Send;
    /// Creates a reminders list in the default reminders source; returns its id.
    fn create_list(&self, name: &str) -> impl Future<Output = Result<String, StoreError>> + Send;
    /// Every item in the list, completed ones included. `ListMissing` when the id doesn't
    /// resolve.
    fn fetch(
        &self,
        list_id: &str,
    ) -> impl Future<Output = Result<Vec<PhoneItem>, StoreError>> + Send;
    fn create_item(
        &self,
        list_id: &str,
        draft: &PhoneDraft,
    ) -> impl Future<Output = Result<PhoneItem, StoreError>> + Send;
    /// Writes the draft and returns the item as stored. `ItemMissing` when it's gone.
    fn update_item(
        &self,
        ek_id: &str,
        draft: &PhoneDraft,
    ) -> impl Future<Output = Result<PhoneItem, StoreError>> + Send;
    /// Deletes the item; an item that is already gone is not an error.
    fn delete_item(&self, ek_id: &str) -> impl Future<Output = Result<(), StoreError>> + Send;
}

/// The store on platforms without EventKit: every call reports that sync is unavailable.
#[derive(Debug, Default)]
pub struct UnsupportedStore;

impl ReminderStore for UnsupportedStore {
    async fn access(&self) -> AccessState {
        AccessState::Unsupported
    }
    async fn request_access(&self) -> Result<AccessState, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn list_exists(&self, _list_id: &str) -> Result<bool, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn create_list(&self, _name: &str) -> Result<String, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn fetch(&self, _list_id: &str) -> Result<Vec<PhoneItem>, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn create_item(
        &self,
        _list_id: &str,
        _draft: &PhoneDraft,
    ) -> Result<PhoneItem, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn update_item(
        &self,
        _ek_id: &str,
        _draft: &PhoneDraft,
    ) -> Result<PhoneItem, StoreError> {
        Err(StoreError::Unsupported)
    }
    async fn delete_item(&self, _ek_id: &str) -> Result<(), StoreError> {
        Err(StoreError::Unsupported)
    }
}

/// The sent task reminders whose native macOS notification the scheduler skips because the
/// phone alarms for them: only while sync is on and healthy (not paused, last pass without
/// errors), for reminders of a paired task whose fire time is among the alarms last pushed
/// to its phone item (`PhoneAlarms`). A snoozed reminder fires at a time the phone doesn't
/// have, so the Mac notifies. Mac-only and repeating tasks always notify on the Mac, even
/// while a pairing the next pass drops is still there. On any error nothing is skipped (a
/// duplicate beats a missed reminder). Returns reminder ids.
pub async fn phone_alarms_cover(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    fired: &[crate::commands::tasks::FiredReminder],
) -> std::collections::HashSet<i64> {
    let mut skip = std::collections::HashSet::new();
    let settings = match crate::app_settings::get_apple_sync_settings(pool).await {
        Ok(s) => s,
        Err(e) => {
            tracing::error!("Apple Reminders sync: couldn't read settings: {}", e);
            return skip;
        }
    };
    if !settings.enabled || settings.paused.is_some() || settings.last_error.is_some() {
        return skip;
    }
    for f in fired {
        let pushed: Result<Option<String>, sqlx::Error> = sqlx::query_scalar(
            "SELECT p.PhoneAlarms FROM apple_sync_items p JOIN tasks t ON t.id = p.BadgerId \
             WHERE p.BadgerId = ? AND t.MacOnly = 0 AND t.Repeat = 'none'",
        )
        .bind(f.task_id)
        .fetch_optional(pool)
        .await;
        match pushed {
            Ok(Some(json)) => {
                let alarms: Vec<String> = serde_json::from_str(&json).unwrap_or_default();
                if alarms.contains(&alarm_key(f.fire_at)) {
                    skip.insert(f.reminder_id);
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("Apple Reminders sync: couldn't read a pairing: {}", e);
                return std::collections::HashSet::new();
            }
        }
    }
    skip
}

/// How an alarm time is stored in `PhoneAlarms`: whole seconds, storage format.
pub fn alarm_key(t: NaiveDateTime) -> String {
    use chrono::Timelike;
    crate::recurrence::format_timestamp(t.with_nanosecond(0).unwrap_or(t))
}

/// A phone item a pass left alone, and why ("needs a date", "needs a title").
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkippedItem {
    pub title: String,
    pub list: String,
    pub reason: String,
}

/// Why a pass was held back by the mass-delete guard (stored in `apple_sync_paused`).
pub const PAUSED_ACCESS: &str = "access";
pub const PAUSED_LIST_MISSING: &str = "list_missing";
pub const PAUSED_EMPTY_FETCH: &str = "empty_fetch";

/// Emitted (no payload) when the sync status changes; Settings re-reads it.
pub const STATUS_EVENT: &str = "apple-sync-status";

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unsupported_store_reports_macos_only() {
        let store = UnsupportedStore;
        assert_eq!(store.access().await, AccessState::Unsupported);
        let err = store.fetch("x").await.unwrap_err();
        assert_eq!(
            AppError::from(err).to_string(),
            "Apple Reminders sync is only available on macOS"
        );
    }

    #[test]
    fn debug_builds_use_the_dev_list() {
        if cfg!(debug_assertions) {
            assert_eq!(list_name(), "Badger Dev");
            assert_eq!(
                old_list_names(),
                ["Badger Dev Todos", "Badger Dev Reminders"]
            );
        } else {
            assert_eq!(list_name(), "Badger");
        }
    }
}
