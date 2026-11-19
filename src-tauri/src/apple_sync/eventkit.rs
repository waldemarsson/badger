//! The EventKit [`ReminderStore`] (macOS only).
//!
//! `EKEventStore` and its objects are not `Send`, so one dedicated OS thread owns the store
//! and runs jobs sent over a channel; the async methods wait on a oneshot reply and never
//! block the Tauri main thread. Completion-block APIs (access request, fetch) hand their
//! result back through a oneshot as well, so the worker isn't blocked either.
//!
//! Release is `panic = "abort"`: nothing here unwraps; failures become [`StoreError`]s.

use std::ptr::NonNull;
use std::sync::{mpsc, Arc, Mutex};

use block2::RcBlock;
use chrono::{Datelike, Local, NaiveDate, NaiveDateTime, TimeZone, Timelike};
use objc2::rc::Retained;
use objc2::runtime::{Bool, NSObjectProtocol, ProtocolObject};
use objc2_event_kit::{
    EKAlarm, EKAuthorizationStatus, EKCalendar, EKEntityType, EKEventStore,
    EKEventStoreChangedNotification, EKReminder,
};
use objc2_foundation::{
    NSArray, NSDate, NSDateComponentUndefined, NSDateComponents, NSError, NSNotification,
    NSNotificationCenter, NSString,
};
use tokio::sync::oneshot;

use super::reply::{self, ReplyError};
use super::{AccessState, PhoneDraft, PhoneDue, PhoneItem, ReminderStore, StoreError};

/// Called (on any thread) when EventKit reports a store change.
pub type OnChange = Arc<dyn Fn() + Send + Sync>;

type Job = Box<dyn FnOnce(&mut Worker) + Send>;

type FetchReply = oneshot::Receiver<Result<Vec<PhoneItem>, StoreError>>;

/// How long a call may take before the pass gives up (it holds the sync lock, which
/// restore waits for). The access prompt waits for the user and has no limit.
const CALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn timed_out() -> StoreError {
    StoreError::Other("Reminders didn't answer in time".to_string())
}

fn reply_error(e: ReplyError) -> StoreError {
    match e {
        ReplyError::TimedOut => timed_out(),
        ReplyError::Stopped => stopped(),
    }
}

pub struct EventKitStore {
    jobs: mpsc::Sender<Job>,
}

fn stopped() -> StoreError {
    StoreError::Other("the EventKit worker stopped".to_string())
}

fn ns_error(e: Retained<NSError>) -> StoreError {
    StoreError::Other(e.localizedDescription().to_string())
}

impl EventKitStore {
    /// Starts the worker thread. If the thread can't start, every call fails with an error.
    pub fn new(on_change: OnChange) -> Self {
        let (jobs, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name("eventkit".to_string())
            .spawn(move || {
                let mut worker = Worker::new(on_change);
                while let Ok(job) = rx.recv() {
                    worker.refresh();
                    job(&mut worker);
                }
            });
        if let Err(e) = spawned {
            tracing::error!("Apple Reminders sync: EventKit thread not started: {}", e);
        }
        EventKitStore { jobs }
    }

    async fn call<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Worker) -> T + Send + 'static,
    ) -> Result<T, StoreError> {
        self.call_or_undo(f, |_, _| {}).await
    }

    /// Like [`Self::call`]; a result that arrives after the caller gave up goes to `undo`
    /// on the worker instead.
    async fn call_or_undo<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Worker) -> T + Send + 'static,
        undo: impl FnOnce(&mut Worker, T) + Send + 'static,
    ) -> Result<T, StoreError> {
        let (tx, rx) = oneshot::channel();
        self.jobs
            .send(Box::new(move |w: &mut Worker| {
                let value = f(w);
                reply::deliver(tx, value, |late| undo(w, late));
            }))
            .map_err(|_| stopped())?;
        reply::await_reply(rx, CALL_TIMEOUT)
            .await
            .map_err(reply_error)
    }
}

impl ReminderStore for EventKitStore {
    async fn access(&self) -> AccessState {
        self.call(|_| access_state())
            .await
            .unwrap_or(AccessState::Denied)
    }

    async fn request_access(&self) -> Result<AccessState, StoreError> {
        let answer = self.call(|w| w.request_access()).await?;
        // The prompt may stay open for a while; the worker keeps serving meanwhile.
        answer.await.map_err(|_| stopped())?;
        // The next job sees the new status and replaces the store (`Worker::refresh`).
        Ok(self.access().await)
    }

    async fn list_exists(&self, list_id: &str) -> Result<bool, StoreError> {
        let id = list_id.to_string();
        self.call(move |w| w.calendar(&id).is_some()).await
    }

    async fn create_list(&self, name: &str) -> Result<String, StoreError> {
        let name = name.to_string();
        self.call(move |w| w.create_list(&name)).await?
    }

    async fn fetch(&self, list_id: &str) -> Result<Vec<PhoneItem>, StoreError> {
        let id = list_id.to_string();
        let reply = self.call(move |w| w.fetch(&id)).await??;
        reply::await_reply(reply, CALL_TIMEOUT)
            .await
            .map_err(reply_error)?
    }

    async fn create_item(
        &self,
        list_id: &str,
        draft: &PhoneDraft,
    ) -> Result<PhoneItem, StoreError> {
        let (id, draft) = (list_id.to_string(), draft.clone());
        self.call_or_undo(
            move |w| w.create_item(&id, &draft),
            |w, late| {
                // Timed out: nobody will pair it, so it would come back as a duplicate.
                if let Ok(item) = late {
                    if let Err(e) = w.delete_item(&item.ek_id) {
                        tracing::error!("Apple Reminders sync: late item not removed: {}", e);
                    }
                }
            },
        )
        .await?
    }

    async fn update_item(&self, ek_id: &str, draft: &PhoneDraft) -> Result<PhoneItem, StoreError> {
        let (id, draft) = (ek_id.to_string(), draft.clone());
        self.call(move |w| w.update_item(&id, &draft)).await?
    }

    async fn delete_item(&self, ek_id: &str) -> Result<(), StoreError> {
        let id = ek_id.to_string();
        self.call(move |w| w.delete_item(&id)).await?
    }
}

fn access_state() -> AccessState {
    // SAFETY: a class method with a valid entity type.
    let status = unsafe { EKEventStore::authorizationStatusForEntityType(EKEntityType::Reminder) };
    // FullAccess is also macOS 13's "Authorized".
    if status == EKAuthorizationStatus::FullAccess {
        AccessState::Granted
    } else if status == EKAuthorizationStatus::NotDetermined {
        AccessState::NotDetermined
    } else if status == EKAuthorizationStatus::Restricted {
        AccessState::Restricted
    } else {
        // Denied, or write-only (Badger needs to read).
        AccessState::Denied
    }
}

/// Lives on the worker thread only.
struct Worker {
    store: Retained<EKEventStore>,
    /// The authorization status `store` was created under.
    created_under: AccessState,
    _observer: Retained<ProtocolObject<dyn NSObjectProtocol>>,
}

impl Worker {
    fn new(on_change: OnChange) -> Self {
        let created_under = access_state();
        // SAFETY: plain object creation.
        let store = unsafe { EKEventStore::new() };
        let block: RcBlock<dyn Fn(NonNull<NSNotification>)> =
            RcBlock::new(move |_note: NonNull<NSNotification>| on_change());
        // SAFETY: the block only calls `on_change`, which is Send + Sync and just wakes the
        // runner. With no queue the block runs synchronously on whichever thread posts the
        // notification, so delivery doesn't depend on this thread (which has no run loop).
        // Any sender: the store is replaced on access changes. The 60 s tick backs this up.
        let observer = unsafe {
            NSNotificationCenter::defaultCenter().addObserverForName_object_queue_usingBlock(
                Some(EKEventStoreChangedNotification),
                None,
                None,
                &block,
            )
        };
        Worker {
            store,
            created_under,
            _observer: observer,
        }
    }

    /// A store created before access was granted doesn't see the reminders. Runs before
    /// every job: when the status changed since the store was made (answered in the prompt
    /// or changed later in System Settings), a fresh store replaces it.
    fn refresh(&mut self) {
        let now = access_state();
        if now != self.created_under {
            // SAFETY: plain object creation.
            self.store = unsafe { EKEventStore::new() };
            self.created_under = now;
        }
    }

    /// Prompts for access; the reply arrives when the user answers.
    fn request_access(&self) -> oneshot::Receiver<()> {
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block: RcBlock<dyn Fn(Bool, *mut NSError)> =
            RcBlock::new(move |_granted: Bool, _error: *mut NSError| {
                if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = tx.send(());
                }
            });
        if objc2::available!(macos = 14.0) {
            // SAFETY: a valid block; EventKit copies it.
            unsafe {
                self.store
                    .requestFullAccessToRemindersWithCompletion(RcBlock::as_ptr(&block))
            };
        } else {
            request_access_legacy(&self.store, &block);
        }
        rx
    }

    fn calendar(&self, id: &str) -> Option<Retained<EKCalendar>> {
        // SAFETY: a valid string.
        unsafe { self.store.calendarWithIdentifier(&NSString::from_str(id)) }
    }

    fn reminder(&self, ek_id: &str) -> Option<Retained<EKReminder>> {
        // SAFETY: a valid string.
        let item = unsafe {
            self.store
                .calendarItemWithIdentifier(&NSString::from_str(ek_id))
        }?;
        item.downcast::<EKReminder>().ok()
    }

    fn create_list(&self, name: &str) -> Result<String, StoreError> {
        // SAFETY: calls on this thread's store with valid arguments.
        unsafe {
            let source = self
                .store
                .defaultCalendarForNewReminders()
                .and_then(|c| c.source())
                .ok_or_else(|| {
                    StoreError::Other("there is no default Reminders account".to_string())
                })?;
            let calendar =
                EKCalendar::calendarForEntityType_eventStore(EKEntityType::Reminder, &self.store);
            calendar.setTitle(&NSString::from_str(name));
            calendar.setSource(Some(&source));
            self.store
                .saveCalendar_commit_error(&calendar, true)
                .map_err(ns_error)?;
            Ok(calendar.calendarIdentifier().to_string())
        }
    }

    fn fetch(&self, list_id: &str) -> Result<FetchReply, StoreError> {
        let calendar = self.calendar(list_id).ok_or(StoreError::ListMissing)?;
        let (tx, rx) = oneshot::channel();
        let tx = Mutex::new(Some(tx));
        let block: RcBlock<dyn Fn(*mut NSArray<EKReminder>)> =
            RcBlock::new(move |found: *mut NSArray<EKReminder>| {
                // SAFETY: EventKit passes a valid array or nil.
                let result = match unsafe { found.as_ref() } {
                    Some(list) => Ok(list.to_vec().into_iter().map(|r| to_item(&r)).collect()),
                    None => Err(StoreError::Other("the list couldn't be read".to_string())),
                };
                if let Some(tx) = tx.lock().ok().and_then(|mut slot| slot.take()) {
                    let _ = tx.send(result);
                }
            });
        let calendars = NSArray::from_retained_slice(&[calendar]);
        // SAFETY: a predicate from this store and a valid block (EventKit copies it).
        unsafe {
            let predicate = self
                .store
                .predicateForRemindersInCalendars(Some(&calendars));
            let _request = self
                .store
                .fetchRemindersMatchingPredicate_completion(&predicate, &block);
        }
        Ok(rx)
    }

    fn create_item(&self, list_id: &str, draft: &PhoneDraft) -> Result<PhoneItem, StoreError> {
        let calendar = self.calendar(list_id).ok_or(StoreError::ListMissing)?;
        // SAFETY: objects of this thread's store.
        unsafe {
            let reminder = EKReminder::reminderWithEventStore(&self.store);
            reminder.setCalendar(Some(&calendar));
            apply_draft(&reminder, draft);
            self.store
                .saveReminder_commit_error(&reminder, true)
                .map_err(ns_error)?;
            Ok(to_item(&reminder))
        }
    }

    fn update_item(&self, ek_id: &str, draft: &PhoneDraft) -> Result<PhoneItem, StoreError> {
        let reminder = self.reminder(ek_id).ok_or(StoreError::ItemMissing)?;
        apply_draft(&reminder, draft);
        // SAFETY: a reminder of this thread's store.
        unsafe { self.store.saveReminder_commit_error(&reminder, true) }.map_err(ns_error)?;
        Ok(to_item(&reminder))
    }

    fn delete_item(&self, ek_id: &str) -> Result<(), StoreError> {
        let Some(reminder) = self.reminder(ek_id) else {
            return Ok(());
        };
        // SAFETY: a reminder of this thread's store.
        unsafe { self.store.removeReminder_commit_error(&reminder, true) }.map_err(ns_error)
    }
}

/// macOS 13.4–13.x: the pre-14 access request.
#[allow(deprecated)]
fn request_access_legacy(store: &EKEventStore, block: &RcBlock<dyn Fn(Bool, *mut NSError)>) {
    // SAFETY: a valid block; EventKit copies it.
    unsafe {
        store.requestAccessToEntityType_completion(EKEntityType::Reminder, RcBlock::as_ptr(block))
    };
}

/// A local wall-clock time from an instant.
fn local_from_nsdate(date: &NSDate) -> Option<NaiveDateTime> {
    let secs = date.timeIntervalSince1970();
    if !secs.is_finite() {
        return None;
    }
    let utc = chrono::DateTime::from_timestamp(secs.round() as i64, 0)?;
    Some(utc.with_timezone(&Local).naive_local())
}

fn nsdate_from_local(t: NaiveDateTime) -> Option<Retained<NSDate>> {
    let local = Local.from_local_datetime(&t).earliest()?;
    Some(NSDate::dateWithTimeIntervalSince1970(
        local.timestamp() as f64
    ))
}

fn defined(n: isize) -> Option<isize> {
    (n != NSDateComponentUndefined).then_some(n)
}

/// Floating date components: a date, plus the time for a date-time.
fn components(due: PhoneDue) -> Retained<NSDateComponents> {
    let c = NSDateComponents::new();
    let date = due.date();
    c.setYear(date.year() as isize);
    c.setMonth(date.month() as isize);
    c.setDay(date.day() as isize);
    if let PhoneDue::DateTime(t) = due {
        c.setHour(t.hour() as isize);
        c.setMinute(t.minute() as isize);
        c.setSecond(t.second() as isize);
    }
    c
}

fn read_due(reminder: &EKReminder) -> Option<PhoneDue> {
    // SAFETY: a property read.
    let c = unsafe { reminder.dueDateComponents() }?;
    let date = NaiveDate::from_ymd_opt(
        i32::try_from(defined(c.year())?).ok()?,
        u32::try_from(defined(c.month())?).ok()?,
        u32::try_from(defined(c.day())?).ok()?,
    )?;
    let Some(hour) = defined(c.hour()) else {
        return Some(PhoneDue::Date(date));
    };
    // A zoned due converts through its instant; a floating one is local wall-clock time.
    if c.timeZone().is_some() {
        if let Some(t) = c.date().and_then(|d| local_from_nsdate(&d)) {
            return Some(PhoneDue::DateTime(t));
        }
    }
    let minute = defined(c.minute()).unwrap_or(0);
    let second = defined(c.second()).unwrap_or(0);
    date.and_hms_opt(
        u32::try_from(hour).ok()?,
        u32::try_from(minute).ok()?,
        u32::try_from(second).ok()?,
    )
    .map(PhoneDue::DateTime)
}

fn to_item(reminder: &EKReminder) -> PhoneItem {
    // SAFETY: property reads.
    unsafe {
        let alarms = reminder
            .alarms()
            .map(|list| list.to_vec())
            .unwrap_or_default()
            .iter()
            .filter_map(|a| a.absoluteDate())
            .filter_map(|d| local_from_nsdate(&d))
            .collect();
        PhoneItem {
            ek_id: reminder.calendarItemIdentifier().to_string(),
            external_id: reminder
                .calendarItemExternalIdentifier()
                .map(|s| s.to_string()),
            title: reminder.title().to_string(),
            notes: reminder.notes().map(|s| s.to_string()),
            due: read_due(reminder),
            alarms,
            completed: reminder.isCompleted(),
            recurring: reminder.hasRecurrenceRules(),
        }
    }
}

fn apply_draft(reminder: &EKReminder, d: &PhoneDraft) {
    // SAFETY: property writes on a reminder of the worker's store.
    unsafe {
        reminder.setTitle(Some(&NSString::from_str(&d.title)));
        if let Some(notes) = &d.notes {
            reminder.setNotes(notes.as_deref().map(NSString::from_str).as_deref());
        }
        if let Some(due) = d.due {
            reminder.setDueDateComponents(due.map(components).as_deref());
        }
        if let Some(at) = d.alarm {
            if let Some(date) = nsdate_from_local(at) {
                // Our one absolute alarm replaces other absolute ones; relative alarms stay.
                let mut alarms: Vec<Retained<EKAlarm>> = reminder
                    .alarms()
                    .map(|list| list.to_vec())
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|a| a.absoluteDate().is_none())
                    .collect();
                alarms.push(EKAlarm::alarmWithAbsoluteDate(&date));
                reminder.setAlarms(Some(&NSArray::from_retained_slice(&alarms)));
            }
        }
        if d.strip_recurrence && reminder.hasRecurrenceRules() {
            reminder.setRecurrenceRules(None);
        }
        if reminder.isCompleted() != d.completed {
            reminder.setCompleted(d.completed);
        }
    }
}
