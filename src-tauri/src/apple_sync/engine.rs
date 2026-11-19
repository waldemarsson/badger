//! One sync pass: load Badger's rows in scope (plus every paired row), fetch both lists,
//! merge each pair field by field against the last-synced snapshots, apply both sides,
//! and import or push what is unpaired.
//!
//! Order: todo pairings, then reminder pairings (loaded after, so a todo deleted on the
//! phone has already cascaded its reminders away), then pushes and imports of both kinds
//! (after every dropped pairing, so an item moved between lists can be paired again).

use std::collections::{HashMap, HashSet};

use chrono::{NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use sqlx::{Pool, Sqlite};

use crate::app_settings::{
    get_apple_sync_settings, set_apple_sync_list_id, set_apple_sync_outcome,
    set_apple_sync_restore_pending,
};
use crate::commands::reminders::{format_timestamp, local_exists, parse_timestamp, resolve_gap};
use crate::commands::todos::clean_notes;
use crate::AppError;

use super::badger_writes::{
    apply_reminder, apply_todo, badger_edited, clean_reminder_text, delete_and_unpair,
    delete_pairing, import_reminder, import_todo, insert_pairing, load_pairings, load_reminder,
    load_reminders, load_todo, load_todos, set_pairing_ek_id, set_pairing_missed, update_pairing,
    BReminder, BTodo, Pairing, PairingState, Snap,
};
use super::{
    AccessState, ListKind, PhoneDraft, PhoneDue, PhoneItem, ReminderStore, SkippedItem, StoreError,
    PAUSED_ACCESS, PAUSED_EMPTY_FETCH, PAUSED_LIST_MISSING,
};

/// Time a date-only phone reminder gets.
const DATE_ONLY_TIME: (u32, u32) = (9, 0);

/// Consecutive passes an open item must be missing on the phone before Badger deletes it.
const MISSES_BEFORE_DELETE: i64 = 2;
/// Mass-delete guard: this many open items missing from one list in one pass, if they are
/// also more than half of its open items, pauses sync.
const MASS_MISSING_MIN: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassMode {
    Normal,
    /// The user confirmed "Delete in Badger anyway": while paused for an empty or mostly
    /// missing list (`empty_fetch`), skip the mass-delete guards once and apply phone deletes
    /// without waiting for a second missed pass. Otherwise an ordinary pass.
    DeleteAnyway,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Sync is off; nothing ran.
    Disabled,
    Synced,
    /// Held back by the mass-delete guard; nothing was written.
    Paused {
        kind: &'static str,
        message: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PassReport {
    pub outcome: Outcome,
    pub todos_changed: bool,
    pub reminders_changed: bool,
    pub phone_writes: usize,
    pub skipped: Vec<SkippedItem>,
    /// Per-item failures; the items are retried next pass.
    pub errors: Vec<String>,
}

impl PassReport {
    fn new() -> Self {
        PassReport {
            outcome: Outcome::Disabled,
            todos_changed: false,
            reminders_changed: false,
            phone_writes: 0,
            skipped: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Whether Badger rows changed (the runner tells the frontend).
    pub fn badger_changed(&self) -> bool {
        self.todos_changed || self.reminders_changed
    }

    fn changed(&mut self, kind: ListKind) {
        // Reminders show their todo's title and cascade with it.
        self.reminders_changed = true;
        if kind == ListKind::Todos {
            self.todos_changed = true;
        }
    }
}

/// Runs one pass and records its outcome in `app_settings`. Never fails: errors end up in
/// the report and the stored status.
pub async fn run_pass<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
    now: NaiveDateTime,
    mode: PassMode,
) -> PassReport {
    let mut report = PassReport::new();
    report.outcome = match pass(pool, store, now, mode, &mut report).await {
        Ok(outcome) => outcome,
        Err(e) => Outcome::Failed(e.to_string()),
    };
    if let Err(e) = record(pool, &report).await {
        tracing::error!("Apple Reminders sync: couldn't record the outcome: {}", e);
    }
    match &report.outcome {
        Outcome::Failed(e) => tracing::error!("Apple Reminders sync failed: {}", e),
        Outcome::Paused { message, .. } => {
            tracing::warn!("Apple Reminders sync paused: {}", message)
        }
        _ => {}
    }
    for e in &report.errors {
        tracing::error!("Apple Reminders sync: {}", e);
    }
    report
}

async fn record(pool: &Pool<Sqlite>, report: &PassReport) -> Result<(), AppError> {
    match &report.outcome {
        Outcome::Disabled => Ok(()),
        Outcome::Synced => {
            let error = match report.errors.as_slice() {
                [] => None,
                [one] => Some(one.clone()),
                [first, rest @ ..] => Some(format!(
                    "{} items couldn't sync: {} (and {} more)",
                    rest.len() + 1,
                    first,
                    rest.len()
                )),
            };
            let skipped = (!report.skipped.is_empty())
                .then(|| serde_json::to_string(&report.skipped).unwrap_or_default());
            let stamp = format_timestamp(crate::commands::reminders::local_now());
            set_apple_sync_outcome(
                pool,
                Some(&stamp),
                error.as_deref(),
                None,
                skipped.as_deref(),
            )
            .await?;
            if report.errors.is_empty() {
                let mut conn = pool.acquire().await?;
                set_apple_sync_restore_pending(&mut conn, false).await?;
            }
            Ok(())
        }
        Outcome::Paused { kind, message } => {
            set_apple_sync_outcome(pool, None, Some(message), Some(kind), None).await
        }
        Outcome::Failed(message) => {
            set_apple_sync_outcome(pool, None, Some(message), None, None).await
        }
    }
}

fn paused_access() -> Outcome {
    Outcome::Paused {
        kind: PAUSED_ACCESS,
        message: "Badger has no access to Reminders, so sync is paused and nothing is deleted. \
                  Grant access to continue."
            .to_string(),
    }
}

fn paused_list_missing(kind: ListKind) -> Outcome {
    Outcome::Paused {
        kind: PAUSED_LIST_MISSING,
        message: format!(
            "The “{}” list is missing in Reminders, so sync is paused and nothing is deleted. \
             Recreate the lists to continue.",
            kind.list_name()
        ),
    }
}

fn paused_empty(kind: ListKind, synced: usize) -> Outcome {
    let items = if synced == 1 { "item" } else { "items" };
    Outcome::Paused {
        kind: PAUSED_EMPTY_FETCH,
        message: format!(
            "The “{}” list came back empty although {synced} synced {items} should be in it, \
             so sync is paused and nothing is deleted in Badger.",
            kind.list_name()
        ),
    }
}

fn paused_missing(kind: ListKind, missing: usize, open: usize) -> Outcome {
    Outcome::Paused {
        kind: PAUSED_EMPTY_FETCH,
        message: format!(
            "{missing} of the {open} synced items in the “{}” list went missing at once, so \
             sync is paused and nothing is deleted in Badger.",
            kind.list_name()
        ),
    }
}

// ── Field mapping ─────────────────────────────────────────────────────────────

/// A Badger row of either kind.
#[derive(Debug, Clone)]
enum Row {
    Todo(BTodo),
    Reminder(BReminder),
}

impl Row {
    fn id(&self) -> i64 {
        match self {
            Row::Todo(t) => t.id,
            Row::Reminder(r) => r.id,
        }
    }

    fn snap(&self) -> Snap {
        match self {
            Row::Todo(t) => t.snap(),
            Row::Reminder(r) => r.snap(),
        }
    }

    /// Pushed when unpaired: open todos, open one-off reminders.
    fn in_scope(&self) -> bool {
        match self {
            Row::Todo(t) => t.in_scope(),
            Row::Reminder(r) => r.in_scope(),
        }
    }

    fn done(&self) -> bool {
        self.snap().done
    }

    /// A recurring reminder stays Mac-only (D7).
    fn recurring(&self) -> bool {
        matches!(self, Row::Reminder(r) if r.repeat != "none")
    }
}

async fn load_rows(kind: ListKind, pool: &Pool<Sqlite>) -> Result<HashMap<i64, Row>, AppError> {
    Ok(match kind {
        ListKind::Todos => load_todos(pool)
            .await?
            .into_iter()
            .map(|t| (t.id, Row::Todo(t)))
            .collect(),
        ListKind::Reminders => load_reminders(pool)
            .await?
            .into_iter()
            .map(|r| (r.id, Row::Reminder(r)))
            .collect(),
    })
}

async fn load_row(kind: ListKind, id: i64, pool: &Pool<Sqlite>) -> Result<Option<Row>, AppError> {
    Ok(match kind {
        ListKind::Todos => load_todo(id, pool).await?.map(Row::Todo),
        ListKind::Reminders => load_reminder(id, pool).await?.map(Row::Reminder),
    })
}

/// The local date-time a phone reminder's due stands for (date-only → 09:00).
fn reminder_time(due: PhoneDue) -> NaiveDateTime {
    match due {
        PhoneDue::DateTime(t) => t.with_nanosecond(0).unwrap_or(t),
        PhoneDue::Date(d) => d.and_time(
            NaiveTime::from_hms_opt(DATE_ONLY_TIME.0, DATE_ONLY_TIME.1, 0)
                .unwrap_or(NaiveTime::MIN),
        ),
    }
}

fn format_date(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

/// A phone item's synced fields in Badger terms (the phone side of the merge).
fn phone_snap(kind: ListKind, p: &PhoneItem) -> Snap {
    match kind {
        ListKind::Todos => Snap {
            title: p.title.trim().to_string(),
            notes: clean_notes(p.notes.as_deref()),
            due: p.due.map(|d| format_date(d.date())),
            done: p.completed,
        },
        ListKind::Reminders => Snap {
            title: clean_reminder_text(&p.title).unwrap_or_default(),
            // Reminder notes are Badger-owned (link line); the phone's aren't synced.
            notes: None,
            due: p.due.map(|d| format_timestamp(reminder_time(d))),
            done: p.completed,
        },
    }
}

/// The pushed due a native notification may defer to: a reminder's due date-time that has
/// an alarm at the same time and isn't completed.
fn phone_due(kind: ListKind, p: &PhoneItem) -> Option<String> {
    match (kind, p.due) {
        (ListKind::Reminders, Some(PhoneDue::DateTime(t))) if !p.completed => {
            let t = t.with_nanosecond(0).unwrap_or(t);
            p.alarms.contains(&t).then(|| format_timestamp(t))
        }
        _ => None,
    }
}

/// Per-field three-way merge. A field changed on one side only takes that side; changed on
/// both, Badger wins. Phone values Badger can't hold (empty title, a reminder without a
/// date) lose. A done reminder is final; after a restore Badger wins everything.
fn merge(kind: ListKind, b: &Snap, bs: &Snap, p: &Snap, ps: &Snap, authoritative: bool) -> Snap {
    if authoritative || (kind == ListKind::Reminders && b.done) {
        return b.clone();
    }
    let take =
        |b_changed: bool, p_changed: bool, phone_ok: bool| p_changed && !b_changed && phone_ok;
    let mut m = b.clone();
    if take(
        b.title != bs.title,
        p.title != ps.title,
        !p.title.is_empty(),
    ) {
        m.title = p.title.clone();
    }
    if kind == ListKind::Todos && take(b.notes != bs.notes, p.notes != ps.notes, true) {
        m.notes = p.notes.clone();
    }
    let due_ok = kind == ListKind::Todos || p.due.is_some();
    if take(b.due != bs.due, p.due != ps.due, due_ok) {
        m.due = p.due.clone();
    }
    if take(b.done != bs.done, p.done != ps.done, true) {
        m.done = p.done;
    }
    m
}

/// What the phone item should look like for this Badger row. `current` is the phone item
/// (none when creating), `synced` Badger's side at the last sync.
fn draft_for(row: &Row, current: Option<&PhoneItem>, synced: Option<&Snap>) -> PhoneDraft {
    match row {
        Row::Todo(t) => {
            let want = t
                .due_date
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
            // Same date: keep the phone's due as is (it may carry a time).
            let same = current.is_some_and(|p| p.due.map(PhoneDue::date) == want);
            PhoneDraft {
                title: t.title.clone(),
                notes: Some(t.notes.clone()),
                due: (!same).then_some(want.map(PhoneDue::Date)),
                alarm: None,
                completed: t.status == "done",
                strip_recurrence: false,
            }
        }
        Row::Reminder(r) => {
            let at = parse_timestamp(&r.remind_at)
                .ok()
                .map(|t| t.with_nanosecond(0).unwrap_or(t));
            let old_line = synced.and_then(|s| s.notes.as_deref());
            let notes = match (r.link_note(), current) {
                (Some(line), _) => Some(Some(line)),
                // Unlinked since the last sync: take Badger's line off the phone, unless the
                // notes were rewritten there (then they're the user's).
                (None, Some(p))
                    if old_line.is_some()
                        && clean_notes(p.notes.as_deref()).as_deref() == old_line =>
                {
                    Some(None)
                }
                // Unlinked: the phone's notes are left alone.
                (None, _) => None,
            };
            PhoneDraft {
                title: r.text.clone(),
                notes,
                due: Some(at.map(PhoneDue::DateTime)),
                alarm: at,
                completed: r.state == "done",
                strip_recurrence: current.is_some_and(|p| p.recurring),
            }
        }
    }
}

/// Whether writing `d` would change `p`.
fn needs_push(p: &PhoneItem, d: &PhoneDraft) -> bool {
    p.title.trim() != d.title
        || d.notes
            .as_ref()
            .is_some_and(|n| clean_notes(p.notes.as_deref()) != *n)
        || d.due.is_some_and(|due| p.due != due)
        || d.alarm.is_some_and(|t| p.alarms.as_slice() != [t])
        || p.completed != d.completed
        || (d.strip_recurrence && p.recurring)
}

fn pairing_state(kind: ListKind, row: &Row, p: &PhoneItem) -> PairingState {
    PairingState {
        ek_id: p.ek_id.clone(),
        external_id: p.external_id.clone(),
        badger_snap: row.snap(),
        phone_snap: phone_snap(kind, p),
        phone_due: phone_due(kind, p),
    }
}

/// The other Badger list.
fn other(kind: ListKind) -> ListKind {
    match kind {
        ListKind::Reminders => ListKind::Todos,
        ListKind::Todos => ListKind::Reminders,
    }
}

/// Whether `items` holds the pairing's phone item (by EventKit id, or external id).
fn holds(items: &[PhoneItem], p: &Pairing) -> bool {
    items
        .iter()
        .any(|i| i.ek_id == p.ek_id || (p.external_id.is_some() && i.external_id == p.external_id))
}

fn current_state(p: &Pairing) -> PairingState {
    PairingState {
        ek_id: p.ek_id.clone(),
        external_id: p.external_id.clone(),
        badger_snap: p.badger_snap.clone(),
        phone_snap: p.phone_snap.clone(),
        phone_due: p.phone_due.clone(),
    }
}

// ── The pass ──────────────────────────────────────────────────────────────────

struct Ctx<'a, S> {
    pool: &'a Pool<Sqlite>,
    store: &'a S,
    now: NaiveDateTime,
    authoritative: bool,
    /// list id and the items fetched at the start of the pass, per kind
    lists: HashMap<ListKind, (String, Vec<PhoneItem>)>,
    /// Phone items this pass deleted (they are still in `lists`).
    gone: HashSet<String>,
    /// "Delete in Badger anyway": phone deletes apply at once.
    delete_anyway: bool,
}

impl<S> Ctx<'_, S> {
    fn list_id(&self, kind: ListKind) -> &str {
        self.lists
            .get(&kind)
            .map(|(id, _)| id.as_str())
            .unwrap_or("")
    }

    fn items(&self, kind: ListKind) -> &[PhoneItem] {
        self.lists
            .get(&kind)
            .map(|(_, items)| items.as_slice())
            .unwrap_or(&[])
    }
}

async fn pass<S: ReminderStore>(
    pool: &Pool<Sqlite>,
    store: &S,
    now: NaiveDateTime,
    mode: PassMode,
    report: &mut PassReport,
) -> Result<Outcome, AppError> {
    let settings = get_apple_sync_settings(pool).await?;
    if !settings.enabled {
        return Ok(Outcome::Disabled);
    }
    if store.access().await != AccessState::Granted {
        return Ok(paused_access());
    }
    let pairings = load_pairings(pool).await?;

    let mut lists = HashMap::new();
    for kind in ListKind::ALL {
        let stored = match kind {
            ListKind::Reminders => settings.reminders_list_id.clone(),
            ListKind::Todos => settings.todos_list_id.clone(),
        };
        let list_id = match stored {
            Some(id) => id,
            None if pairings.iter().any(|p| p.kind == kind) => {
                return Ok(paused_list_missing(kind))
            }
            None => {
                let id = store.create_list(kind.list_name()).await?;
                set_apple_sync_list_id(pool, kind.settings_key(), Some(&id)).await?;
                id
            }
        };
        let items = match store.fetch(&list_id).await {
            Ok(items) => items,
            Err(StoreError::ListMissing) => return Ok(paused_list_missing(kind)),
            Err(StoreError::AccessDenied) => return Ok(paused_access()),
            Err(e) => return Err(e.into()),
        };
        lists.insert(kind, (list_id, items));
    }

    // "Delete in Badger anyway" overrides only the pause the user saw it on; a stale click
    // runs an ordinary pass.
    let delete_anyway =
        mode == PassMode::DeleteAnyway && settings.paused.as_deref() == Some(PAUSED_EMPTY_FETCH);

    // Mass-delete guard: an empty list while open items are paired to it, or most of the
    // Badger rows a miss would delete missing at once, is more likely an iCloud or account
    // problem than the user deleting everything. Items moved to the other list aren't
    // missing; rows deleted, done or recurring in Badger only unpair, so they don't count.
    if !delete_anyway {
        let items = |kind: ListKind| {
            lists
                .get(&kind)
                .map(|(_, items)| items.as_slice())
                .unwrap_or(&[])
        };
        for kind in ListKind::ALL {
            let open: Vec<&Pairing> = pairings
                .iter()
                .filter(|p| p.kind == kind && !p.phone_snap.done)
                .collect();
            if items(kind).is_empty() && !open.is_empty() {
                return Ok(paused_empty(kind, open.len()));
            }
            let rows = load_rows(kind, pool).await?;
            let deletable: Vec<&&Pairing> = open
                .iter()
                .filter(|p| {
                    rows.get(&p.badger_id)
                        .is_some_and(|r| !r.done() && !r.recurring())
                })
                .collect();
            let missing = deletable
                .iter()
                .filter(|p| !holds(items(kind), p) && !holds(items(other(kind)), p))
                .count();
            if missing >= MASS_MISSING_MIN && missing * 2 > deletable.len() {
                return Ok(paused_missing(kind, missing, deletable.len()));
            }
        }
    }

    let mut ctx = Ctx {
        pool,
        store,
        now,
        authoritative: settings.restore_pending,
        lists,
        gone: HashSet::new(),
        delete_anyway,
    };
    repair_by_external_id(&ctx, pairings).await?;
    for kind in ListKind::ALL {
        process_pairings(&mut ctx, kind, report).await?;
    }
    for kind in ListKind::ALL {
        push_unpaired(&ctx, kind, report).await?;
        import_unpaired(&ctx, kind, report).await?;
    }
    Ok(Outcome::Synced)
}

/// A pairing whose EventKit id no longer shows up is moved to the item in the same list
/// with its external id, when exactly one unclaimed item has it.
async fn repair_by_external_id<S>(
    ctx: &Ctx<'_, S>,
    pairings: Vec<Pairing>,
) -> Result<(), AppError> {
    let present: HashSet<&str> = ListKind::ALL
        .iter()
        .flat_map(|k| ctx.items(*k).iter().map(|i| i.ek_id.as_str()))
        .collect();
    let mut claimed: HashSet<String> = pairings.iter().map(|p| p.ek_id.clone()).collect();
    for p in &pairings {
        let Some(external) = p.external_id.as_deref() else {
            continue;
        };
        if present.contains(p.ek_id.as_str()) {
            continue;
        }
        let candidates: Vec<&PhoneItem> = ctx
            .items(p.kind)
            .iter()
            .filter(|i| i.external_id.as_deref() == Some(external) && !claimed.contains(&i.ek_id))
            .collect();
        if let [item] = candidates.as_slice() {
            set_pairing_ek_id(p.id, &item.ek_id, ctx.pool).await?;
            claimed.insert(item.ek_id.clone());
        }
    }
    Ok(())
}

async fn process_pairings<S: ReminderStore>(
    ctx: &mut Ctx<'_, S>,
    kind: ListKind,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let pairings: Vec<Pairing> = load_pairings(ctx.pool)
        .await?
        .into_iter()
        .filter(|p| p.kind == kind)
        .collect();
    let rows = load_rows(kind, ctx.pool).await?;
    let phone: HashMap<String, PhoneItem> = ctx
        .items(kind)
        .iter()
        .map(|i| (i.ek_id.clone(), i.clone()))
        .collect();

    for pair in &pairings {
        if pair.missed > 0 && phone.contains_key(&pair.ek_id) {
            // Back after a miss: the count starts over.
            if let Err(e) = set_pairing_missed(pair.id, 0, ctx.pool).await {
                report.errors.push(e.to_string());
            }
        }
        let result = match (rows.get(&pair.badger_id), phone.get(&pair.ek_id)) {
            (Some(row), Some(p)) if row.recurring() => {
                // Became recurring in Badger: it leaves the phone (D7).
                delete_on_phone(ctx, pair, p, report).await
            }
            (Some(row), Some(p)) => merge_pair(ctx, kind, pair, row, p, report).await,
            (Some(row), None) => phone_missing(ctx, kind, pair, row, report).await,
            (None, Some(p)) if ctx.authoritative => {
                // Not in the restored data: unpair, and the item is imported like a new one.
                let _ = p;
                delete_pairing(pair.id, ctx.pool).await
            }
            (None, Some(p)) => delete_on_phone(ctx, pair, p, report).await,
            (None, None) => delete_pairing(pair.id, ctx.pool).await,
        };
        if let Err(e) = result {
            report.errors.push(e.to_string());
        }
    }
    Ok(())
}

/// Deletes the phone item and its pairing (Badger deleted the row, or it became recurring).
async fn delete_on_phone<S: ReminderStore>(
    ctx: &mut Ctx<'_, S>,
    pair: &Pairing,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    ctx.store.delete_item(&p.ek_id).await?;
    report.phone_writes += 1;
    ctx.gone.insert(p.ek_id.clone());
    delete_pairing(pair.id, ctx.pool).await
}

/// The phone item is gone (deleted, cleared, or moved to another list).
///
/// An open item is deleted in Badger only once it has been missing for two passes in a row
/// (a one-pass iCloud glitch deletes nothing), at once when it moved to the other list or
/// the user confirmed "Delete in Badger anyway". If Badger edited the row since the last
/// sync, Badger wins (D14) and the item is put back on the phone.
async fn phone_missing<S: ReminderStore>(
    ctx: &mut Ctx<'_, S>,
    kind: ListKind,
    pair: &Pairing,
    row: &Row,
    report: &mut PassReport,
) -> Result<(), AppError> {
    if row.recurring() || row.done() || pair.phone_snap.done {
        // Completed items cleared on the phone, or rows that left the sync: only unpair.
        return delete_pairing(pair.id, ctx.pool).await;
    }
    if ctx.authoritative {
        // After a restore Badger is authoritative: put it back on the phone.
        return push_new(ctx, kind, row, Some(pair.id), report).await;
    }
    let moved = holds(ctx.items(other(kind)), pair);
    if !moved && !ctx.delete_anyway && pair.missed + 1 < MISSES_BEFORE_DELETE {
        return set_pairing_missed(pair.id, pair.missed + 1, ctx.pool).await;
    }
    if badger_edited(kind, &row.snap(), &pair.badger_snap) {
        return push_new(ctx, kind, row, Some(pair.id), report).await;
    }
    if delete_and_unpair(kind, row.id(), pair.id, &pair.badger_snap, ctx.pool).await? {
        report.changed(kind);
        return Ok(());
    }
    // Edited in Badger meanwhile: Badger wins.
    match load_row(kind, row.id(), ctx.pool).await? {
        Some(row) if !(row.recurring() || row.done()) => {
            push_new(ctx, kind, &row, Some(pair.id), report).await
        }
        _ => delete_pairing(pair.id, ctx.pool).await,
    }
}

/// Creates the phone item for `row` and records it in the pairing `pairing_id` (a new
/// pairing when `None`). If the pairing can't be recorded, the new phone item is deleted
/// again (best effort): unpaired, the next pass would import it as a duplicate.
async fn push_new<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    kind: ListKind,
    row: &Row,
    pairing_id: Option<i64>,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let item = ctx
        .store
        .create_item(ctx.list_id(kind), &draft_for(row, None, None))
        .await?;
    report.phone_writes += 1;
    let state = pairing_state(kind, row, &item);
    let recorded = match pairing_id {
        Some(id) => update_pairing(id, &state, ctx.pool).await,
        None => match ctx.pool.acquire().await {
            Ok(mut conn) => insert_pairing(kind, row.id(), &state, &mut conn)
                .await
                .map(|_| ()),
            Err(e) => Err(e.into()),
        },
    };
    if let Err(e) = recorded {
        match ctx.store.delete_item(&item.ek_id).await {
            Ok(()) => report.phone_writes += 1,
            Err(undo) => tracing::error!(
                "Apple Reminders sync: couldn't remove an unpaired item: {}",
                undo
            ),
        }
        return Err(e);
    }
    Ok(())
}

async fn merge_pair<S: ReminderStore>(
    ctx: &mut Ctx<'_, S>,
    kind: ListKind,
    pair: &Pairing,
    row: &Row,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let b_now = row.snap();
    let p_now = phone_snap(kind, p);
    let target = merge(
        kind,
        &b_now,
        &pair.badger_snap,
        &p_now,
        &pair.phone_snap,
        ctx.authoritative,
    );
    let row = if target != b_now {
        let applied = match row {
            Row::Todo(t) => apply_todo(t, &target, ctx.pool).await?,
            Row::Reminder(r) => apply_reminder(r, &target, ctx.now, ctx.pool).await?,
        };
        if !applied {
            // Edited meanwhile: next pass.
            return Ok(());
        }
        report.changed(kind);
        match load_row(kind, row.id(), ctx.pool).await? {
            Some(row) => row,
            None => return Ok(()),
        }
    } else {
        row.clone()
    };
    finish(ctx, kind, pair.id, &current_state(pair), &row, p, report).await
}

/// Brings the phone item in line with the Badger row and records the pairing.
async fn finish<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    kind: ListKind,
    pairing_id: i64,
    current: &PairingState,
    row: &Row,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let draft = draft_for(row, Some(p), Some(&current.badger_snap));
    let p_after = if needs_push(p, &draft) {
        let item = match ctx.store.update_item(&p.ek_id, &draft).await {
            Ok(item) => item,
            // Deleted on the phone meanwhile: next pass sees it.
            Err(StoreError::ItemMissing) => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        report.phone_writes += 1;
        item
    } else {
        p.clone()
    };
    let state = pairing_state(kind, row, &p_after);
    if state != *current {
        update_pairing(pairing_id, &state, ctx.pool).await?;
    }
    Ok(())
}

/// Creates phone items for unpaired Badger rows in scope.
async fn push_unpaired<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    kind: ListKind,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let paired: HashSet<i64> = load_pairings(ctx.pool)
        .await?
        .into_iter()
        .filter(|p| p.kind == kind)
        .map(|p| p.badger_id)
        .collect();
    let mut rows: Vec<Row> = load_rows(kind, ctx.pool).await?.into_values().collect();
    rows.sort_by_key(Row::id);
    for row in rows
        .iter()
        .filter(|r| r.in_scope() && !paired.contains(&r.id()))
    {
        if let Err(e) = push_new(ctx, kind, row, None, report).await {
            report.errors.push(e.to_string());
        }
    }
    Ok(())
}

/// Imports phone items nobody is paired with. Completed ones are left alone.
async fn import_unpaired<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    kind: ListKind,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let claimed: HashSet<String> = load_pairings(ctx.pool)
        .await?
        .into_iter()
        .map(|p| p.ek_id)
        .collect();
    for p in ctx.items(kind) {
        if p.completed || claimed.contains(&p.ek_id) || ctx.gone.contains(&p.ek_id) {
            continue;
        }
        if let Err(e) = import_one(ctx, kind, p, report).await {
            report.errors.push(e.to_string());
        }
    }
    Ok(())
}

fn skip(report: &mut PassReport, kind: ListKind, p: &PhoneItem, reason: &str) {
    let title = p.title.trim();
    report.skipped.push(SkippedItem {
        title: if title.is_empty() {
            "(no title)".to_string()
        } else {
            title.to_string()
        },
        list: kind.list_name().to_string(),
        reason: reason.to_string(),
    });
}

async fn import_one<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    kind: ListKind,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let snap = phone_snap(kind, p);
    if snap.title.is_empty() {
        skip(report, kind, p, "needs a title");
        return Ok(());
    }
    let initial = PairingState {
        ek_id: p.ek_id.clone(),
        external_id: p.external_id.clone(),
        badger_snap: Snap::default(),
        phone_snap: snap.clone(),
        phone_due: phone_due(kind, p),
    };
    let (id, pairing_id) = match kind {
        ListKind::Todos => {
            import_todo(
                &snap.title,
                snap.notes.as_deref(),
                snap.due.as_deref(),
                &initial,
                ctx.pool,
            )
            .await?
        }
        ListKind::Reminders => {
            let Some(due) = p.due else {
                skip(report, kind, p, "needs a date");
                return Ok(());
            };
            let at = resolve_gap(reminder_time(due), local_exists);
            import_reminder(&snap.title, at, ctx.now, &initial, ctx.pool).await?
        }
    };
    report.changed(kind);
    let Some(row) = load_row(kind, id, ctx.pool).await? else {
        return Ok(());
    };
    let mut current = initial;
    current.badger_snap = row.snap();
    // Phone form of the new row: a 09:00 time and alarm, no repeat rule, a cut title.
    finish(ctx, kind, pairing_id, &current, &row, p, report).await
}

#[cfg(test)]
mod tests;
