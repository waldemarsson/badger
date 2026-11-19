//! In-memory [`ReminderStore`] for tests. Tests act as the phone through the helpers
//! (`add`, `edit`, `remove`, …) and inspect what sync wrote.

use std::collections::BTreeMap;
use std::sync::Mutex;

use super::{AccessState, PhoneDraft, PhoneItem, ReminderStore, StoreError};

#[derive(Debug)]
struct State {
    access: AccessState,
    /// list id → (name, items in insertion order)
    lists: BTreeMap<String, (String, Vec<PhoneItem>)>,
    next_id: u64,
    /// Write calls (create/update/delete) sync made, failed ones included.
    writes: usize,
    fail_writes: bool,
}

#[derive(Debug)]
pub struct FakeStore {
    state: Mutex<State>,
}

impl Default for FakeStore {
    fn default() -> Self {
        FakeStore {
            state: Mutex::new(State {
                access: AccessState::Granted,
                lists: BTreeMap::new(),
                next_id: 1,
                writes: 0,
                fail_writes: false,
            }),
        }
    }
}

fn apply(item: &mut PhoneItem, draft: &PhoneDraft) {
    item.title = draft.title.clone();
    if let Some(notes) = &draft.notes {
        item.notes = notes.clone();
    }
    if let Some(due) = draft.due {
        item.due = due;
    }
    if let Some(alarm) = draft.alarm {
        item.alarms = vec![alarm];
    }
    item.completed = draft.completed;
    if draft.strip_recurrence {
        item.recurring = false;
    }
}

impl FakeStore {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap()
    }

    pub fn set_access(&self, access: AccessState) {
        self.lock().access = access;
    }

    pub fn fail_writes(&self, fail: bool) {
        self.lock().fail_writes = fail;
    }

    /// Number of create/update/delete calls sync made so far (failed ones included).
    pub fn writes(&self) -> usize {
        self.lock().writes
    }

    /// Phone-side create: a new item in `list_id`; returns its id.
    pub fn add(&self, list_id: &str, mut item: PhoneItem) -> String {
        let mut s = self.lock();
        let n = s.next_id;
        s.next_id += 1;
        if item.ek_id.is_empty() {
            item.ek_id = format!("ek-{n}");
        }
        if item.external_id.is_none() {
            item.external_id = Some(format!("ext-{n}"));
        }
        let id = item.ek_id.clone();
        if let Some((_, items)) = s.lists.get_mut(list_id) {
            items.push(item);
        }
        id
    }

    /// Phone-side edit.
    pub fn edit(&self, ek_id: &str, f: impl FnOnce(&mut PhoneItem)) {
        let mut s = self.lock();
        for (_, items) in s.lists.values_mut() {
            if let Some(item) = items.iter_mut().find(|i| i.ek_id == ek_id) {
                f(item);
                return;
            }
        }
        panic!("no item {ek_id}");
    }

    /// Phone-side delete.
    pub fn remove(&self, ek_id: &str) {
        let mut s = self.lock();
        for (_, items) in s.lists.values_mut() {
            items.retain(|i| i.ek_id != ek_id);
        }
    }

    /// Phone-side move to another list (keeps the ids, like EventKit).
    pub fn move_to(&self, ek_id: &str, list_id: &str) {
        let mut s = self.lock();
        let mut moved = None;
        for (_, items) in s.lists.values_mut() {
            if let Some(pos) = items.iter().position(|i| i.ek_id == ek_id) {
                moved = Some(items.remove(pos));
            }
        }
        if let (Some(item), Some((_, items))) = (moved, s.lists.get_mut(list_id)) {
            items.push(item);
        }
    }

    /// Phone-side list delete.
    pub fn remove_list(&self, list_id: &str) {
        self.lock().lists.remove(list_id);
    }

    /// Empties a list without deleting it (an iCloud hiccup, or a mass delete).
    pub fn clear_list(&self, list_id: &str) {
        if let Some((_, items)) = self.lock().lists.get_mut(list_id) {
            items.clear();
        }
    }

    pub fn items(&self, list_id: &str) -> Vec<PhoneItem> {
        self.lock()
            .lists
            .get(list_id)
            .map(|(_, items)| items.clone())
            .unwrap_or_default()
    }

    pub fn item(&self, ek_id: &str) -> Option<PhoneItem> {
        self.lock()
            .lists
            .values()
            .flat_map(|(_, items)| items.iter())
            .find(|i| i.ek_id == ek_id)
            .cloned()
    }

    /// The id of the list called `name`.
    pub fn list_id(&self, name: &str) -> Option<String> {
        self.lock()
            .lists
            .iter()
            .find(|(_, (n, _))| n == name)
            .map(|(id, _)| id.clone())
    }

    pub fn list_count(&self) -> usize {
        self.lock().lists.len()
    }

    /// EventKit changes an item's local id (the external id stays).
    pub fn change_ek_id(&self, ek_id: &str, new_id: &str) {
        self.edit(ek_id, |i| i.ek_id = new_id.to_string());
    }

    fn write(&self) -> Result<std::sync::MutexGuard<'_, State>, StoreError> {
        let mut s = self.lock();
        if s.access != AccessState::Granted {
            return Err(StoreError::AccessDenied);
        }
        if s.fail_writes {
            return Err(StoreError::Other("write failed".to_string()));
        }
        s.writes += 1;
        Ok(s)
    }
}

impl ReminderStore for FakeStore {
    async fn access(&self) -> AccessState {
        self.lock().access
    }

    async fn request_access(&self) -> Result<AccessState, StoreError> {
        let mut s = self.lock();
        if s.access == AccessState::NotDetermined {
            s.access = AccessState::Granted;
        }
        Ok(s.access)
    }

    async fn list_exists(&self, list_id: &str) -> Result<bool, StoreError> {
        let s = self.lock();
        if s.access != AccessState::Granted {
            return Err(StoreError::AccessDenied);
        }
        Ok(s.lists.contains_key(list_id))
    }

    async fn create_list(&self, name: &str) -> Result<String, StoreError> {
        let mut s = self.lock();
        if s.access != AccessState::Granted {
            return Err(StoreError::AccessDenied);
        }
        let id = format!("cal-{}", s.next_id);
        s.next_id += 1;
        s.lists.insert(id.clone(), (name.to_string(), Vec::new()));
        Ok(id)
    }

    async fn fetch(&self, list_id: &str) -> Result<Vec<PhoneItem>, StoreError> {
        let s = self.lock();
        if s.access != AccessState::Granted {
            return Err(StoreError::AccessDenied);
        }
        s.lists
            .get(list_id)
            .map(|(_, items)| items.clone())
            .ok_or(StoreError::ListMissing)
    }

    async fn create_item(
        &self,
        list_id: &str,
        draft: &PhoneDraft,
    ) -> Result<PhoneItem, StoreError> {
        let mut s = self.write()?;
        let n = s.next_id;
        s.next_id += 1;
        let mut item = PhoneItem {
            ek_id: format!("ek-{n}"),
            external_id: Some(format!("ext-{n}")),
            title: String::new(),
            notes: None,
            due: None,
            alarms: Vec::new(),
            completed: false,
            recurring: false,
        };
        apply(&mut item, draft);
        let (_, items) = s.lists.get_mut(list_id).ok_or(StoreError::ListMissing)?;
        items.push(item.clone());
        Ok(item)
    }

    async fn update_item(&self, ek_id: &str, draft: &PhoneDraft) -> Result<PhoneItem, StoreError> {
        let mut s = self.write()?;
        for (_, items) in s.lists.values_mut() {
            if let Some(item) = items.iter_mut().find(|i| i.ek_id == ek_id) {
                apply(item, draft);
                return Ok(item.clone());
            }
        }
        Err(StoreError::ItemMissing)
    }

    async fn delete_item(&self, ek_id: &str) -> Result<(), StoreError> {
        let mut s = self.write()?;
        for (_, items) in s.lists.values_mut() {
            items.retain(|i| i.ek_id != ek_id);
        }
        Ok(())
    }
}

/// A phone item with just a title; tests fill in the rest.
pub fn phone(title: &str) -> PhoneItem {
    PhoneItem {
        ek_id: String::new(),
        external_id: None,
        title: title.to_string(),
        notes: None,
        due: None,
        alarms: Vec::new(),
        completed: false,
        recurring: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apple_sync::PhoneDue;
    use chrono::NaiveDate;

    fn draft(title: &str) -> PhoneDraft {
        PhoneDraft {
            title: title.to_string(),
            notes: None,
            due: None,
            alarm: None,
            completed: false,
            strip_recurrence: false,
        }
    }

    #[tokio::test]
    async fn create_fetch_update_delete_round_trip() {
        let store = FakeStore::default();
        let list = store.create_list("Badger Todos").await.unwrap();
        assert!(store.list_exists(&list).await.unwrap());
        assert_eq!(store.list_id("Badger Todos"), Some(list.clone()));

        let created = store.create_item(&list, &draft("A")).await.unwrap();
        assert!(created.ek_id.starts_with("ek-"));
        assert!(created.external_id.is_some());

        let due = PhoneDue::Date(NaiveDate::from_ymd_opt(2026, 11, 20).unwrap());
        let mut d = draft("B");
        d.due = Some(Some(due));
        d.notes = Some(Some("n".to_string()));
        d.completed = true;
        let updated = store.update_item(&created.ek_id, &d).await.unwrap();
        assert_eq!(updated.title, "B");
        assert_eq!(updated.due, Some(due));
        assert!(updated.completed);

        // Completed items are fetched too.
        let fetched = store.fetch(&list).await.unwrap();
        assert_eq!(fetched, vec![updated.clone()]);

        // A draft without notes/due leaves them alone.
        let kept = store
            .update_item(&created.ek_id, &draft("C"))
            .await
            .unwrap();
        assert_eq!(kept.notes.as_deref(), Some("n"));
        assert_eq!(kept.due, Some(due));

        store.delete_item(&created.ek_id).await.unwrap();
        store.delete_item(&created.ek_id).await.unwrap(); // already gone: fine
        assert!(store.fetch(&list).await.unwrap().is_empty());
        assert_eq!(
            store.update_item(&created.ek_id, &draft("D")).await,
            Err(StoreError::ItemMissing)
        );
        assert_eq!(
            store.writes(),
            6,
            "every write call counts, failed ones too"
        );
    }

    #[tokio::test]
    async fn missing_list_and_lost_access_are_errors() {
        let store = FakeStore::default();
        let list = store.create_list("L").await.unwrap();
        store.remove_list(&list);
        assert_eq!(store.fetch(&list).await, Err(StoreError::ListMissing));
        assert!(!store.list_exists(&list).await.unwrap());

        store.set_access(AccessState::Denied);
        assert_eq!(store.fetch("x").await, Err(StoreError::AccessDenied));
        assert_eq!(
            store.create_item("x", &draft("A")).await,
            Err(StoreError::AccessDenied)
        );
    }

    #[tokio::test]
    async fn phone_side_helpers_keep_ids_on_move() {
        let store = FakeStore::default();
        let a = store.create_list("A").await.unwrap();
        let b = store.create_list("B").await.unwrap();
        let id = store.add(&a, phone("x"));
        store.move_to(&id, &b);
        assert!(store.items(&a).is_empty());
        assert_eq!(store.items(&b)[0].ek_id, id);
        store.change_ek_id(&id, "ek-new");
        assert!(store.item("ek-new").is_some());
        assert_eq!(store.writes(), 0, "phone-side changes aren't sync writes");
    }
}
