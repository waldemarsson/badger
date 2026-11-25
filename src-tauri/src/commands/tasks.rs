//! Tasks: open or done, an optional due date (and time), an optional repeat rule, links to
//! an issue and a note, and up to ten reminders.
//!
//! A reminder is relative to the due (`OffsetMinutes` before it; a date-only due counts from
//! 09:00) or at a custom time (`At`). Its fire time is derived, never stored, except as
//! `FiredFor`: the time its `State` (pending → sent → dismissed) applies to. Whenever an edit
//! moves a reminder's time, [`reconcile_reminders`] re-arms it (future) or dismisses it
//! silently (past), so editing a due date never notifies. Completing a task suppresses its
//! reminders without touching them; a repeating task instead moves on to its next
//! occurrence. Times are naive local wall-clock strings (see `crate::recurrence`).

use std::collections::HashMap;

use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};
use tauri::{AppHandle, State};

use super::issues::inner_resolve_issue;
use crate::recurrence::{
    format_timestamp, local_exists, local_now, month_day_column, next_occurrence, parse_timestamp,
    parse_weekdays, repeat_name, resolve_gap, weekdays_column, Rule,
};
use crate::AppError;

/// Maximum title length in characters (not bytes).
pub const MAX_TITLE_CHARS: usize = 500;
/// Most reminders one task can have.
pub const MAX_REMINDERS: usize = 10;
/// Where relative reminders count from on a date-only due.
pub const DATE_ONLY_ANCHOR: (u32, u32) = (9, 0);
/// Longest relative reminder: four weeks before the due.
pub const MAX_OFFSET_MINUTES: i64 = 4 * 7 * 24 * 60;
/// How long the frontend offers Undo after a completion. Sync doesn't push a completion
/// younger than this, so an undone completion never reaches the phone.
pub const UNDO_WINDOW_SECS: i64 = 5;

pub const STATE_PENDING: &str = "pending";
pub const STATE_SENT: &str = "sent";
pub const STATE_DISMISSED: &str = "dismissed";

const DATE_FORMAT: &str = "%Y-%m-%d";
const TIME_FORMAT: &str = "%H:%M";

// ── Rows ──────────────────────────────────────────────────────────────────────

/// A task with its links (LEFT JOINs: null when unlinked) and reminders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Task {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub completed_at: Option<String>,
    /// `YYYY-MM-DD`
    pub due_date: Option<String>,
    /// `HH:MM`; only set with a due date.
    pub due_time: Option<String>,
    /// `none|daily|weekdays|weekly|monthly`
    pub repeat: String,
    /// ISO weekdays (1 = Monday … 7 = Sunday); only set for weekly rules.
    pub weekdays: Vec<u32>,
    pub month_day: Option<i64>,
    pub issue_id: Option<i64>,
    pub issue_title: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub customer_id: Option<i64>,
    pub customer_name: Option<String>,
    pub note_id: Option<i64>,
    pub note_title: Option<String>,
    /// "Keep off phone": never synced to Apple Reminders. Repeating tasks never sync either.
    pub mac_only: bool,
    pub created_at: String,
    pub updated_at: String,
    /// By fire time (reminders without one last), then id.
    pub reminders: Vec<TaskReminder>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskReminder {
    pub id: i64,
    pub task_id: i64,
    /// Relative: minutes before the due (0 = at due time).
    pub offset_minutes: Option<i64>,
    /// Custom: a local timestamp.
    pub at: Option<String>,
    /// `pending|sent|dismissed`
    pub state: String,
    pub snoozed_until: Option<String>,
    /// When it fires (or fired): the snoozed time, else its due-relative or custom time.
    /// `None` for a relative reminder on a task without a due date ("Needs a due date").
    pub fire_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, FromRow)]
pub(crate) struct TaskRow {
    pub id: i64,
    pub title: String,
    pub notes: Option<String>,
    pub done: bool,
    pub completed_at: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub repeat: String,
    pub weekdays: String,
    pub month_day: Option<i64>,
    pub issue_id: Option<i64>,
    pub issue_title: Option<String>,
    pub project_id: Option<i64>,
    pub project_name: Option<String>,
    pub customer_id: Option<i64>,
    pub customer_name: Option<String>,
    pub note_id: Option<i64>,
    pub note_title: Option<String>,
    pub mac_only: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, FromRow, PartialEq)]
pub(crate) struct ReminderRow {
    pub id: i64,
    pub task_id: i64,
    pub offset_minutes: Option<i64>,
    pub at: Option<String>,
    pub state: String,
    pub snoozed_until: Option<String>,
    pub fired_for: Option<String>,
    pub created_at: String,
}

const SELECT_TASKS: &str = "SELECT t.id,
        t.Title       AS title,
        t.Notes       AS notes,
        t.Done        AS done,
        t.CompletedAt AS completed_at,
        t.DueDate     AS due_date,
        t.DueTime     AS due_time,
        t.Repeat      AS repeat,
        t.Weekdays    AS weekdays,
        t.MonthDay    AS month_day,
        i.id          AS issue_id,
        i.Title       AS issue_title,
        p.id          AS project_id,
        p.Name        AS project_name,
        c.id          AS customer_id,
        c.Name        AS customer_name,
        n.id          AS note_id,
        n.Title       AS note_title,
        t.MacOnly     AS mac_only,
        t.CreatedAt   AS created_at,
        t.UpdatedAt   AS updated_at
 FROM tasks t
 LEFT JOIN issues    i ON i.id = t.IssueId
 LEFT JOIN projects  p ON p.id = i.ProjectId
 LEFT JOIN customers c ON c.id = p.CustomerId
 LEFT JOIN notes     n ON n.id = t.NoteId";

const SELECT_REMINDERS: &str = "SELECT id,
        TaskId        AS task_id,
        OffsetMinutes AS offset_minutes,
        At            AS at,
        State         AS state,
        SnoozedUntil  AS snoozed_until,
        FiredFor      AS fired_for,
        CreatedAt     AS created_at
 FROM task_reminders";

/// What the create and edit forms send (camelCase over `invoke`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskInput {
    pub title: String,
    #[serde(default)]
    pub notes: Option<String>,
    /// `YYYY-MM-DD`
    #[serde(default)]
    pub due_date: Option<String>,
    /// `HH:MM`; needs a due date.
    #[serde(default)]
    pub due_time: Option<String>,
    #[serde(default = "default_repeat")]
    pub repeat: String,
    /// Weekly: ISO weekdays; empty means the due date's weekday.
    #[serde(default)]
    pub weekdays: Vec<i64>,
    /// Monthly: 1–31; missing means the due date's day.
    #[serde(default)]
    pub month_day: Option<i64>,
    #[serde(default)]
    pub issue_id: Option<i64>,
    #[serde(default)]
    pub note_id: Option<i64>,
    /// "Keep off phone". Create: missing = synced. Update: missing keeps the current value.
    #[serde(default)]
    pub mac_only: Option<bool>,
}

impl Default for TaskInput {
    fn default() -> Self {
        TaskInput {
            title: String::new(),
            notes: None,
            due_date: None,
            due_time: None,
            repeat: default_repeat(),
            weekdays: Vec::new(),
            month_day: None,
            issue_id: None,
            note_id: None,
            mac_only: None,
        }
    }
}

fn default_repeat() -> String {
    "none".to_string()
}

/// One reminder from the forms: exactly one of `offsetMinutes` (relative) or `at` (custom).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct TaskReminderInput {
    #[serde(default)]
    pub offset_minutes: Option<i64>,
    /// Local timestamp, e.g. `2026-11-09T09:00` or `2026-11-09T09:00:00.000`.
    #[serde(default)]
    pub at: Option<String>,
}

/// `complete_task`'s answer: the task as it is now, and what `undo_complete_task` needs to
/// put it back (`None` when the task was already done, so there is nothing to undo).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskCompletion {
    pub task: Task,
    pub undo: Option<TaskUndo>,
}

/// The pre-completion state of a task and its reminders. The frontend hands it back as is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskUndo {
    pub task_id: i64,
    /// `UpdatedAt` written by the completion: undo applies only while it's unchanged.
    pub completed_updated_at: String,
    /// What the completion left (`Done`, `DueDate`): undo also requires the task to still
    /// look like that, so the same undo never applies twice.
    pub completed_done: bool,
    pub completed_due_date: Option<String>,
    pub done: bool,
    pub completed_at: Option<String>,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
    pub reminders: Vec<ReminderUndo>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReminderUndo {
    pub id: i64,
    pub state: String,
    pub snoozed_until: Option<String>,
    pub fired_for: Option<String>,
}

// ── Fire time math ────────────────────────────────────────────────────────────

/// The time relative reminders count back from: the due date at its time, or at 09:00 for
/// a date-only due; moved out of a DST gap. `None` without a (valid) due date.
pub fn due_anchor(
    due_date: Option<&str>,
    due_time: Option<&str>,
    exists: impl Fn(NaiveDateTime) -> bool,
) -> Option<NaiveDateTime> {
    let date = NaiveDate::parse_from_str(due_date?, DATE_FORMAT).ok()?;
    let time = match due_time {
        Some(t) => NaiveTime::parse_from_str(t, TIME_FORMAT).ok()?,
        None => NaiveTime::from_hms_opt(DATE_ONLY_ANCHOR.0, DATE_ONLY_ANCHOR.1, 0)?,
    };
    Some(resolve_gap(date.and_time(time), exists))
}

/// A reminder's own fire time (snooze aside): `anchor` − offset, or its custom time. `None`
/// for a relative reminder without a due, or a value that doesn't parse.
pub fn base_fire_time(
    anchor: Option<NaiveDateTime>,
    offset_minutes: Option<i64>,
    at: Option<&str>,
    exists: impl Fn(NaiveDateTime) -> bool,
) -> Option<NaiveDateTime> {
    match (offset_minutes, at) {
        (Some(offset), _) => {
            let t = anchor?.checked_sub_signed(Duration::minutes(offset))?;
            Some(resolve_gap(t, exists))
        }
        (None, Some(at)) => parse_timestamp(at).ok(),
        (None, None) => None,
    }
}

fn row_base_time(task_due: (Option<&str>, Option<&str>), r: &ReminderRow) -> Option<NaiveDateTime> {
    let anchor = due_anchor(task_due.0, task_due.1, local_exists);
    base_fire_time(anchor, r.offset_minutes, r.at.as_deref(), local_exists)
}

/// When `r` fires next (or fired): its snooze, else its base time.
fn effective_fire_time(base: Option<NaiveDateTime>, r: &ReminderRow) -> Option<NaiveDateTime> {
    r.snoozed_until
        .as_deref()
        .and_then(|s| parse_timestamp(s).ok())
        .or(base)
}

/// The state a reminder takes when its base time becomes `base` (D20): pending for a
/// future time (or none: dormant until the task gets a due date), dismissed for a past one.
fn armed_state(base: Option<NaiveDateTime>, now: NaiveDateTime) -> &'static str {
    match base {
        Some(t) if t <= now => STATE_DISMISSED,
        _ => STATE_PENDING,
    }
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn custom(msg: &str) -> AppError {
    AppError::Custom(msg.to_string())
}

fn not_found() -> AppError {
    custom("Task not found")
}

fn reminder_not_found() -> AppError {
    custom("Reminder not found")
}

/// Trimmed title, 1–500 characters.
pub(crate) fn clean_title(title: &str) -> Result<String, AppError> {
    let t = title.trim();
    if t.is_empty() {
        return Err(custom("Title is required"));
    }
    if t.chars().count() > MAX_TITLE_CHARS {
        return Err(custom("Title is too long (max 500 characters)"));
    }
    Ok(t.to_string())
}

pub(crate) fn clean_notes(notes: Option<&str>) -> Option<String> {
    notes
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

/// A due date (`YYYY-MM-DD`) and time (`HH:MM`, seconds dropped); blanks are `None`. A time
/// needs a date.
pub(crate) fn clean_due(
    due_date: Option<&str>,
    due_time: Option<&str>,
) -> Result<(Option<String>, Option<String>), AppError> {
    let date = match due_date.map(str::trim).filter(|d| !d.is_empty()) {
        None => None,
        Some(d) => Some(
            NaiveDate::parse_from_str(d, DATE_FORMAT)
                .map_err(|_| custom("Invalid due date"))?
                .format(DATE_FORMAT)
                .to_string(),
        ),
    };
    let time = match due_time.map(str::trim).filter(|t| !t.is_empty()) {
        None => None,
        Some(t) => Some(
            NaiveTime::parse_from_str(t, TIME_FORMAT)
                .or_else(|_| NaiveTime::parse_from_str(t, "%H:%M:%S"))
                .map_err(|_| custom("Invalid due time"))?
                .format(TIME_FORMAT)
                .to_string(),
        ),
    };
    if time.is_some() && date.is_none() {
        return Err(custom("A due time needs a due date"));
    }
    Ok((date, time))
}

/// The repeat rule; weekly without days repeats on the due date's weekday, monthly without a
/// day on the due date's day. A rule needs a due date.
fn clean_rule(input: &TaskInput, due_date: Option<&str>) -> Result<Option<Rule>, AppError> {
    if input.repeat == "none" {
        return Ok(None);
    }
    let Some(due) = due_date.and_then(|d| NaiveDate::parse_from_str(d, DATE_FORMAT).ok()) else {
        return Err(custom("A repeating task needs a due date"));
    };
    use chrono::Datelike;
    let weekdays = if input.repeat == "weekly" && input.weekdays.is_empty() {
        vec![i64::from(due.weekday().number_from_monday())]
    } else {
        input.weekdays.clone()
    };
    let month_day = match (input.repeat.as_str(), input.month_day) {
        ("monthly", None) => Some(i64::from(due.day())),
        (_, day) => day,
    };
    Rule::from_input(&input.repeat, &weekdays, month_day)
}

fn task_rule(row: &TaskRow) -> Result<Option<Rule>, AppError> {
    Rule::from_columns(&row.repeat, &row.weekdays, row.month_day)
}

async fn ensure_links(
    issue_id: Option<i64>,
    note_id: Option<i64>,
    conn: &mut SqliteConnection,
) -> Result<(), AppError> {
    if let Some(id) = issue_id {
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM issues WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
        if found.is_none() {
            return Err(custom("Issue not found"));
        }
    }
    if let Some(id) = note_id {
        let found: Option<i64> = sqlx::query_scalar("SELECT id FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut *conn)
            .await?;
        if found.is_none() {
            return Err(custom("Note not found"));
        }
    }
    Ok(())
}

/// A validated reminder: an offset, or a custom time (resolved out of a DST gap) that must
/// be in the future.
fn clean_reminder(
    input: &TaskReminderInput,
    now: NaiveDateTime,
) -> Result<(Option<i64>, Option<String>), AppError> {
    let at = input.at.as_deref().map(str::trim).filter(|a| !a.is_empty());
    match (input.offset_minutes, at) {
        (Some(offset), None) => {
            if !(0..=MAX_OFFSET_MINUTES).contains(&offset) {
                return Err(custom("Invalid reminder"));
            }
            Ok((Some(offset), None))
        }
        (None, Some(at)) => {
            let t = resolve_gap(parse_timestamp(at)?, local_exists);
            if t <= now {
                return Err(custom("Time is in the past"));
            }
            Ok((None, Some(format_timestamp(t))))
        }
        _ => Err(custom("Pick when to remind")),
    }
}

fn stamp() -> String {
    format_timestamp(local_now())
}

/// A stamp strictly after `prev` (millisecond precision, as stored): now, or `prev` + 1 ms
/// when the clock hasn't moved past it. Compare-and-set guards on `UpdatedAt` rely on it.
fn stamp_after(prev: &str) -> String {
    use chrono::SubsecRound;
    let now = local_now().trunc_subsecs(3);
    match parse_timestamp(prev) {
        Ok(p) if p >= now => format_timestamp(p + Duration::milliseconds(1)),
        _ => format_timestamp(now),
    }
}

fn valid_date(s: &str) -> bool {
    NaiveDate::parse_from_str(s, DATE_FORMAT).is_ok()
}

fn valid_time_of_day(s: &str) -> bool {
    NaiveTime::parse_from_str(s, TIME_FORMAT).is_ok()
}

fn valid_timestamp(s: &str) -> bool {
    parse_timestamp(s).is_ok()
}

/// [`TaskUndo`] comes back from the frontend: everything it would write must be well-formed.
fn check_undo(undo: &TaskUndo) -> Result<(), AppError> {
    let opt = |v: &Option<String>, ok: fn(&str) -> bool| v.as_deref().map_or(true, ok);
    let valid = opt(&undo.due_date, valid_date)
        && opt(&undo.due_time, valid_time_of_day)
        && (undo.due_time.is_none() || undo.due_date.is_some())
        && opt(&undo.completed_at, valid_timestamp)
        && undo.done == undo.completed_at.is_some()
        && opt(&undo.completed_due_date, valid_date)
        && undo.reminders.iter().all(|r| {
            [STATE_PENDING, STATE_SENT, STATE_DISMISSED].contains(&r.state.as_str())
                && opt(&r.fired_for, valid_timestamp)
                && opt(&r.snoozed_until, valid_timestamp)
        });
    if valid {
        Ok(())
    } else {
        Err(custom("This undo is no longer valid"))
    }
}

// ── Loading ───────────────────────────────────────────────────────────────────

pub(crate) async fn load_task_row(
    id: i64,
    conn: &mut SqliteConnection,
) -> Result<Option<TaskRow>, AppError> {
    let sql = format!("{SELECT_TASKS} WHERE t.id = ?");
    Ok(sqlx::query_as::<_, TaskRow>(sqlx::AssertSqlSafe(sql))
        .bind(id)
        .fetch_optional(conn)
        .await?)
}

pub(crate) async fn load_reminder_rows(
    task_id: i64,
    conn: &mut SqliteConnection,
) -> Result<Vec<ReminderRow>, AppError> {
    let sql = format!("{SELECT_REMINDERS} WHERE TaskId = ? ORDER BY id");
    Ok(sqlx::query_as::<_, ReminderRow>(sqlx::AssertSqlSafe(sql))
        .bind(task_id)
        .fetch_all(conn)
        .await?)
}

fn to_reminder(task: &TaskRow, r: &ReminderRow) -> TaskReminder {
    let base = row_base_time((task.due_date.as_deref(), task.due_time.as_deref()), r);
    TaskReminder {
        id: r.id,
        task_id: r.task_id,
        offset_minutes: r.offset_minutes,
        at: r.at.clone(),
        state: r.state.clone(),
        snoozed_until: r.snoozed_until.clone(),
        fire_at: effective_fire_time(base, r).map(format_timestamp),
        created_at: r.created_at.clone(),
    }
}

fn to_task(row: TaskRow, reminders: &[ReminderRow]) -> Task {
    let mut list: Vec<TaskReminder> = reminders.iter().map(|r| to_reminder(&row, r)).collect();
    list.sort_by(|a, b| {
        (a.fire_at.is_none(), &a.fire_at, a.id).cmp(&(b.fire_at.is_none(), &b.fire_at, b.id))
    });
    Task {
        id: row.id,
        title: row.title,
        notes: row.notes,
        done: row.done,
        completed_at: row.completed_at,
        due_date: row.due_date,
        due_time: row.due_time,
        repeat: row.repeat,
        weekdays: parse_weekdays(&row.weekdays),
        month_day: row.month_day,
        issue_id: row.issue_id,
        issue_title: row.issue_title,
        project_id: row.project_id,
        project_name: row.project_name,
        customer_id: row.customer_id,
        customer_name: row.customer_name,
        note_id: row.note_id,
        note_title: row.note_title,
        mac_only: row.mac_only,
        created_at: row.created_at,
        updated_at: row.updated_at,
        reminders: list,
    }
}

async fn get_task_in(id: i64, conn: &mut SqliteConnection) -> Result<Task, AppError> {
    let row = load_task_row(id, &mut *conn).await?.ok_or_else(not_found)?;
    let reminders = load_reminder_rows(id, conn).await?;
    Ok(to_task(row, &reminders))
}

/// Brings every reminder of `task_id` in line with its current fire time (D20): one whose
/// base time differs from `FiredFor` is re-armed (pending, snooze dropped) when the time is
/// in the future or missing, and dismissed silently when it has passed. Reminders whose time
/// didn't move are left alone. Runs inside the caller's transaction after any write that
/// can move a fire time (due, offset, custom time; sync's due writes too).
pub(crate) async fn reconcile_reminders(
    task_id: i64,
    now: NaiveDateTime,
    conn: &mut SqliteConnection,
) -> Result<(), AppError> {
    let Some(task) = load_task_row(task_id, &mut *conn).await? else {
        return Ok(());
    };
    for r in load_reminder_rows(task_id, &mut *conn).await? {
        let base = row_base_time((task.due_date.as_deref(), task.due_time.as_deref()), &r);
        let key = base.map(format_timestamp);
        if key == r.fired_for {
            continue;
        }
        sqlx::query(
            "UPDATE task_reminders SET State = ?, SnoozedUntil = NULL, FiredFor = ? WHERE id = ?",
        )
        .bind(armed_state(base, now))
        .bind(&key)
        .bind(r.id)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_reminder(
    task_id: i64,
    (offset, at): (Option<i64>, Option<String>),
    conn: &mut SqliteConnection,
) -> Result<i64, AppError> {
    // FiredFor stays NULL: reconcile arms it for the task's current due.
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO task_reminders (TaskId, OffsetMinutes, At, State, CreatedAt) \
         VALUES (?, ?, ?, 'pending', ?) RETURNING id",
    )
    .bind(task_id)
    .bind(offset)
    .bind(at)
    .bind(stamp())
    .fetch_one(conn)
    .await?;
    Ok(id)
}

async fn reminder_count(task_id: i64, conn: &mut SqliteConnection) -> Result<usize, AppError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM task_reminders WHERE TaskId = ?")
        .bind(task_id)
        .fetch_one(conn)
        .await?;
    Ok(usize::try_from(n).unwrap_or(usize::MAX))
}

/// The task id of reminder `id`.
/// Moves a task's `UpdatedAt` forward for a user's reminder write, so an undo snapshot taken
/// before it is refused rather than overwriting the edit. The scheduler's `mark_sent` never
/// calls it: firing must not invalidate undo (and its own CAS reads `UpdatedAt` unchanged).
async fn touch_task(task_id: i64, conn: &mut SqliteConnection) -> Result<(), AppError> {
    let prev: String = sqlx::query_scalar("SELECT UpdatedAt FROM tasks WHERE id = ?")
        .bind(task_id)
        .fetch_optional(&mut *conn)
        .await?
        .ok_or_else(not_found)?;
    sqlx::query("UPDATE tasks SET UpdatedAt = ? WHERE id = ?")
        .bind(stamp_after(&prev))
        .bind(task_id)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn task_of_reminder(id: i64, conn: &mut SqliteConnection) -> Result<i64, AppError> {
    sqlx::query_scalar("SELECT TaskId FROM task_reminders WHERE id = ?")
        .bind(id)
        .fetch_optional(conn)
        .await?
        .ok_or_else(reminder_not_found)
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

pub async fn inner_get_task(id: i64, pool: &Pool<Sqlite>) -> Result<Task, AppError> {
    let mut conn = pool.acquire().await?;
    get_task_in(id, &mut conn).await
}

/// Open tasks by due date, due time (untimed last), created; then done tasks, most recently
/// completed first (all of them, or the latest `done_limit`).
pub async fn inner_list_tasks(
    done_limit: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<Vec<Task>, AppError> {
    let open = format!(
        "{SELECT_TASKS} WHERE t.Done = 0 \
         ORDER BY t.DueDate IS NULL, t.DueDate, t.DueTime IS NULL, t.DueTime, t.CreatedAt, t.id"
    );
    let done =
        format!("{SELECT_TASKS} WHERE t.Done = 1 ORDER BY t.CompletedAt DESC, t.id DESC LIMIT ?");
    let mut rows = sqlx::query_as::<_, TaskRow>(sqlx::AssertSqlSafe(open))
        .fetch_all(pool)
        .await?;
    rows.extend(
        sqlx::query_as::<_, TaskRow>(sqlx::AssertSqlSafe(done))
            .bind(done_limit.unwrap_or(-1))
            .fetch_all(pool)
            .await?,
    );
    let sql = format!("{SELECT_REMINDERS} ORDER BY TaskId, id");
    let mut by_task: HashMap<i64, Vec<ReminderRow>> = HashMap::new();
    for r in sqlx::query_as::<_, ReminderRow>(sqlx::AssertSqlSafe(sql))
        .fetch_all(pool)
        .await?
    {
        by_task.entry(r.task_id).or_default().push(r);
    }
    Ok(rows
        .into_iter()
        .map(|row| {
            let reminders = by_task.remove(&row.id).unwrap_or_default();
            to_task(row, &reminders)
        })
        .collect())
}

/// Creates an open task with its first reminders (at most ten).
pub async fn inner_create_task(
    input: &TaskInput,
    reminders: &[TaskReminderInput],
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let title = clean_title(&input.title)?;
    let (due_date, due_time) = clean_due(input.due_date.as_deref(), input.due_time.as_deref())?;
    let rule = clean_rule(input, due_date.as_deref())?;
    if reminders.len() > MAX_REMINDERS {
        return Err(custom("A task can have at most 10 reminders"));
    }
    let reminders = reminders
        .iter()
        .map(|r| clean_reminder(r, now))
        .collect::<Result<Vec<_>, _>>()?;

    let mut tx = pool.begin().await?;
    ensure_links(input.issue_id, input.note_id, &mut tx).await?;
    let ts = stamp();
    let rule = rule.as_ref();
    let (id,): (i64,) = sqlx::query_as(
        "INSERT INTO tasks (Title, Notes, Done, DueDate, DueTime, Repeat, Weekdays, MonthDay, \
             IssueId, NoteId, MacOnly, CreatedAt, UpdatedAt) \
         VALUES (?, ?, 0, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&title)
    .bind(clean_notes(input.notes.as_deref()))
    .bind(&due_date)
    .bind(&due_time)
    .bind(repeat_name(rule))
    .bind(weekdays_column(rule))
    .bind(month_day_column(rule))
    .bind(input.issue_id)
    .bind(input.note_id)
    .bind(input.mac_only.unwrap_or(false))
    .bind(&ts)
    .bind(&ts)
    .fetch_one(&mut *tx)
    .await?;
    for r in reminders {
        insert_reminder(id, r, &mut tx).await?;
    }
    reconcile_reminders(id, now, &mut tx).await?;
    let task = get_task_in(id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Replaces title, notes, due, repeat and links (`None` unlinks); "Keep off phone" only when
/// given. Reminders whose time moved re-arm or are dismissed silently (D20).
pub async fn inner_update_task(
    id: i64,
    input: &TaskInput,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let title = clean_title(&input.title)?;
    let (due_date, due_time) = clean_due(input.due_date.as_deref(), input.due_time.as_deref())?;
    let rule = clean_rule(input, due_date.as_deref())?;

    let mut tx = pool.begin().await?;
    ensure_links(input.issue_id, input.note_id, &mut tx).await?;
    let rule = rule.as_ref();
    let result = sqlx::query(
        "UPDATE tasks SET Title = ?, Notes = ?, DueDate = ?, DueTime = ?, Repeat = ?, \
             Weekdays = ?, MonthDay = ?, IssueId = ?, NoteId = ?, \
             MacOnly = COALESCE(?, MacOnly), UpdatedAt = ? \
         WHERE id = ?",
    )
    .bind(&title)
    .bind(clean_notes(input.notes.as_deref()))
    .bind(&due_date)
    .bind(&due_time)
    .bind(repeat_name(rule))
    .bind(weekdays_column(rule))
    .bind(month_day_column(rule))
    .bind(input.issue_id)
    .bind(input.note_id)
    .bind(input.mac_only)
    .bind(stamp())
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    reconcile_reminders(id, now, &mut tx).await?;
    let task = get_task_in(id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Done: a one-off task becomes done (its reminders are kept but suppressed); a repeating
/// one moves its due to the next occurrence after max(due, now) and stays open, relative
/// reminders re-arming for it; custom ones fire once, so a sent one is dismissed (its toast
/// goes, it stops counting) and pending ones stay as they are. Returns the snapshot Undo
/// restores; an already done task comes back unchanged with no undo.
pub async fn inner_complete_task(
    id: i64,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<TaskCompletion, AppError> {
    let mut tx = pool.begin().await?;
    let row = load_task_row(id, &mut tx).await?.ok_or_else(not_found)?;
    if row.done {
        let task = get_task_in(id, &mut tx).await?;
        return Ok(TaskCompletion { task, undo: None });
    }
    let before = load_reminder_rows(id, &mut tx).await?;
    let ts = stamp_after(&row.updated_at);
    match task_rule(&row)? {
        None => {
            sqlx::query(
                "UPDATE tasks SET Done = 1, CompletedAt = ?, UpdatedAt = ? WHERE id = ? AND Done = 0",
            )
            .bind(format_timestamp(now))
            .bind(&ts)
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
        Some(rule) => {
            let next = next_due(&row, &rule, now)
                .ok_or_else(|| custom("Couldn't find the next occurrence"))?;
            sqlx::query("UPDATE tasks SET DueDate = ?, UpdatedAt = ? WHERE id = ?")
                .bind(next.format(DATE_FORMAT).to_string())
                .bind(&ts)
                .bind(id)
                .execute(&mut *tx)
                .await?;
            reconcile_reminders(id, now, &mut tx).await?;
            sqlx::query(
                "UPDATE task_reminders SET State = 'dismissed', SnoozedUntil = NULL \
                 WHERE TaskId = ? AND At IS NOT NULL AND State = 'sent'",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
        }
    }
    let task = get_task_in(id, &mut tx).await?;
    tx.commit().await?;
    let undo = TaskUndo {
        task_id: id,
        completed_updated_at: ts,
        completed_done: task.done,
        completed_due_date: task.due_date.clone(),
        done: row.done,
        completed_at: row.completed_at,
        due_date: row.due_date,
        due_time: row.due_time,
        reminders: before
            .into_iter()
            .map(|r| ReminderUndo {
                id: r.id,
                state: r.state,
                snoozed_until: r.snoozed_until,
                fired_for: r.fired_for,
            })
            .collect(),
    };
    Ok(TaskCompletion {
        task,
        undo: Some(undo),
    })
}

/// The date a repeating task moves to when completed at `now`: the first occurrence strictly
/// after max(its due, now), at its due time (09:00 for a date-only due).
fn next_due(row: &TaskRow, rule: &Rule, now: NaiveDateTime) -> Option<NaiveDate> {
    let anchor = due_anchor(row.due_date.as_deref(), row.due_time.as_deref(), |_| true)?;
    next_occurrence(rule, anchor.max(now), anchor.time(), local_exists).map(|t| t.date())
}

/// Puts a task back exactly as [`TaskUndo`] recorded it (done flag, due, reminder states and
/// snoozes), as long as nothing changed it since the completion: `UpdatedAt` and the done
/// flag and due the completion left must all still match. Undo writes a later `UpdatedAt`,
/// so the same undo can't apply twice.
pub async fn inner_undo_complete_task(
    undo: &TaskUndo,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    check_undo(undo)?;
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "UPDATE tasks SET Done = ?, CompletedAt = ?, DueDate = ?, DueTime = ?, UpdatedAt = ? \
         WHERE id = ? AND UpdatedAt = ? AND Done = ? AND DueDate IS ?",
    )
    .bind(undo.done)
    .bind(&undo.completed_at)
    .bind(&undo.due_date)
    .bind(&undo.due_time)
    .bind(stamp_after(&undo.completed_updated_at))
    .bind(undo.task_id)
    .bind(&undo.completed_updated_at)
    .bind(undo.completed_done)
    .bind(&undo.completed_due_date)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        load_task_row(undo.task_id, &mut tx)
            .await?
            .ok_or_else(not_found)?;
        return Err(custom("The task changed meanwhile, so it can't be undone"));
    }
    for r in &undo.reminders {
        sqlx::query(
            "UPDATE task_reminders SET State = ?, SnoozedUntil = ?, FiredFor = ? \
             WHERE id = ? AND TaskId = ?",
        )
        .bind(&r.state)
        .bind(&r.snoozed_until)
        .bind(&r.fired_for)
        .bind(r.id)
        .bind(undo.task_id)
        .execute(&mut *tx)
        .await?;
    }
    let task = get_task_in(undo.task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Reopens a done task. Reminders whose time has passed are dismissed silently, so nothing
/// already in the past is sent again (D8).
pub async fn inner_reopen_task(
    id: i64,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let mut tx = pool.begin().await?;
    reopen_in(id, now, &mut tx).await?;
    let task = get_task_in(id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// [`inner_reopen_task`]'s writes, on the caller's transaction (sync reuses them). A task that
/// is already open is left alone.
pub(crate) async fn reopen_in(
    id: i64,
    now: NaiveDateTime,
    conn: &mut SqliteConnection,
) -> Result<(), AppError> {
    let row = load_task_row(id, &mut *conn).await?.ok_or_else(not_found)?;
    if !row.done {
        return Ok(());
    }
    sqlx::query("UPDATE tasks SET Done = 0, CompletedAt = NULL, UpdatedAt = ? WHERE id = ?")
        .bind(stamp())
        .bind(id)
        .execute(&mut *conn)
        .await?;
    for r in load_reminder_rows(id, &mut *conn).await? {
        if r.state == STATE_DISMISSED {
            continue;
        }
        let base = row_base_time((row.due_date.as_deref(), row.due_time.as_deref()), &r);
        if effective_fire_time(base, &r).is_some_and(|t| t <= now) {
            sqlx::query(
                "UPDATE task_reminders SET State = 'dismissed', SnoozedUntil = NULL WHERE id = ?",
            )
            .bind(r.id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// [`inner_complete_task`] for a one-off on the caller's transaction (sync applies a phone
/// completion with it). Repeating tasks never sync, so they aren't handled here.
pub(crate) async fn complete_in(
    id: i64,
    now: NaiveDateTime,
    conn: &mut SqliteConnection,
) -> Result<(), AppError> {
    sqlx::query(
        "UPDATE tasks SET Done = 1, CompletedAt = ?, UpdatedAt = ? WHERE id = ? AND Done = 0",
    )
    .bind(format_timestamp(now))
    .bind(stamp())
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Deletes a task and (by cascade) its reminders. Its phone item goes on the next sync pass.
pub async fn inner_delete_task(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM tasks WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(())
}

/// Creates (or reuses a same-title) issue in `project_id` and links the task.
pub async fn inner_create_issue_for_task(
    task_id: i64,
    project_id: i64,
    issue_title: &str,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    inner_get_task(task_id, pool).await?; // fail before creating an issue
    let issue_id = inner_resolve_issue(project_id, issue_title, pool).await?;
    sqlx::query("UPDATE tasks SET IssueId = ?, UpdatedAt = ? WHERE id = ?")
        .bind(issue_id)
        .bind(stamp())
        .bind(task_id)
        .execute(pool)
        .await?;
    inner_get_task(task_id, pool).await
}

/// Adds a reminder (at most ten per task). A custom time must be in the future; a relative
/// one whose time has already passed is stored dismissed (no notification).
pub async fn inner_add_task_reminder(
    task_id: i64,
    reminder: &TaskReminderInput,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let cleaned = clean_reminder(reminder, now)?;
    let mut tx = pool.begin().await?;
    load_task_row(task_id, &mut tx)
        .await?
        .ok_or_else(not_found)?;
    if reminder_count(task_id, &mut tx).await? >= MAX_REMINDERS {
        return Err(custom("A task can have at most 10 reminders"));
    }
    insert_reminder(task_id, cleaned, &mut tx).await?;
    reconcile_reminders(task_id, now, &mut tx).await?;
    touch_task(task_id, &mut tx).await?;
    let task = get_task_in(task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Changes a reminder's offset or custom time (switching between the two is fine). A moved
/// time re-arms it or dismisses it silently (D20); an unchanged one keeps its state.
pub async fn inner_update_task_reminder(
    id: i64,
    reminder: &TaskReminderInput,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let (offset, at) = clean_reminder(reminder, now)?;
    let mut tx = pool.begin().await?;
    let task_id = task_of_reminder(id, &mut tx).await?;
    sqlx::query("UPDATE task_reminders SET OffsetMinutes = ?, At = ? WHERE id = ?")
        .bind(offset)
        .bind(at)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    reconcile_reminders(task_id, now, &mut tx).await?;
    touch_task(task_id, &mut tx).await?;
    let task = get_task_in(task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Deletes a reminder; returns its task.
pub async fn inner_delete_task_reminder(id: i64, pool: &Pool<Sqlite>) -> Result<Task, AppError> {
    let mut tx = pool.begin().await?;
    let task_id = task_of_reminder(id, &mut tx).await?;
    sqlx::query("DELETE FROM task_reminders WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    touch_task(task_id, &mut tx).await?;
    let task = get_task_in(task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// Moves only this reminder to `until` (back to pending; Badger-only, the due stays). A done
/// task's reminder is returned unchanged.
pub async fn inner_snooze_task_reminder(
    id: i64,
    until: &str,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Task, AppError> {
    let until = resolve_gap(parse_timestamp(until)?, local_exists);
    if until <= now {
        return Err(custom("Time is in the past"));
    }
    let mut tx = pool.begin().await?;
    let task_id = task_of_reminder(id, &mut tx).await?;
    let snoozed = sqlx::query(
        "UPDATE task_reminders SET State = 'pending', SnoozedUntil = ? \
         WHERE id = ? AND TaskId IN (SELECT id FROM tasks WHERE Done = 0)",
    )
    .bind(format_timestamp(until))
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if snoozed.rows_affected() > 0 {
        touch_task(task_id, &mut tx).await?;
    }
    let task = get_task_in(task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

/// The toast's ×: the reminder is dismissed and stops counting as overdue.
pub async fn inner_dismiss_task_reminder(id: i64, pool: &Pool<Sqlite>) -> Result<Task, AppError> {
    let mut tx = pool.begin().await?;
    let task_id = task_of_reminder(id, &mut tx).await?;
    let dismissed = sqlx::query(
        "UPDATE task_reminders SET State = 'dismissed', SnoozedUntil = NULL \
         WHERE id = ? AND NOT (State = 'dismissed' AND SnoozedUntil IS NULL)",
    )
    .bind(id)
    .execute(&mut *tx)
    .await?;
    if dismissed.rows_affected() > 0 {
        touch_task(task_id, &mut tx).await?;
    }
    let task = get_task_in(task_id, &mut tx).await?;
    tx.commit().await?;
    Ok(task)
}

// ── Scheduler side ────────────────────────────────────────────────────────────

/// A reminder the scheduler just sent, with what its notification shows.
#[derive(Debug, Clone, PartialEq)]
pub struct FiredReminder {
    pub reminder_id: i64,
    pub task_id: i64,
    pub task_title: String,
    pub note_title: Option<String>,
    /// The time it fired for (its snooze or its base time).
    pub fire_at: NaiveDateTime,
}

#[derive(Debug, Clone, FromRow)]
struct PendingRow {
    id: i64,
    task_id: i64,
    offset_minutes: Option<i64>,
    at: Option<String>,
    snoozed_until: Option<String>,
    fired_for: Option<String>,
    title: String,
    note_title: Option<String>,
    due_date: Option<String>,
    due_time: Option<String>,
    updated_at: String,
}

/// The scheduler's "fire due" step: every pending reminder of an open task whose time has
/// come becomes `sent`, through a conditional update that matches the fire time and task
/// version it was computed from (`FiredFor`, `SnoozedUntil`, the task's `UpdatedAt`), so a
/// concurrent edit, snooze or completion wins and a stale time never fires. A reminder whose
/// `FiredFor` no longer matches its time is reconciled instead (silently). Returns what was
/// sent, by fire time.
pub async fn inner_fire_due(
    pool: &Pool<Sqlite>,
    now: NaiveDateTime,
) -> Result<Vec<FiredReminder>, AppError> {
    let rows = sqlx::query_as::<_, PendingRow>(
        "SELECT r.id, r.TaskId AS task_id, r.OffsetMinutes AS offset_minutes, r.At AS at, \
             r.SnoozedUntil AS snoozed_until, r.FiredFor AS fired_for, t.Title AS title, \
             n.Title AS note_title, t.DueDate AS due_date, t.DueTime AS due_time, \
             t.UpdatedAt AS updated_at \
         FROM task_reminders r JOIN tasks t ON t.id = r.TaskId \
         LEFT JOIN notes n ON n.id = t.NoteId \
         WHERE r.State = 'pending' AND t.Done = 0 ORDER BY r.id",
    )
    .fetch_all(pool)
    .await?;
    let mut fired = Vec::new();
    let mut stale = Vec::new();
    for row in rows {
        let anchor = due_anchor(
            row.due_date.as_deref(),
            row.due_time.as_deref(),
            local_exists,
        );
        let base = base_fire_time(anchor, row.offset_minutes, row.at.as_deref(), local_exists);
        if base.map(format_timestamp) != row.fired_for {
            stale.push(row.task_id);
            continue;
        }
        let snoozed = row
            .snoozed_until
            .as_deref()
            .and_then(|s| parse_timestamp(s).ok());
        let Some(fire_at) = snoozed.or(base) else {
            continue;
        };
        if fire_at > now {
            continue;
        }
        if mark_sent(&row, pool).await? {
            fired.push(FiredReminder {
                reminder_id: row.id,
                task_id: row.task_id,
                task_title: row.title,
                note_title: row.note_title,
                fire_at,
            });
        }
    }
    stale.sort_unstable();
    stale.dedup();
    for task_id in stale {
        let mut tx = pool.begin().await?;
        reconcile_reminders(task_id, now, &mut tx).await?;
        tx.commit().await?;
    }
    fired.sort_by_key(|f| (f.fire_at, f.reminder_id));
    Ok(fired)
}

/// The conditional pending → sent write: only while the reminder and its task are exactly as
/// `row` read them (same fire time, snooze and task version, task still open).
async fn mark_sent(row: &PendingRow, pool: &Pool<Sqlite>) -> Result<bool, AppError> {
    let sent: Option<i64> = sqlx::query_scalar(
        "UPDATE task_reminders SET State = 'sent' \
         WHERE id = ? AND State = 'pending' AND FiredFor IS ? AND SnoozedUntil IS ? \
           AND TaskId IN (SELECT id FROM tasks WHERE id = ? AND Done = 0 AND UpdatedAt = ?) \
         RETURNING id",
    )
    .bind(row.id)
    .bind(&row.fired_for)
    .bind(&row.snoozed_until)
    .bind(row.task_id)
    .bind(&row.updated_at)
    .fetch_optional(pool)
    .await?;
    Ok(sent.is_some())
}

/// D10: open tasks that are past due (an earlier date, or today at a time that has come) or
/// have a sent reminder. The dock badge and the nav count.
pub async fn inner_count_overdue(pool: &Pool<Sqlite>, now: NaiveDateTime) -> Result<i64, AppError> {
    let (n,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM tasks t WHERE t.Done = 0 AND ( \
             (t.DueDate IS NOT NULL AND (t.DueDate < ?1 \
                 OR (t.DueDate = ?1 AND t.DueTime IS NOT NULL AND t.DueTime <= ?2))) \
             OR EXISTS (SELECT 1 FROM task_reminders r WHERE r.TaskId = t.id AND r.State = 'sent'))",
    )
    .bind(now.date().format(DATE_FORMAT).to_string())
    .bind(now.time().format(TIME_FORMAT).to_string())
    .fetch_one(pool)
    .await?;
    Ok(n)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

use crate::task_scheduler::{tasks_changed, tasks_changed_later};

#[tauri::command]
pub async fn list_tasks(
    done_limit: Option<i64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<Task>, AppError> {
    inner_list_tasks(done_limit, pool.inner()).await
}

#[tauri::command]
pub async fn get_task(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<Task, AppError> {
    inner_get_task(id, pool.inner()).await
}

#[tauri::command]
pub async fn create_task(
    input: TaskInput,
    reminders: Option<Vec<TaskReminderInput>>,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let reminders = reminders.unwrap_or_default();
    let task = inner_create_task(&input, &reminders, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn update_task(
    id: i64,
    input: TaskInput,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_update_task(id, &input, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

/// Completes a task. The sync nudge waits until the Undo window has closed.
#[tauri::command]
pub async fn complete_task(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TaskCompletion, AppError> {
    let done = inner_complete_task(id, local_now(), pool.inner()).await?;
    tasks_changed_later(&app, pool.inner()).await;
    Ok(done)
}

#[tauri::command]
pub async fn undo_complete_task(
    undo: TaskUndo,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_undo_complete_task(&undo, pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn reopen_task(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_reopen_task(id, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn delete_task(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    inner_delete_task(id, pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn create_issue_for_task(
    task_id: i64,
    project_id: i64,
    issue_title: String,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_create_issue_for_task(task_id, project_id, &issue_title, pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn add_task_reminder(
    task_id: i64,
    reminder: TaskReminderInput,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_add_task_reminder(task_id, &reminder, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn update_task_reminder(
    id: i64,
    reminder: TaskReminderInput,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_update_task_reminder(id, &reminder, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn delete_task_reminder(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_delete_task_reminder(id, pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn snooze_task_reminder(
    id: i64,
    until: String,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_snooze_task_reminder(id, &until, local_now(), pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

#[tauri::command]
pub async fn dismiss_task_reminder(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Task, AppError> {
    let task = inner_dismiss_task_reminder(id, pool.inner()).await?;
    tasks_changed(&app, pool.inner()).await;
    Ok(task)
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
