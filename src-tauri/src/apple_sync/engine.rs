//! One sync pass: load the tasks in scope (plus every paired one), fetch the "Badger" list,
//! merge each pair field by field against the last-synced snapshots, apply both sides, and
//! push or import what is unpaired.
//!
//! Synced fields: title, notes, due (date or date-time) and completed, merged three-way
//! (Badger wins a conflict). Alarms are Badger's: every pass sets the item's absolute alarms
//! to one per reminder (recomputed when the due moves, on either side), overwriting phone
//! edits. Repeating and Mac-only tasks stay off the phone; a completion younger than the
//! Undo window waits for a later pass.

use std::collections::{HashMap, HashSet};

use chrono::NaiveDateTime;
use sqlx::{Pool, Sqlite};

use crate::app_settings::{
    get_apple_sync_settings, set_apple_sync_list_id, set_apple_sync_outcome,
    set_apple_sync_restore_pending,
};
use crate::commands::tasks::clean_notes;
use crate::recurrence::{format_timestamp, local_now};
use crate::AppError;

use super::badger_writes::{
    apply_task, badger_edited, cut_title, delete_and_unpair, delete_pairing, import_task,
    insert_pairing, load_pairings, load_task, load_tasks, phone_due_key, set_pairing_ek_id,
    set_pairing_missed, to_phone_due, update_pairing, BTask, Pairing, PairingState, Snap,
};
use super::{
    alarm_key, list_name, AccessState, PhoneDraft, PhoneItem, ReminderStore, SkippedItem,
    StoreError, PAUSED_ACCESS, PAUSED_EMPTY_FETCH, PAUSED_LIST_MISSING,
};

/// Consecutive passes an open item must be missing on the phone before Badger deletes it.
const MISSES_BEFORE_DELETE: i64 = 2;
/// Mass-delete guard: this many open items missing from the list in one pass, if they are
/// also more than half of its open items, pauses sync.
const MASS_MISSING_MIN: usize = 3;
/// Open pairings the list must have before an empty fetch of it pauses sync.
const EMPTY_LIST_MIN: usize = 2;

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
    /// Badger tasks changed (the runner tells the frontend).
    pub tasks_changed: bool,
    pub phone_writes: usize,
    pub skipped: Vec<SkippedItem>,
    /// Per-item failures; the items are retried next pass.
    pub errors: Vec<String>,
}

impl PassReport {
    fn new() -> Self {
        PassReport {
            outcome: Outcome::Disabled,
            tasks_changed: false,
            phone_writes: 0,
            skipped: Vec::new(),
            errors: Vec::new(),
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
            let stamp = format_timestamp(local_now());
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

fn paused_list_missing() -> Outcome {
    Outcome::Paused {
        kind: PAUSED_LIST_MISSING,
        message: format!(
            "The “{}” list is missing in Reminders, so sync is paused and nothing is deleted. \
             Recreate the list to continue.",
            list_name()
        ),
    }
}

fn paused_empty(synced: usize) -> Outcome {
    let items = if synced == 1 { "item" } else { "items" };
    Outcome::Paused {
        kind: PAUSED_EMPTY_FETCH,
        message: format!(
            "The “{}” list came back empty although {synced} synced {items} should be in it, \
             so sync is paused and nothing is deleted in Badger.",
            list_name()
        ),
    }
}

fn paused_missing(missing: usize, open: usize) -> Outcome {
    Outcome::Paused {
        kind: PAUSED_EMPTY_FETCH,
        message: format!(
            "{missing} of the {open} synced items in the “{}” list went missing at once, so \
             sync is paused and nothing is deleted in Badger.",
            list_name()
        ),
    }
}

// ── Field mapping ─────────────────────────────────────────────────────────────

/// A phone item's synced fields in Badger terms (the phone side of the merge).
fn phone_snap(p: &PhoneItem) -> Snap {
    Snap {
        title: cut_title(&p.title).unwrap_or_default(),
        notes: clean_notes(p.notes.as_deref()),
        due: phone_due_key(p.due),
        done: p.completed,
    }
}

/// Per-field three-way merge. A field changed on one side only takes that side; changed on
/// both, Badger wins. An empty phone title loses. After a restore Badger wins everything.
fn merge(b: &Snap, bs: &Snap, p: &Snap, ps: &Snap, authoritative: bool) -> Snap {
    if authoritative {
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
    if take(b.notes != bs.notes, p.notes != ps.notes, true) {
        m.notes = p.notes.clone();
    }
    if take(b.due != bs.due, p.due != ps.due, true) {
        m.due = p.due.clone();
    }
    if take(b.done != bs.done, p.done != ps.done, true) {
        m.done = p.done;
    }
    m
}

/// What the phone item should look like for this task. `current` is the phone item (none
/// when creating).
fn draft_for(task: &BTask, current: Option<&PhoneItem>, now: NaiveDateTime) -> PhoneDraft {
    let on_phone = current.map(|p| p.alarms.as_slice()).unwrap_or(&[]);
    PhoneDraft {
        title: task.title.clone(),
        notes: Some(task.notes.clone()),
        due: Some(to_phone_due(task.snap().due.as_deref())),
        alarms: Some(task.wanted_alarms(on_phone, now)),
        completed: task.done,
        strip_recurrence: current.is_some_and(|p| p.recurring),
    }
}

fn alarm_keys(alarms: &[NaiveDateTime]) -> Vec<String> {
    let mut keys: Vec<String> = alarms.iter().map(|t| alarm_key(*t)).collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Whether writing `d` would change `p` (dues compared to the minute, alarms as a set).
fn needs_push(p: &PhoneItem, d: &PhoneDraft) -> bool {
    p.title.trim() != d.title
        || d.notes
            .as_ref()
            .is_some_and(|n| clean_notes(p.notes.as_deref()) != *n)
        || d.due
            .is_some_and(|due| phone_due_key(p.due) != phone_due_key(due))
        || d.alarms
            .as_ref()
            .is_some_and(|a| alarm_keys(&p.alarms) != alarm_keys(a))
        || p.completed != d.completed
        || (d.strip_recurrence && p.recurring)
}

fn pairing_state(task: &BTask, p: &PhoneItem) -> PairingState {
    PairingState {
        ek_id: p.ek_id.clone(),
        external_id: p.external_id.clone(),
        badger_snap: task.snap(),
        phone_snap: phone_snap(p),
        phone_alarms: alarm_keys(&p.alarms),
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
        phone_alarms: p.phone_alarms.clone(),
    }
}

// ── The pass ──────────────────────────────────────────────────────────────────

struct Ctx<'a, S> {
    pool: &'a Pool<Sqlite>,
    store: &'a S,
    now: NaiveDateTime,
    authoritative: bool,
    list_id: String,
    /// The items fetched at the start of the pass.
    items: Vec<PhoneItem>,
    /// Phone items this pass deleted (they are still in `items`).
    gone: HashSet<String>,
    /// "Delete in Badger anyway": phone deletes apply at once.
    delete_anyway: bool,
}

async fn load_rows(pool: &Pool<Sqlite>) -> Result<HashMap<i64, BTask>, AppError> {
    Ok(load_tasks(pool)
        .await?
        .into_iter()
        .map(|t| (t.id, t))
        .collect())
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

    let list_id = match settings.list_id.clone() {
        Some(id) => id,
        None if !pairings.is_empty() => return Ok(paused_list_missing()),
        None => {
            let id = store.create_list(list_name()).await?;
            set_apple_sync_list_id(pool, Some(&id)).await?;
            id
        }
    };
    let items = match store.fetch(&list_id).await {
        Ok(items) => items,
        Err(StoreError::ListMissing) => return Ok(paused_list_missing()),
        Err(StoreError::AccessDenied) => return Ok(paused_access()),
        Err(e) => return Err(e.into()),
    };

    // "Delete in Badger anyway" overrides only the pause the user saw it on; a stale click
    // runs an ordinary pass.
    let delete_anyway =
        mode == PassMode::DeleteAnyway && settings.paused.as_deref() == Some(PAUSED_EMPTY_FETCH);

    // Mass-delete guard: an empty list while open items are paired to it, or most of the
    // tasks a miss would delete missing at once, is more likely an iCloud or account
    // problem than the user deleting everything. Tasks deleted, done, repeating or Mac-only
    // in Badger only unpair, so they don't count (Mac-only ones not even toward an empty
    // list).
    if !delete_anyway {
        let rows = load_rows(pool).await?;
        let open: Vec<&Pairing> = pairings
            .iter()
            .filter(|p| !p.phone_snap.done)
            .filter(|p| !rows.get(&p.badger_id).is_some_and(|t| t.mac_only))
            .collect();
        // One open item can be the user clearing their last one; the two-pass rule still
        // covers a glitch then.
        if items.is_empty() && open.len() >= EMPTY_LIST_MIN {
            return Ok(paused_empty(open.len()));
        }
        let deletable: Vec<&&Pairing> = open
            .iter()
            .filter(|p| {
                rows.get(&p.badger_id)
                    .is_some_and(|t| !t.done && !t.off_phone())
            })
            .collect();
        let missing = deletable.iter().filter(|p| !holds(&items, p)).count();
        if missing >= MASS_MISSING_MIN && missing * 2 > deletable.len() {
            return Ok(paused_missing(missing, deletable.len()));
        }
    }

    let mut ctx = Ctx {
        pool,
        store,
        now,
        authoritative: settings.restore_pending,
        list_id,
        items,
        gone: HashSet::new(),
        delete_anyway,
    };
    repair_by_external_id(&ctx, pairings).await?;
    process_pairings(&mut ctx, report).await?;
    push_unpaired(&ctx, report).await?;
    import_unpaired(&ctx, report).await?;
    Ok(Outcome::Synced)
}

/// A pairing whose EventKit id no longer shows up is moved to the item in the list with its
/// external id, when exactly one unclaimed item has it.
async fn repair_by_external_id<S>(
    ctx: &Ctx<'_, S>,
    pairings: Vec<Pairing>,
) -> Result<(), AppError> {
    let present: HashSet<&str> = ctx.items.iter().map(|i| i.ek_id.as_str()).collect();
    let mut claimed: HashSet<String> = pairings.iter().map(|p| p.ek_id.clone()).collect();
    for p in &pairings {
        let Some(external) = p.external_id.as_deref() else {
            continue;
        };
        if present.contains(p.ek_id.as_str()) {
            continue;
        }
        let candidates: Vec<&PhoneItem> = ctx
            .items
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
    report: &mut PassReport,
) -> Result<(), AppError> {
    let pairings = load_pairings(ctx.pool).await?;
    let rows = load_rows(ctx.pool).await?;
    let phone: HashMap<String, PhoneItem> = ctx
        .items
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
        let row = rows.get(&pair.badger_id);
        if !pair.badger_snap.done && row.is_some_and(|t| t.in_undo_window(ctx.now)) {
            // Completed in Badger moments ago and not synced yet: Undo may still take it
            // back (D21). A completion that came from the phone is already in the snapshot.
            continue;
        }
        let result = match (row, phone.get(&pair.ek_id)) {
            (Some(task), Some(p)) if task.off_phone() => {
                // Became repeating (D6) or Mac-only in Badger: it leaves the phone; the task
                // stays.
                delete_on_phone(ctx, pair, p, report).await
            }
            (Some(task), Some(p)) => merge_pair(ctx, pair, task, p, report).await,
            (Some(task), None) => phone_missing(ctx, pair, task, report).await,
            (None, Some(_)) if ctx.authoritative => {
                // Not in the restored data: unpair, and the item is imported like a new one.
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

/// Deletes the phone item and its pairing (Badger deleted the task, or it became repeating
/// or Mac-only).
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
/// (a one-pass iCloud glitch deletes nothing), at once when the user confirmed "Delete in
/// Badger anyway". If Badger edited the task since the last sync, Badger wins and the item
/// is put back on the phone.
async fn phone_missing<S: ReminderStore>(
    ctx: &mut Ctx<'_, S>,
    pair: &Pairing,
    task: &BTask,
    report: &mut PassReport,
) -> Result<(), AppError> {
    if task.off_phone() || task.done || pair.phone_snap.done {
        // Completed items cleared on the phone, or tasks that left the sync (repeating,
        // Mac-only): only unpair. A Mac-only task is never deleted because of the phone.
        return delete_pairing(pair.id, ctx.pool).await;
    }
    if ctx.authoritative {
        // After a restore Badger is authoritative: put it back on the phone.
        return push_new(ctx, task, Some(pair.id), report).await;
    }
    if !ctx.delete_anyway && pair.missed + 1 < MISSES_BEFORE_DELETE {
        return set_pairing_missed(pair.id, pair.missed + 1, ctx.pool).await;
    }
    if badger_edited(&task.snap(), &pair.badger_snap) {
        return push_new(ctx, task, Some(pair.id), report).await;
    }
    if delete_and_unpair(task.id, pair.id, &pair.badger_snap, ctx.pool).await? {
        report.tasks_changed = true;
        return Ok(());
    }
    // Edited in Badger meanwhile: Badger wins.
    match load_task(task.id, ctx.pool).await? {
        Some(task) if !(task.off_phone() || task.done) => {
            push_new(ctx, &task, Some(pair.id), report).await
        }
        _ => delete_pairing(pair.id, ctx.pool).await,
    }
}

/// Creates the phone item for `task` and records it in the pairing `pairing_id` (a new
/// pairing when `None`). If the pairing can't be recorded, the new phone item is deleted
/// again (best effort): unpaired, the next pass would import it as a duplicate.
async fn push_new<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    task: &BTask,
    pairing_id: Option<i64>,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let item = ctx
        .store
        .create_item(&ctx.list_id, &draft_for(task, None, ctx.now))
        .await?;
    report.phone_writes += 1;
    let state = pairing_state(task, &item);
    let recorded = match pairing_id {
        Some(id) => update_pairing(id, &state, ctx.pool).await,
        None => match ctx.pool.acquire().await {
            Ok(mut conn) => insert_pairing(task.id, &state, &mut conn).await.map(|_| ()),
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
    pair: &Pairing,
    task: &BTask,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let b_now = task.snap();
    let target = merge(
        &b_now,
        &pair.badger_snap,
        &phone_snap(p),
        &pair.phone_snap,
        ctx.authoritative,
    );
    let task = if target != b_now {
        if !apply_task(task, &target, ctx.now, ctx.pool).await? {
            // Edited meanwhile: next pass.
            return Ok(());
        }
        report.tasks_changed = true;
        match load_task(task.id, ctx.pool).await? {
            Some(task) => task,
            None => return Ok(()),
        }
    } else {
        task.clone()
    };
    finish(ctx, pair.id, &current_state(pair), &task, p, report).await
}

/// Brings the phone item in line with the task (alarms recomputed) and records the pairing.
async fn finish<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    pairing_id: i64,
    current: &PairingState,
    task: &BTask,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let draft = draft_for(task, Some(p), ctx.now);
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
    let state = pairing_state(task, &p_after);
    if state != *current {
        update_pairing(pairing_id, &state, ctx.pool).await?;
    }
    Ok(())
}

/// Creates phone items for unpaired tasks in scope.
async fn push_unpaired<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let paired: HashSet<i64> = load_pairings(ctx.pool)
        .await?
        .into_iter()
        .map(|p| p.badger_id)
        .collect();
    let mut tasks = load_tasks(ctx.pool).await?;
    tasks.sort_by_key(|t| t.id);
    for task in tasks
        .iter()
        .filter(|t| t.in_scope() && !paired.contains(&t.id))
    {
        if let Err(e) = push_new(ctx, task, None, report).await {
            report.errors.push(e.to_string());
        }
    }
    Ok(())
}

/// Imports phone items nobody is paired with. Completed ones are left alone.
async fn import_unpaired<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let claimed: HashSet<String> = load_pairings(ctx.pool)
        .await?
        .into_iter()
        .map(|p| p.ek_id)
        .collect();
    for p in &ctx.items {
        if p.completed || claimed.contains(&p.ek_id) || ctx.gone.contains(&p.ek_id) {
            continue;
        }
        if let Err(e) = import_one(ctx, p, report).await {
            report.errors.push(e.to_string());
        }
    }
    Ok(())
}

fn skip(report: &mut PassReport, p: &PhoneItem, reason: &str) {
    let title = p.title.trim();
    report.skipped.push(SkippedItem {
        title: if title.is_empty() {
            "(no title)".to_string()
        } else {
            title.to_string()
        },
        list: list_name().to_string(),
        reason: reason.to_string(),
    });
}

/// A phone-created item becomes a task: date-only dues stay date-only, alarms become
/// reminders (D22), a repeat rule is stripped (it imports as a one-off).
async fn import_one<S: ReminderStore>(
    ctx: &Ctx<'_, S>,
    p: &PhoneItem,
    report: &mut PassReport,
) -> Result<(), AppError> {
    let snap = phone_snap(p);
    if snap.title.is_empty() {
        skip(report, p, "needs a title");
        return Ok(());
    }
    let initial = PairingState {
        ek_id: p.ek_id.clone(),
        external_id: p.external_id.clone(),
        badger_snap: Snap::default(),
        phone_snap: snap.clone(),
        phone_alarms: alarm_keys(&p.alarms),
    };
    let (id, pairing_id) = import_task(&snap, &p.alarms, &initial, ctx.now, ctx.pool).await?;
    report.tasks_changed = true;
    let Some(task) = load_task(id, ctx.pool).await? else {
        return Ok(());
    };
    let mut current = initial;
    current.badger_snap = task.snap();
    // Phone form of the new task: no repeat rule, a cut title, at most ten alarms.
    finish(ctx, pairing_id, &current, &task, p, report).await
}

#[cfg(test)]
mod tests;
