//! Two-way sync of one-off reminders and open todos with two Apple Reminders lists
//! ("Badger Reminders", "Badger Todos") through EventKit; iCloud carries them to the phone.
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

use crate::app_settings::{APPLE_SYNC_REMINDERS_LIST_KEY, APPLE_SYNC_TODOS_LIST_KEY};
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

/// Which Badger list an item lives in; the list decides the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ListKind {
    Reminders,
    Todos,
}

impl ListKind {
    pub const ALL: [ListKind; 2] = [ListKind::Todos, ListKind::Reminders];

    /// `apple_sync_items.Kind` value.
    pub fn kind(self) -> &'static str {
        match self {
            ListKind::Reminders => "reminder",
            ListKind::Todos => "todo",
        }
    }

    /// The list's name when Badger creates it. Debug builds use separate lists, so a
    /// development database never touches the real ones.
    pub fn list_name(self) -> &'static str {
        match (self, cfg!(debug_assertions)) {
            (ListKind::Reminders, false) => "Badger Reminders",
            (ListKind::Todos, false) => "Badger Todos",
            (ListKind::Reminders, true) => "Badger Dev Reminders",
            (ListKind::Todos, true) => "Badger Dev Todos",
        }
    }

    /// `app_settings` key of the stored list id.
    pub fn settings_key(self) -> &'static str {
        match self {
            ListKind::Reminders => APPLE_SYNC_REMINDERS_LIST_KEY,
            ListKind::Todos => APPLE_SYNC_TODOS_LIST_KEY,
        }
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

/// An item in one of the two lists, as the store reports it.
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
    /// The item's absolute alarms become exactly this one (relative alarms stay).
    pub alarm: Option<NaiveDateTime>,
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
    #[error("A Badger list is missing in Reminders")]
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

/// The fired reminders whose native macOS notification the scheduler skips because the
/// phone notifies: only while sync is on and healthy (not paused, last pass without
/// errors), for paired one-offs whose pushed phone time equals the firing `RemindAt`.
/// On any error nothing is skipped (a duplicate beats a missed reminder).
pub async fn phone_notifies(
    pool: &sqlx::Pool<sqlx::Sqlite>,
    fired: &[crate::commands::reminders::Reminder],
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
    for r in fired.iter().filter(|r| r.repeat == "none") {
        let pushed: Result<Option<(Option<String>,)>, sqlx::Error> = sqlx::query_as(
            "SELECT PhoneDue FROM apple_sync_items WHERE Kind = 'reminder' AND BadgerId = ?",
        )
        .bind(r.id)
        .fetch_optional(pool)
        .await;
        match pushed {
            Ok(Some((Some(due),))) if due == badger_writes::normalize_timestamp(&r.remind_at) => {
                skip.insert(r.id);
            }
            Ok(_) => {}
            Err(e) => {
                tracing::error!("Apple Reminders sync: couldn't read a pairing: {}", e);
                return std::collections::HashSet::new();
            }
        }
    }
    skip
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
/// Emitted (no payload) when todos changed outside the frontend's own calls (sync); the
/// todos board, Today and TodoModal re-read on it.
pub const TODOS_CHANGED_EVENT: &str = "todos-changed";

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
    fn debug_builds_use_the_dev_lists() {
        if cfg!(debug_assertions) {
            assert_eq!(ListKind::Reminders.list_name(), "Badger Dev Reminders");
            assert_eq!(ListKind::Todos.list_name(), "Badger Dev Todos");
        } else {
            assert_eq!(ListKind::Reminders.list_name(), "Badger Reminders");
        }
        assert_eq!(ListKind::Reminders.kind(), "reminder");
    }
}
