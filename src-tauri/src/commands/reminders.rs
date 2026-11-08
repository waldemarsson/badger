//! Reminders: one-off or recurring pings at a naive local wall-clock time.
//!
//! Times are stored as `%Y-%m-%dT%H:%M:%S%.3f` local strings, so string comparison orders
//! them and "09:00" stays 09:00 across DST and time-zone changes. `RemindAt` is the current
//! occurrence; recurring rows also keep `TimeOfDay` (the `HH:MM` anchor every occurrence
//! returns to, unaffected by snooze or a DST shift) and the rule.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};

use crate::reminders_scheduler::reminders_changed;
use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite, SqliteConnection};
use tauri::{AppHandle, State};

/// Storage format of every reminder timestamp (same as todos and notes).
pub const TIMESTAMP_FORMAT: &str = "%Y-%m-%dT%H:%M:%S%.3f";
/// Storage format of `TimeOfDay`.
const TIME_OF_DAY_FORMAT: &str = "%H:%M";
/// How far ahead the occurrence search looks. Every rule matches at least once a month.
const SEARCH_DAYS: i64 = 400;

// ── Recurrence rule ───────────────────────────────────────────────────────────

/// A recurrence rule. One-off reminders have no rule (`Option<Rule>` is `None`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rule {
    Daily,
    /// Monday to Friday.
    Weekdays,
    /// ISO weekdays (1 = Monday … 7 = Sunday), sorted, deduplicated, never empty.
    Weekly(Vec<u32>),
    /// Day of the month, 1–31; shorter months use their last day.
    Monthly(u32),
}

impl Rule {
    /// Builds a rule from user input, with user-facing validation messages.
    pub fn from_input(
        repeat: &str,
        weekdays: &[i64],
        month_day: Option<i64>,
    ) -> Result<Option<Rule>, AppError> {
        match repeat {
            "none" => Ok(None),
            "daily" => Ok(Some(Rule::Daily)),
            "weekdays" => Ok(Some(Rule::Weekdays)),
            "weekly" => {
                if weekdays.is_empty() {
                    return Err(custom("Pick at least one weekday"));
                }
                let mut days = Vec::with_capacity(weekdays.len());
                for &d in weekdays {
                    match u32::try_from(d) {
                        Ok(d) if (1..=7).contains(&d) => days.push(d),
                        _ => return Err(custom("Invalid weekday")),
                    }
                }
                days.sort_unstable();
                days.dedup();
                Ok(Some(Rule::Weekly(days)))
            }
            "monthly" => match month_day.and_then(|d| u32::try_from(d).ok()) {
                Some(d) if (1..=31).contains(&d) => Ok(Some(Rule::Monthly(d))),
                _ => Err(custom("Pick a day of the month (1–31)")),
            },
            _ => Err(custom("Invalid repeat")),
        }
    }

    /// Reads a rule back from its stored columns (`Repeat`, `Weekdays`, `MonthDay`).
    pub fn from_columns(
        repeat: &str,
        weekdays: &str,
        month_day: Option<i64>,
    ) -> Result<Option<Rule>, AppError> {
        let days: Vec<i64> = parse_weekdays(weekdays)
            .into_iter()
            .map(i64::from)
            .collect();
        Rule::from_input(repeat, &days, month_day)
    }

    /// Whether an occurrence falls on `date`.
    pub fn matches(&self, date: NaiveDate) -> bool {
        let weekday = date.weekday().number_from_monday();
        match self {
            Rule::Daily => true,
            Rule::Weekdays => weekday <= 5,
            Rule::Weekly(days) => days.contains(&weekday),
            Rule::Monthly(day) => date.day() == (*day).min(last_day_of_month(date)),
        }
    }
}

/// `Repeat` column value for an optional rule.
pub fn repeat_name(rule: Option<&Rule>) -> &'static str {
    match rule {
        None => "none",
        Some(Rule::Daily) => "daily",
        Some(Rule::Weekdays) => "weekdays",
        Some(Rule::Weekly(_)) => "weekly",
        Some(Rule::Monthly(_)) => "monthly",
    }
}

/// `Weekdays` column value: `"1,4"` for weekly rules, empty otherwise.
pub fn weekdays_column(rule: Option<&Rule>) -> String {
    match rule {
        Some(Rule::Weekly(days)) => days
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(","),
        _ => String::new(),
    }
}

/// `MonthDay` column value: the day for monthly rules, `NULL` otherwise.
pub fn month_day_column(rule: Option<&Rule>) -> Option<i64> {
    match rule {
        Some(Rule::Monthly(day)) => Some(i64::from(*day)),
        _ => None,
    }
}

/// Parses a stored `Weekdays` value; ignores anything that is not 1–7.
pub fn parse_weekdays(stored: &str) -> Vec<u32> {
    stored
        .split(',')
        .filter_map(|s| s.trim().parse::<u32>().ok())
        .filter(|d| (1..=7).contains(d))
        .collect()
}

fn last_day_of_month(date: NaiveDate) -> u32 {
    let (y, m) = (date.year(), date.month());
    let next = if m == 12 {
        NaiveDate::from_ymd_opt(y + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(y, m + 1, 1)
    };
    next.and_then(|n| n.pred_opt()).map_or(28, |d| d.day())
}

// ── Local time ────────────────────────────────────────────────────────────────

/// Whether `naive` exists on the local clock (false inside a DST gap).
pub fn local_exists(naive: NaiveDateTime) -> bool {
    chrono::Local
        .from_local_datetime(&naive)
        .earliest()
        .is_some()
}

/// The local wall-clock time now.
pub fn local_now() -> NaiveDateTime {
    chrono::Local::now().naive_local()
}

/// Moves a time that does not exist locally (DST gap) to the first valid minute after.
/// `exists` is `local_exists` outside tests.
pub fn resolve_gap(naive: NaiveDateTime, exists: impl Fn(NaiveDateTime) -> bool) -> NaiveDateTime {
    if exists(naive) {
        return naive;
    }
    let Some(mut t) = naive.with_second(0).and_then(|t| t.with_nanosecond(0)) else {
        return naive;
    };
    // A gap is at most a few hours; give up after a day and keep the original.
    for _ in 0..(24 * 60) {
        t += Duration::minutes(1);
        if exists(t) {
            return t;
        }
    }
    naive
}

/// Parses a timestamp from the frontend or the database. Seconds and fractions are optional.
pub fn parse_timestamp(s: &str) -> Result<NaiveDateTime, AppError> {
    let s = s.trim();
    NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f")
        .or_else(|_| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M"))
        .map_err(|_| custom("Invalid time"))
}

pub fn format_timestamp(t: NaiveDateTime) -> String {
    t.format(TIMESTAMP_FORMAT).to_string()
}

pub fn format_time_of_day(t: NaiveTime) -> String {
    t.format(TIME_OF_DAY_FORMAT).to_string()
}

/// Parses a stored `TimeOfDay` (`HH:MM`).
pub fn parse_time_of_day(s: &str) -> Result<NaiveTime, AppError> {
    NaiveTime::parse_from_str(s.trim(), TIME_OF_DAY_FORMAT).map_err(|_| custom("Invalid time"))
}

/// `HH:MM` of `t`, seconds dropped: the anchor every occurrence returns to.
pub fn anchor_of(t: NaiveDateTime) -> NaiveTime {
    NaiveTime::from_hms_opt(t.hour(), t.minute(), 0).unwrap_or(NaiveTime::MIN)
}

// ── Occurrences ───────────────────────────────────────────────────────────────

/// The occurrence on `date` at `time_of_day`, moved out of a DST gap.
fn occurrence_on(
    date: NaiveDate,
    time_of_day: NaiveTime,
    exists: &impl Fn(NaiveDateTime) -> bool,
) -> NaiveDateTime {
    resolve_gap(date.and_time(time_of_day), exists)
}

/// First occurrence at or after `start`: a recurring reminder's first time, moved forward
/// onto the first matching day (`every weekday 9` typed on Saturday → Monday 09:00).
pub fn first_occurrence(
    rule: &Rule,
    start: NaiveDateTime,
    time_of_day: NaiveTime,
    exists: impl Fn(NaiveDateTime) -> bool,
) -> Option<NaiveDateTime> {
    (0..SEARCH_DAYS)
        .filter_map(|i| start.date().checked_add_signed(Duration::days(i)))
        .filter(|d| rule.matches(*d))
        .map(|d| occurrence_on(d, time_of_day, &exists))
        .find(|t| *t >= start)
}

/// First occurrence strictly after `after`, at `time_of_day`.
pub fn next_occurrence(
    rule: &Rule,
    after: NaiveDateTime,
    time_of_day: NaiveTime,
    exists: impl Fn(NaiveDateTime) -> bool,
) -> Option<NaiveDateTime> {
    (0..SEARCH_DAYS)
        .filter_map(|i| after.date().checked_add_signed(Duration::days(i)))
        .filter(|d| rule.matches(*d))
        .map(|d| occurrence_on(d, time_of_day, &exists))
        .find(|t| *t > after)
}

/// The latest occurrence in `(remind_at, now]`, if any: where a recurring reminder that
/// came due (or was ignored) moves to, so missed occurrences collapse into one.
pub fn latest_due_occurrence(
    rule: &Rule,
    remind_at: NaiveDateTime,
    time_of_day: NaiveTime,
    now: NaiveDateTime,
    exists: impl Fn(NaiveDateTime) -> bool,
) -> Option<NaiveDateTime> {
    for i in 0..SEARCH_DAYS {
        let date = now.date().checked_sub_signed(Duration::days(i))?;
        if date < remind_at.date() {
            return None;
        }
        if !rule.matches(date) {
            continue;
        }
        let t = occurrence_on(date, time_of_day, &exists);
        if t <= remind_at {
            return None;
        }
        if t <= now {
            return Some(t);
        }
    }
    None
}

// ── Rows and input ────────────────────────────────────────────────────────────

/// Maximum reminder text length in characters (not bytes).
pub const MAX_TEXT_CHARS: usize = 500;
/// How many done reminders the list returns (all are kept).
const DONE_LIST_LIMIT: i64 = 50;

/// A reminder with its linked todo or note (LEFT JOINs: null when unlinked).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Reminder {
    pub id: i64,
    pub text: String,
    pub remind_at: String,
    pub time_of_day: String,
    /// `none|daily|weekdays|weekly|monthly`
    pub repeat: String,
    /// ISO weekdays (1 = Monday … 7 = Sunday); only set for weekly rules.
    pub weekdays: Vec<u32>,
    pub month_day: Option<i64>,
    /// `pending|fired|done`
    pub state: String,
    pub todo_id: Option<i64>,
    pub note_id: Option<i64>,
    pub todo_title: Option<String>,
    pub todo_status: Option<String>,
    pub note_title: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(FromRow)]
struct ReminderRow {
    id: i64,
    text: String,
    remind_at: String,
    time_of_day: String,
    repeat: String,
    weekdays: String,
    month_day: Option<i64>,
    state: String,
    todo_id: Option<i64>,
    note_id: Option<i64>,
    todo_title: Option<String>,
    todo_status: Option<String>,
    note_title: Option<String>,
    created_at: String,
    updated_at: String,
}

impl From<ReminderRow> for Reminder {
    fn from(r: ReminderRow) -> Self {
        Reminder {
            id: r.id,
            text: r.text,
            remind_at: r.remind_at,
            time_of_day: r.time_of_day,
            repeat: r.repeat,
            weekdays: parse_weekdays(&r.weekdays),
            month_day: r.month_day,
            state: r.state,
            todo_id: r.todo_id,
            note_id: r.note_id,
            todo_title: r.todo_title,
            todo_status: r.todo_status,
            note_title: r.note_title,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }
    }
}

impl Reminder {
    fn rule(&self) -> Result<Option<Rule>, AppError> {
        let days: Vec<i64> = self.weekdays.iter().copied().map(i64::from).collect();
        Rule::from_input(&self.repeat, &days, self.month_day)
    }
}

/// What the create and edit forms send (camelCase over `invoke`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReminderInput {
    pub text: String,
    /// Local timestamp, e.g. `2026-11-09T09:00` or `2026-11-09T09:00:00.000`.
    pub remind_at: String,
    #[serde(default = "default_repeat")]
    pub repeat: String,
    #[serde(default)]
    pub weekdays: Vec<i64>,
    #[serde(default)]
    pub month_day: Option<i64>,
    #[serde(default)]
    pub todo_id: Option<i64>,
    #[serde(default)]
    pub note_id: Option<i64>,
}

fn default_repeat() -> String {
    "none".to_string()
}

const SELECT_REMINDERS: &str = "SELECT r.id,
        r.Text      AS text,
        r.RemindAt  AS remind_at,
        r.TimeOfDay AS time_of_day,
        r.Repeat    AS repeat,
        r.Weekdays  AS weekdays,
        r.MonthDay  AS month_day,
        r.State     AS state,
        r.TodoId    AS todo_id,
        r.NoteId    AS note_id,
        t.Title     AS todo_title,
        t.Status    AS todo_status,
        n.Title     AS note_title,
        r.CreatedAt AS created_at,
        r.UpdatedAt AS updated_at
 FROM reminders r
 LEFT JOIN todos t ON t.id = r.TodoId
 LEFT JOIN notes n ON n.id = r.NoteId";

// ── Validation helpers ────────────────────────────────────────────────────────

fn not_found() -> AppError {
    custom("Reminder not found")
}

fn clean_text(text: &str) -> Result<String, AppError> {
    let t = text.trim();
    if t.is_empty() {
        return Err(custom("Text is required"));
    }
    if t.chars().count() > MAX_TEXT_CHARS {
        return Err(custom("Text is too long (max 500 characters)"));
    }
    Ok(t.to_string())
}

/// Checks that at most one link is set and that it exists.
async fn check_links(
    todo_id: Option<i64>,
    note_id: Option<i64>,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    if todo_id.is_some() && note_id.is_some() {
        return Err(custom("A reminder can link a todo or a note, not both"));
    }
    if let Some(id) = todo_id {
        let found: Option<(i64,)> = sqlx::query_as("SELECT id FROM todos WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if found.is_none() {
            return Err(custom("Todo not found"));
        }
    }
    if let Some(id) = note_id {
        let found: Option<(i64,)> = sqlx::query_as("SELECT id FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
        if found.is_none() {
            return Err(custom("Note not found"));
        }
    }
    Ok(())
}

/// A validated schedule: the first occurrence, its anchor and the rule.
struct Schedule {
    remind_at: NaiveDateTime,
    time_of_day: NaiveTime,
    rule: Option<Rule>,
}

/// Validates a requested time and, for recurring rules, moves it onto the first matching
/// day. A one-off must be in the future; a recurring start that has passed (today at an
/// earlier time, or an earlier date) rolls forward to the first occurrence after now. The
/// anchor is the requested `HH:MM`, before any DST shift.
fn schedule(requested: &str, rule: Option<Rule>, now: NaiveDateTime) -> Result<Schedule, AppError> {
    let requested = parse_timestamp(requested)?;
    let time_of_day = anchor_of(requested);
    let at = resolve_gap(requested, local_exists);
    let remind_at = match &rule {
        None if at <= now => return Err(custom("Time is in the past")),
        None => at,
        Some(r) if at <= now => next_occurrence(r, now, time_of_day, local_exists)
            .ok_or_else(|| custom("Couldn't find the next occurrence"))?,
        Some(r) => first_occurrence(r, at, time_of_day, local_exists)
            .ok_or_else(|| custom("Couldn't find the next occurrence"))?,
    };
    Ok(Schedule {
        remind_at,
        time_of_day,
        rule,
    })
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

pub async fn inner_get_reminder(id: i64, pool: &Pool<Sqlite>) -> Result<Reminder, AppError> {
    let sql = format!("{SELECT_REMINDERS} WHERE r.id = ?");
    sqlx::query_as::<_, ReminderRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .map(Reminder::from)
        .ok_or_else(not_found)
}

/// Pending and fired reminders by time, then the 50 most recently done.
pub async fn inner_list_reminders(pool: &Pool<Sqlite>) -> Result<Vec<Reminder>, AppError> {
    let open = format!(
        "{SELECT_REMINDERS} WHERE r.State IN ('pending', 'fired') ORDER BY r.RemindAt, r.id"
    );
    let done = format!(
        "{SELECT_REMINDERS} WHERE r.State = 'done' ORDER BY r.UpdatedAt DESC, r.id DESC LIMIT ?"
    );
    let mut list: Vec<Reminder> = sqlx::query_as::<_, ReminderRow>(&open)
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Reminder::from)
        .collect();
    list.extend(
        sqlx::query_as::<_, ReminderRow>(&done)
            .bind(DONE_LIST_LIMIT)
            .fetch_all(pool)
            .await?
            .into_iter()
            .map(Reminder::from),
    );
    Ok(list)
}

pub async fn inner_create_reminder(
    input: &ReminderInput,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Reminder, AppError> {
    let text = clean_text(&input.text)?;
    let rule = Rule::from_input(&input.repeat, &input.weekdays, input.month_day)?;
    let s = schedule(&input.remind_at, rule, now)?;
    check_links(input.todo_id, input.note_id, pool).await?;
    let stamp = format_timestamp(local_now());
    let rule = s.rule.as_ref();

    let row: (i64,) = sqlx::query_as(
        "INSERT INTO reminders (Text, RemindAt, TimeOfDay, Repeat, Weekdays, MonthDay, State, \
             TodoId, NoteId, CreatedAt, UpdatedAt) \
         VALUES (?, ?, ?, ?, ?, ?, 'pending', ?, ?, ?, ?) RETURNING id",
    )
    .bind(&text)
    .bind(format_timestamp(s.remind_at))
    .bind(format_time_of_day(s.time_of_day))
    .bind(repeat_name(rule))
    .bind(weekdays_column(rule))
    .bind(month_day_column(rule))
    .bind(input.todo_id)
    .bind(input.note_id)
    .bind(&stamp)
    .bind(&stamp)
    .fetch_one(pool)
    .await?;
    inner_get_reminder(row.0, pool).await
}

/// Edits a reminder. A changed time or rule reschedules it (back to pending, the anchor
/// resets; see `schedule` for past times); a text- or link-only edit keeps its time and
/// state. Done reminders can't be edited.
pub async fn inner_update_reminder(
    id: i64,
    input: &ReminderInput,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Reminder, AppError> {
    let current = inner_get_reminder(id, pool).await?;
    if current.state == "done" {
        return Err(custom("Done reminders can't be edited"));
    }
    let text = clean_text(&input.text)?;
    let rule = Rule::from_input(&input.repeat, &input.weekdays, input.month_day)?;
    let requested = parse_timestamp(&input.remind_at)?;
    let rescheduled =
        requested != parse_timestamp(&current.remind_at)? || rule != current.rule()?;
    check_links(input.todo_id, input.note_id, pool).await?;
    let stamp = format_timestamp(local_now());

    let result = if rescheduled {
        let s = schedule(&input.remind_at, rule, now)?;
        let rule = s.rule.as_ref();
        sqlx::query(
            "UPDATE reminders SET Text = ?, RemindAt = ?, TimeOfDay = ?, Repeat = ?, Weekdays = ?, \
                 MonthDay = ?, State = 'pending', TodoId = ?, NoteId = ?, UpdatedAt = ? \
             WHERE id = ? AND State != 'done'",
        )
        .bind(&text)
        .bind(format_timestamp(s.remind_at))
        .bind(format_time_of_day(s.time_of_day))
        .bind(repeat_name(rule))
        .bind(weekdays_column(rule))
        .bind(month_day_column(rule))
        .bind(input.todo_id)
        .bind(input.note_id)
        .bind(&stamp)
        .bind(id)
        .execute(pool)
        .await?
    } else {
        sqlx::query(
            "UPDATE reminders SET Text = ?, TodoId = ?, NoteId = ?, UpdatedAt = ? \
             WHERE id = ? AND State != 'done'",
        )
        .bind(&text)
        .bind(input.todo_id)
        .bind(input.note_id)
        .bind(&stamp)
        .bind(id)
        .execute(pool)
        .await?
    };
    if result.rows_affected() == 0 {
        // Deleted or completed since it was read.
        inner_get_reminder(id, pool).await?;
        return Err(custom("Done reminders can't be edited"));
    }
    inner_get_reminder(id, pool).await
}

pub async fn inner_delete_reminder(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM reminders WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(())
}

/// Moves only the current occurrence to `until` (back to pending); the anchor and rule stay.
/// A done reminder is returned unchanged.
pub async fn inner_snooze_reminder(
    id: i64,
    until: &str,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Reminder, AppError> {
    let current = inner_get_reminder(id, pool).await?;
    if current.state == "done" {
        return Ok(current);
    }
    let until = resolve_gap(parse_timestamp(until)?, local_exists);
    if until <= now {
        return Err(custom("Time is in the past"));
    }
    sqlx::query(
        "UPDATE reminders SET RemindAt = ?, State = 'pending', UpdatedAt = ? \
         WHERE id = ? AND State IN ('pending', 'fired')",
    )
    .bind(format_timestamp(until))
    .bind(format_timestamp(local_now()))
    .bind(id)
    .execute(pool)
    .await?;
    // Zero rows: it was completed or deleted meanwhile; report what is there now.
    inner_get_reminder(id, pool).await
}

/// Done: a one-off becomes done; a recurring one moves to the first occurrence strictly
/// after max(now, RemindAt), back to pending. A done reminder is returned unchanged.
pub async fn inner_complete_reminder(
    id: i64,
    now: NaiveDateTime,
    pool: &Pool<Sqlite>,
) -> Result<Reminder, AppError> {
    // The update is conditional on the state and time read, so a concurrent fire or snooze
    // isn't overwritten; on a lost race, read again and retry.
    for _ in 0..3 {
        let current = inner_get_reminder(id, pool).await?;
        if current.state == "done" {
            return Ok(current);
        }
        let stamp = format_timestamp(local_now());
        let result = match current.rule()? {
            None => {
                sqlx::query(
                    "UPDATE reminders SET State = 'done', UpdatedAt = ? \
                     WHERE id = ? AND State IN ('pending', 'fired')",
                )
                .bind(&stamp)
                .bind(id)
                .execute(pool)
                .await?
            }
            Some(rule) => {
                let remind_at = parse_timestamp(&current.remind_at)?;
                let time_of_day = parse_time_of_day(&current.time_of_day)?;
                let next = next_occurrence(&rule, remind_at.max(now), time_of_day, local_exists)
                    .ok_or_else(|| custom("Couldn't find the next occurrence"))?;
                sqlx::query(
                    "UPDATE reminders SET RemindAt = ?, State = 'pending', UpdatedAt = ? \
                     WHERE id = ? AND State = ? AND RemindAt = ?",
                )
                .bind(format_timestamp(next))
                .bind(&stamp)
                .bind(id)
                .bind(&current.state)
                .bind(&current.remind_at)
                .execute(pool)
                .await?
            }
        };
        if result.rows_affected() > 0 {
            return inner_get_reminder(id, pool).await;
        }
    }
    Err(custom("Reminder changed meanwhile, try again"))
}

/// Marks a todo's pending and fired reminders done (recurring series included). Runs inside
/// the transaction that moves the todo to done.
pub async fn complete_for_todo(todo_id: i64, conn: &mut SqliteConnection) -> Result<u64, AppError> {
    let result = sqlx::query(
        "UPDATE reminders SET State = 'done', UpdatedAt = ? \
         WHERE TodoId = ? AND State IN ('pending', 'fired')",
    )
    .bind(format_timestamp(local_now()))
    .bind(todo_id)
    .execute(conn)
    .await?;
    Ok(result.rows_affected())
}

/// The scheduler's "fire due" step, in one transaction:
///
/// 1. one conditional `UPDATE … SET State = 'fired' WHERE State = 'pending' AND RemindAt <=
///    now RETURNING id`, so a row snoozed or completed first is not fired;
/// 2. recurring rows that are fired (just now or earlier, ignored) move to their latest
///    occurrence in `(RemindAt, now]`: missed occurrences collapse into one, and an ignored
///    series fires again when its next occurrence comes due instead of stalling.
///
/// Returns the rows to notify about (fired or re-fired), by time.
pub async fn inner_fire_due(
    pool: &Pool<Sqlite>,
    now: NaiveDateTime,
) -> Result<Vec<Reminder>, AppError> {
    let now_s = format_timestamp(now);
    let mut tx = pool.begin().await?;

    let mut ids: Vec<i64> = sqlx::query_as::<_, (i64,)>(
        "UPDATE reminders SET State = 'fired', UpdatedAt = ? \
         WHERE State = 'pending' AND RemindAt <= ? RETURNING id",
    )
    .bind(&now_s)
    .bind(&now_s)
    .fetch_all(&mut *tx)
    .await?
    .into_iter()
    .map(|(id,)| id)
    .collect();

    let sql = format!("{SELECT_REMINDERS} WHERE r.State = 'fired' AND r.Repeat != 'none'");
    let recurring = sqlx::query_as::<_, ReminderRow>(&sql)
        .fetch_all(&mut *tx)
        .await?;
    for row in recurring.into_iter().map(Reminder::from) {
        let Some(latest) = latest_occurrence_of(&row, now) else {
            continue;
        };
        let moved = sqlx::query(
            "UPDATE reminders SET RemindAt = ?, UpdatedAt = ? \
             WHERE id = ? AND State = 'fired' AND RemindAt = ?",
        )
        .bind(format_timestamp(latest))
        .bind(&now_s)
        .bind(row.id)
        .bind(&row.remind_at)
        .execute(&mut *tx)
        .await?;
        if moved.rows_affected() > 0 && !ids.contains(&row.id) {
            ids.push(row.id);
        }
    }
    tx.commit().await?;

    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql =
        format!("{SELECT_REMINDERS} WHERE r.id IN ({placeholders}) ORDER BY r.RemindAt, r.id");
    let mut query = sqlx::query_as::<_, ReminderRow>(&sql);
    for id in &ids {
        query = query.bind(id);
    }
    Ok(query
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(Reminder::from)
        .collect())
}

/// Where a fired recurring row moves at `now`, if a later occurrence came due. A row whose
/// stored values don't parse is logged and left alone.
fn latest_occurrence_of(r: &Reminder, now: NaiveDateTime) -> Option<NaiveDateTime> {
    match parsed_schedule(r) {
        Ok((Some(rule), remind_at, time_of_day)) => {
            latest_due_occurrence(&rule, remind_at, time_of_day, now, local_exists)
        }
        Ok((None, ..)) => None,
        Err(e) => {
            tracing::error!(
                "Reminders: reminder {} has an invalid schedule: {}",
                r.id,
                e
            );
            None
        }
    }
}

/// A stored row's rule, current occurrence and anchor.
fn parsed_schedule(r: &Reminder) -> Result<(Option<Rule>, NaiveDateTime, NaiveTime), AppError> {
    Ok((
        r.rule()?,
        parse_timestamp(&r.remind_at)?,
        parse_time_of_day(&r.time_of_day)?,
    ))
}

/// Number of fired reminders (the dock badge).
pub async fn inner_count_fired(pool: &Pool<Sqlite>) -> Result<i64, AppError> {
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM reminders WHERE State = 'fired'")
        .fetch_one(pool)
        .await?;
    Ok(n)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_reminders(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<Reminder>, AppError> {
    inner_list_reminders(pool.inner()).await
}

#[tauri::command]
pub async fn create_reminder(
    input: ReminderInput,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Reminder, AppError> {
    let r = inner_create_reminder(&input, local_now(), pool.inner()).await?;
    reminders_changed(&app, pool.inner()).await;
    Ok(r)
}

#[tauri::command]
pub async fn update_reminder(
    id: i64,
    input: ReminderInput,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Reminder, AppError> {
    let r = inner_update_reminder(id, &input, local_now(), pool.inner()).await?;
    reminders_changed(&app, pool.inner()).await;
    Ok(r)
}

#[tauri::command]
pub async fn delete_reminder(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<(), AppError> {
    inner_delete_reminder(id, pool.inner()).await?;
    reminders_changed(&app, pool.inner()).await;
    Ok(())
}

#[tauri::command]
pub async fn snooze_reminder(
    id: i64,
    until: String,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Reminder, AppError> {
    let r = inner_snooze_reminder(id, &until, local_now(), pool.inner()).await?;
    reminders_changed(&app, pool.inner()).await;
    Ok(r)
}

#[tauri::command]
pub async fn complete_reminder(
    id: i64,
    app: AppHandle,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Reminder, AppError> {
    let r = inner_complete_reminder(id, local_now(), pool.inner()).await?;
    reminders_changed(&app, pool.inner()).await;
    Ok(r)
}

fn custom(msg: &str) -> AppError {
    AppError::Custom(msg.to_string())
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::{Pool, Sqlite};

    fn at(s: &str) -> NaiveDateTime {
        parse_timestamp(s).unwrap()
    }

    fn tod(s: &str) -> NaiveTime {
        parse_time_of_day(s).unwrap()
    }

    fn always(_: NaiveDateTime) -> bool {
        true
    }

    fn weekly(days: &[i64]) -> Rule {
        Rule::from_input("weekly", days, None).unwrap().unwrap()
    }

    // 2026-11-07 is a Saturday, 2026-11-09 a Monday.

    #[test]
    fn rule_input_validation() {
        assert_eq!(Rule::from_input("none", &[], None).unwrap(), None);
        assert_eq!(weekly(&[4, 1, 4]), Rule::Weekly(vec![1, 4]));
        let err = |r: Result<Option<Rule>, AppError>| r.unwrap_err().to_string();
        assert_eq!(
            err(Rule::from_input("weekly", &[], None)),
            "Pick at least one weekday"
        );
        assert_eq!(
            err(Rule::from_input("weekly", &[8], None)),
            "Invalid weekday"
        );
        assert!(err(Rule::from_input("monthly", &[], None)).contains("day of the month"));
        assert!(err(Rule::from_input("monthly", &[], Some(32))).contains("day of the month"));
        assert!(err(Rule::from_input("monthly", &[], Some(0))).contains("day of the month"));
        assert_eq!(err(Rule::from_input("hourly", &[], None)), "Invalid repeat");
    }

    #[test]
    fn rule_columns_round_trip() {
        for rule in [
            None,
            Some(Rule::Daily),
            Some(Rule::Weekdays),
            Some(Rule::Weekly(vec![1, 4])),
            Some(Rule::Monthly(31)),
        ] {
            let r = rule.as_ref();
            let back = Rule::from_columns(repeat_name(r), &weekdays_column(r), month_day_column(r))
                .unwrap();
            assert_eq!(back, rule);
        }
        assert_eq!(weekdays_column(Some(&Rule::Weekly(vec![1, 4]))), "1,4");
    }

    #[test]
    fn timestamps_parse_with_or_without_seconds_and_format_with_millis() {
        assert_eq!(
            format_timestamp(at("2026-11-09T09:00")),
            "2026-11-09T09:00:00.000"
        );
        assert_eq!(
            format_timestamp(at("2026-11-09T09:00:05.250")),
            "2026-11-09T09:00:05.250"
        );
        assert!(parse_timestamp("tomorrow").is_err());
        assert_eq!(
            format_time_of_day(anchor_of(at("2026-11-09T09:05:30"))),
            "09:05"
        );
    }

    #[test]
    fn first_occurrence_moves_onto_a_matching_day() {
        // `every weekday 9` typed on Saturday → Monday 09:00.
        let sat = at("2026-11-07T09:00");
        assert_eq!(
            first_occurrence(&Rule::Weekdays, sat, tod("09:00"), always),
            Some(at("2026-11-09T09:00"))
        );
        // Already on a matching day: unchanged.
        let mon = at("2026-11-09T09:00");
        assert_eq!(
            first_occurrence(&Rule::Weekdays, mon, tod("09:00"), always),
            Some(mon)
        );
        // Weekly Thu from Monday → Thursday.
        assert_eq!(
            first_occurrence(&weekly(&[4]), mon, tod("09:00"), always),
            Some(at("2026-11-12T09:00"))
        );
        // Monthly on the 15th from the 8th → the 15th; from the 20th → next month.
        assert_eq!(
            first_occurrence(
                &Rule::Monthly(15),
                at("2026-11-08T10:00"),
                tod("10:00"),
                always
            ),
            Some(at("2026-11-15T10:00"))
        );
        assert_eq!(
            first_occurrence(
                &Rule::Monthly(15),
                at("2026-11-20T10:00"),
                tod("10:00"),
                always
            ),
            Some(at("2026-12-15T10:00"))
        );
    }

    #[test]
    fn next_daily() {
        let next = next_occurrence(&Rule::Daily, at("2026-11-08T09:00"), tod("09:00"), always);
        assert_eq!(next, Some(at("2026-11-09T09:00")));
        // Strictly after: from 08:59 the same day's 09:00 is next.
        let next = next_occurrence(&Rule::Daily, at("2026-11-08T08:59"), tod("09:00"), always);
        assert_eq!(next, Some(at("2026-11-08T09:00")));
    }

    #[test]
    fn next_weekdays_skips_the_weekend() {
        let fri = at("2026-11-06T09:00");
        assert_eq!(
            next_occurrence(&Rule::Weekdays, fri, tod("09:00"), always),
            Some(at("2026-11-09T09:00"))
        );
    }

    #[test]
    fn next_weekly_mon_and_thu() {
        let rule = weekly(&[1, 4]);
        let mon = at("2026-11-09T09:00");
        let thu = next_occurrence(&rule, mon, tod("09:00"), always).unwrap();
        assert_eq!(thu, at("2026-11-12T09:00"));
        let mon2 = next_occurrence(&rule, thu, tod("09:00"), always).unwrap();
        assert_eq!(mon2, at("2026-11-16T09:00"));
    }

    #[test]
    fn next_monthly_on_the_15th() {
        let rule = Rule::Monthly(15);
        assert_eq!(
            next_occurrence(&rule, at("2026-11-15T09:00"), tod("09:00"), always),
            Some(at("2026-12-15T09:00"))
        );
        assert_eq!(
            next_occurrence(&rule, at("2026-12-15T09:00"), tod("09:00"), always),
            Some(at("2027-01-15T09:00"))
        );
    }

    #[test]
    fn monthly_on_the_31st_clamps_through_february_and_returns() {
        let rule = Rule::Monthly(31);
        let t = tod("09:00");
        let step = |from: &str| next_occurrence(&rule, at(from), t, always).unwrap();
        // Non-leap year 2027: Jan 31 → Feb 28 → Mar 31 → Apr 30 → May 31.
        assert_eq!(step("2027-01-31T09:00"), at("2027-02-28T09:00"));
        assert_eq!(step("2027-02-28T09:00"), at("2027-03-31T09:00"));
        assert_eq!(step("2027-03-31T09:00"), at("2027-04-30T09:00"));
        assert_eq!(step("2027-04-30T09:00"), at("2027-05-31T09:00"));
        // Leap year 2028: Jan 31 → Feb 29 → Mar 31.
        assert_eq!(step("2028-01-31T09:00"), at("2028-02-29T09:00"));
        assert_eq!(step("2028-02-29T09:00"), at("2028-03-31T09:00"));
    }

    #[test]
    fn next_after_a_snooze_returns_to_time_of_day() {
        // Daily 09:00 snoozed to 10:00: the next one is tomorrow 09:00, not 10:00.
        let next = next_occurrence(&Rule::Daily, at("2026-11-08T10:00"), tod("09:00"), always);
        assert_eq!(next, Some(at("2026-11-09T09:00")));
    }

    #[test]
    fn latest_due_occurrence_collapses_missed_ones() {
        // Daily 09:00 last at Nov 5; now Nov 8 12:00 → Nov 8 09:00 (one, not three).
        let got = latest_due_occurrence(
            &Rule::Daily,
            at("2026-11-05T09:00"),
            tod("09:00"),
            at("2026-11-08T12:00"),
            always,
        );
        assert_eq!(got, Some(at("2026-11-08T09:00")));
        // Now before today's 09:00 → yesterday's.
        let got = latest_due_occurrence(
            &Rule::Daily,
            at("2026-11-05T09:00"),
            tod("09:00"),
            at("2026-11-08T08:00"),
            always,
        );
        assert_eq!(got, Some(at("2026-11-07T09:00")));
        // Nothing new since remind_at.
        let got = latest_due_occurrence(
            &Rule::Daily,
            at("2026-11-08T09:00"),
            tod("09:00"),
            at("2026-11-08T12:00"),
            always,
        );
        assert_eq!(got, None);
        // A snoozed occurrence (10:00) is not moved back to that day's 09:00.
        let got = latest_due_occurrence(
            &Rule::Daily,
            at("2026-11-08T10:00"),
            tod("09:00"),
            at("2026-11-08T10:00:30"),
            always,
        );
        assert_eq!(got, None);
        // Weekdays: Friday's fired occurrence, now Sunday → nothing new until Monday.
        let got = latest_due_occurrence(
            &Rule::Weekdays,
            at("2026-11-06T09:00"),
            tod("09:00"),
            at("2026-11-08T12:00"),
            always,
        );
        assert_eq!(got, None);
    }

    #[test]
    fn dst_gap_time_moves_to_the_first_valid_minute() {
        // Simulated spring-forward: 02:00–02:59 on 2027-03-28 does not exist.
        let gap_day = NaiveDate::from_ymd_opt(2027, 3, 28).unwrap();
        let exists = move |n: NaiveDateTime| !(n.date() == gap_day && n.hour() == 2);
        assert_eq!(
            resolve_gap(at("2027-03-28T02:30"), exists),
            at("2027-03-28T03:00")
        );
        assert_eq!(
            resolve_gap(at("2027-03-27T02:30"), exists),
            at("2027-03-27T02:30")
        );
        // Daily 02:30: the gap day fires at 03:00, the next day returns to 02:30.
        let t = tod("02:30");
        let on_gap = next_occurrence(&Rule::Daily, at("2027-03-27T02:30"), t, exists).unwrap();
        assert_eq!(on_gap, at("2027-03-28T03:00"));
        let after = next_occurrence(&Rule::Daily, on_gap, t, exists).unwrap();
        assert_eq!(after, at("2027-03-29T02:30"));
        // local_exists answers for an ordinary time on the real clock.
        assert!(local_exists(at("2026-11-08T12:00")));
    }

    // ── Schema checks ─────────────────────────────────────────────────────────

    async fn insert_raw(
        pool: &Pool<Sqlite>,
        state: &str,
        todo_id: Option<i64>,
        note_id: Option<i64>,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "INSERT INTO reminders (Text, RemindAt, TimeOfDay, State, TodoId, NoteId, CreatedAt, UpdatedAt) \
             VALUES ('x', '2026-11-09T09:00:00.000', '09:00', ?, ?, ?, 'c', 'u')",
        )
        .bind(state)
        .bind(todo_id)
        .bind(note_id)
        .execute(pool)
        .await
        .map(|_| ())
    }

    #[tokio::test]
    async fn schema_checks_reject_two_links_and_bad_state() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO todos (id, Title, CreatedAt) VALUES (1, 't', 'c')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO notes (id, Title, CreatedAt, UpdatedAt) VALUES (1, 'n', 'c', 'u')",
        )
        .execute(&pool)
        .await
        .unwrap();
        assert!(insert_raw(&pool, "pending", Some(1), Some(1))
            .await
            .is_err());
        assert!(insert_raw(&pool, "snoozed", None, None).await.is_err());
        assert!(insert_raw(&pool, "pending", Some(1), None).await.is_ok());
        assert!(insert_raw(&pool, "fired", None, Some(1)).await.is_ok());
        assert!(insert_raw(&pool, "done", None, None).await.is_ok());
    }

    // ── Commands ──────────────────────────────────────────────────────────────

    fn input(text: &str, remind_at: &str) -> ReminderInput {
        ReminderInput {
            text: text.to_string(),
            remind_at: remind_at.to_string(),
            repeat: "none".to_string(),
            ..Default::default()
        }
    }

    fn repeating(text: &str, remind_at: &str, repeat: &str) -> ReminderInput {
        ReminderInput {
            repeat: repeat.to_string(),
            ..input(text, remind_at)
        }
    }

    async fn create(i: ReminderInput, now: &str, pool: &Pool<Sqlite>) -> Reminder {
        inner_create_reminder(&i, at(now), pool).await.unwrap()
    }

    async fn set_state(id: i64, state: &str, remind_at: &str, pool: &Pool<Sqlite>) {
        sqlx::query("UPDATE reminders SET State = ?, RemindAt = ? WHERE id = ?")
            .bind(state)
            .bind(format_timestamp(at(remind_at)))
            .bind(id)
            .execute(pool)
            .await
            .unwrap();
    }

    async fn complete(id: i64, now: &str, pool: &Pool<Sqlite>) -> Reminder {
        inner_complete_reminder(id, at(now), pool).await.unwrap()
    }

    async fn new_todo(pool: &Pool<Sqlite>) -> i64 {
        crate::commands::todos::inner_create_todo(
            "Call Anna",
            None,
            None,
            "not_started",
            None,
            pool,
        )
        .await
        .unwrap()
        .id
    }

    async fn new_note(pool: &Pool<Sqlite>) -> i64 {
        crate::commands::notes::inner_create_note_titled("Invoice", None, pool)
            .await
            .unwrap()
            .id
    }

    fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
        r.unwrap_err().to_string()
    }

    const NOW: &str = "2026-11-08T12:00"; // Sunday

    #[tokio::test]
    async fn create_one_off_trims_and_is_pending() {
        let pool = crate::test_db().await;
        let r = create(input("  Send invoice  ", "2026-11-09T09:30"), NOW, &pool).await;
        assert_eq!(r.text, "Send invoice");
        assert_eq!(r.remind_at, "2026-11-09T09:30:00.000");
        assert_eq!(r.time_of_day, "09:30");
        assert_eq!(r.repeat, "none");
        assert_eq!(r.weekdays, Vec::<u32>::new());
        assert_eq!(r.month_day, None);
        assert_eq!(r.state, "pending");
        assert_eq!((r.todo_id, r.note_id), (None, None));
    }

    #[tokio::test]
    async fn create_rejects_invalid_input() {
        let pool = crate::test_db().await;
        let todo = new_todo(&pool).await;
        let note = new_note(&pool).await;
        let try_create = |i: ReminderInput| {
            let pool = pool.clone();
            async move { inner_create_reminder(&i, at(NOW), &pool).await }
        };
        let later = "2026-11-09T09:00";
        assert_eq!(
            err_msg(try_create(input("  ", later)).await),
            "Text is required"
        );
        let long = "x".repeat(501);
        assert!(err_msg(try_create(input(&long, later)).await).contains("too long"));
        assert!(try_create(input(&"ü".repeat(500), later)).await.is_ok());
        assert_eq!(
            err_msg(try_create(input("x", "2026-11-08T11:59")).await),
            "Time is in the past"
        );
        assert_eq!(
            err_msg(try_create(input("x", NOW)).await),
            "Time is in the past"
        );
        assert_eq!(
            err_msg(try_create(input("x", "soon")).await),
            "Invalid time"
        );
        assert_eq!(
            err_msg(try_create(repeating("x", later, "weekly")).await),
            "Pick at least one weekday"
        );
        assert!(err_msg(try_create(repeating("x", later, "monthly")).await)
            .contains("day of the month"));
        let both = ReminderInput {
            todo_id: Some(todo),
            note_id: Some(note),
            ..input("x", later)
        };
        assert!(err_msg(try_create(both).await).contains("not both"));
        let missing_todo = ReminderInput {
            todo_id: Some(999),
            ..input("x", later)
        };
        assert_eq!(err_msg(try_create(missing_todo).await), "Todo not found");
        let missing_note = ReminderInput {
            note_id: Some(999),
            ..input("x", later)
        };
        assert_eq!(err_msg(try_create(missing_note).await), "Note not found");
        assert_eq!(inner_list_reminders(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn create_links_and_joins_todo_and_note() {
        let pool = crate::test_db().await;
        let todo = new_todo(&pool).await;
        let note = new_note(&pool).await;
        let r = create(
            ReminderInput {
                todo_id: Some(todo),
                ..input("t", "2026-11-09T09:00")
            },
            NOW,
            &pool,
        )
        .await;
        assert_eq!(r.todo_title.as_deref(), Some("Call Anna"));
        assert_eq!(r.todo_status.as_deref(), Some("not_started"));
        let r = create(
            ReminderInput {
                note_id: Some(note),
                ..input("n", "2026-11-09T09:00")
            },
            NOW,
            &pool,
        )
        .await;
        assert_eq!(r.note_title.as_deref(), Some("Invoice"));
        assert_eq!(r.todo_title, None);
    }

    #[tokio::test]
    async fn create_recurring_moves_onto_the_first_matching_day() {
        let pool = crate::test_db().await;
        // Typed on Saturday 08:00: `every weekday 9` → Monday 09:00.
        let r = create(
            repeating("Standup", "2026-11-07T09:00", "weekdays"),
            "2026-11-07T08:00",
            &pool,
        )
        .await;
        assert_eq!(r.remind_at, "2026-11-09T09:00:00.000");
        assert_eq!(r.time_of_day, "09:00");
        let weekly = ReminderInput {
            weekdays: vec![4, 1],
            ..repeating("Gym", "2026-11-10T18:00", "weekly")
        };
        let r = create(weekly, NOW, &pool).await;
        assert_eq!(r.weekdays, vec![1, 4]);
        assert_eq!(r.remind_at, "2026-11-12T18:00:00.000");
        let monthly = ReminderInput {
            month_day: Some(15),
            ..repeating("Rent", "2026-11-09T09:00", "monthly")
        };
        let r = create(monthly, NOW, &pool).await;
        assert_eq!(r.month_day, Some(15));
        assert_eq!(r.remind_at, "2026-11-15T09:00:00.000");
    }

    #[tokio::test]
    async fn create_recurring_with_a_past_start_rolls_forward() {
        let pool = crate::test_db().await;
        // Daily at 09:00 typed at 12:00 → tomorrow 09:00, anchored at 09:00.
        let r = create(repeating("d", "2026-11-08T09:00", "daily"), NOW, &pool).await;
        assert_eq!(r.remind_at, "2026-11-09T09:00:00.000");
        assert_eq!(r.time_of_day, "09:00");
        assert_eq!(r.state, "pending");
        // A start exactly now is not in the future either.
        let r = create(repeating("d", NOW, "daily"), NOW, &pool).await;
        assert_eq!(r.remind_at, "2026-11-09T12:00:00.000");
        // A start date last week, weekly Mon+Thu → the coming Monday.
        let weekly = ReminderInput {
            weekdays: vec![1, 4],
            ..repeating("w", "2026-11-01T18:00", "weekly")
        };
        let r = create(weekly, NOW, &pool).await;
        assert_eq!(r.remind_at, "2026-11-09T18:00:00.000");
        // Later today still counts: weekdays at 15:00 typed on Monday 12:00 → today.
        let r = create(
            repeating("s", "2026-11-09T15:00", "weekdays"),
            "2026-11-09T12:00",
            &pool,
        )
        .await;
        assert_eq!(r.remind_at, "2026-11-09T15:00:00.000");
        // Monthly on the 15th from a start in October → this month's 15th.
        let monthly = ReminderInput {
            month_day: Some(15),
            ..repeating("m", "2026-10-01T09:00", "monthly")
        };
        let r = create(monthly, NOW, &pool).await;
        assert_eq!(r.remind_at, "2026-11-15T09:00:00.000");
    }

    #[tokio::test]
    async fn edit_to_recurring_with_a_past_start_rolls_forward() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        // Today 08:00 has passed; as weekdays it moves to Monday 08:00.
        let e = inner_update_reminder(
            r.id,
            &repeating("x", "2026-11-08T08:00", "weekdays"),
            at(NOW),
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(e.remind_at, "2026-11-09T08:00:00.000");
        assert_eq!(e.time_of_day, "08:00");
        // One-offs still reject a past time.
        assert_eq!(
            err_msg(
                inner_update_reminder(r.id, &input("x", "2026-11-08T08:00"), at(NOW), &pool).await
            ),
            "Time is in the past"
        );
    }

    #[tokio::test]
    async fn snooze_moves_only_the_current_occurrence() {
        let pool = crate::test_db().await;
        let r = create(
            repeating("Standup", "2026-11-09T09:00", "daily"),
            NOW,
            &pool,
        )
        .await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        let s = inner_snooze_reminder(r.id, "2026-11-09T10:00", at("2026-11-09T09:05"), &pool)
            .await
            .unwrap();
        assert_eq!(s.state, "pending");
        assert_eq!(s.remind_at, "2026-11-09T10:00:00.000");
        assert_eq!(s.time_of_day, "09:00");
        assert_eq!(s.repeat, "daily");
        assert_eq!(
            err_msg(
                inner_snooze_reminder(r.id, "2026-11-09T09:00", at("2026-11-09T09:05"), &pool)
                    .await
            ),
            "Time is in the past"
        );
    }

    #[tokio::test]
    async fn done_one_off_is_kept_as_done() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        let d = complete(r.id, "2026-11-09T09:05", &pool).await;
        assert_eq!(d.state, "done");
        assert_eq!(d.remind_at, "2026-11-09T09:00:00.000");
        assert_eq!(inner_list_reminders(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn done_and_snooze_on_a_done_row_are_no_ops() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        let d = complete(r.id, NOW, &pool).await;
        assert_eq!(complete(r.id, NOW, &pool).await, d);
        let s = inner_snooze_reminder(r.id, "2026-11-10T09:00", at(NOW), &pool)
            .await
            .unwrap();
        assert_eq!(s, d);
        // Even a past `until` on a done row is a no-op success.
        assert!(
            inner_snooze_reminder(r.id, "2020-01-01T09:00", at(NOW), &pool)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn done_recurring_moves_to_the_next_occurrence() {
        let pool = crate::test_db().await;
        // Daily.
        let r = create(repeating("d", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        let d = complete(r.id, "2026-11-09T09:05", &pool).await;
        assert_eq!(
            (d.state.as_str(), d.remind_at.as_str()),
            ("pending", "2026-11-10T09:00:00.000")
        );
        // Weekdays across a weekend: Friday → Monday.
        let r = create(repeating("w", "2026-11-13T09:00", "weekdays"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-13T09:00", &pool).await;
        let d = complete(r.id, "2026-11-13T09:05", &pool).await;
        assert_eq!(d.remind_at, "2026-11-16T09:00:00.000");
        // Weekly Mon + Thu.
        let i = ReminderInput {
            weekdays: vec![1, 4],
            ..repeating("mt", "2026-11-09T09:00", "weekly")
        };
        let r = create(i, NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        let d = complete(r.id, "2026-11-09T09:05", &pool).await;
        assert_eq!(d.remind_at, "2026-11-12T09:00:00.000");
        let d = complete(r.id, "2026-11-12T09:05", &pool).await;
        assert_eq!(d.remind_at, "2026-11-16T09:00:00.000");
        // Monthly on the 15th.
        let i = ReminderInput {
            month_day: Some(15),
            ..repeating("m", "2026-11-15T09:00", "monthly")
        };
        let r = create(i, NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-15T09:00", &pool).await;
        let d = complete(r.id, "2026-11-15T09:05", &pool).await;
        assert_eq!(d.remind_at, "2026-12-15T09:00:00.000");
    }

    #[tokio::test]
    async fn done_monthly_31st_through_february_and_back() {
        let pool = crate::test_db().await;
        for (year, feb_end) in [(2027, "2027-02-28"), (2028, "2028-02-29")] {
            let i = ReminderInput {
                month_day: Some(31),
                ..repeating("m31", &format!("{year}-01-31T09:00"), "monthly")
            };
            let r = create(i, &format!("{year}-01-30T12:00"), &pool).await;
            assert_eq!(r.remind_at, format!("{year}-01-31T09:00:00.000"));
            let d = complete(r.id, &format!("{year}-01-31T09:05"), &pool).await;
            assert_eq!(d.remind_at, format!("{feb_end}T09:00:00.000"));
            let d = complete(r.id, &format!("{feb_end}T09:05"), &pool).await;
            assert_eq!(d.remind_at, format!("{year}-03-31T09:00:00.000"));
        }
    }

    #[tokio::test]
    async fn done_on_a_pending_recurring_skips_the_upcoming_one() {
        let pool = crate::test_db().await;
        let r = create(repeating("d", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        let d = complete(r.id, NOW, &pool).await;
        assert_eq!(
            (d.state.as_str(), d.remind_at.as_str()),
            ("pending", "2026-11-10T09:00:00.000")
        );
    }

    #[tokio::test]
    async fn snooze_then_done_returns_to_time_of_day() {
        let pool = crate::test_db().await;
        let r = create(repeating("d", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        inner_snooze_reminder(r.id, "2026-11-09T10:00", at("2026-11-09T09:05"), &pool)
            .await
            .unwrap();
        let d = complete(r.id, "2026-11-09T09:10", &pool).await;
        assert_eq!(d.remind_at, "2026-11-10T09:00:00.000");
        assert_eq!(d.time_of_day, "09:00");
    }

    #[tokio::test]
    async fn done_after_missed_occurrences_collapses_past_now() {
        let pool = crate::test_db().await;
        let r = create(repeating("d", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        // Asleep until the 12th at noon: one Done lands on the 13th.
        let d = complete(r.id, "2026-11-12T12:00", &pool).await;
        assert_eq!(d.remind_at, "2026-11-13T09:00:00.000");
    }

    #[tokio::test]
    async fn edit_time_on_fired_goes_back_to_pending() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        let now = at("2026-11-09T09:05");
        let e = inner_update_reminder(r.id, &input("x", "2026-11-09T15:00"), now, &pool)
            .await
            .unwrap();
        assert_eq!(e.state, "pending");
        assert_eq!(e.remind_at, "2026-11-09T15:00:00.000");
        assert_eq!(e.time_of_day, "15:00");
        assert_eq!(
            err_msg(inner_update_reminder(r.id, &input("x", "2026-11-09T09:01"), now, &pool).await),
            "Time is in the past"
        );
    }

    #[tokio::test]
    async fn edit_text_only_on_fired_keeps_time_and_state() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        set_state(r.id, "fired", "2026-11-09T09:00", &pool).await;
        // The stored (past) time comes back unchanged, in either format: no past check.
        let now = at("2026-11-09T12:00");
        for sent in ["2026-11-09T09:00", "2026-11-09T09:00:00.000"] {
            let e = inner_update_reminder(r.id, &input("  Renamed ", sent), now, &pool)
                .await
                .unwrap();
            assert_eq!(e.text, "Renamed");
            assert_eq!(e.state, "fired");
            assert_eq!(e.remind_at, "2026-11-09T09:00:00.000");
        }
    }

    #[tokio::test]
    async fn edit_rule_resets_time_of_day() {
        let pool = crate::test_db().await;
        let r = create(repeating("d", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        // Snoozed to 10:30; switching to weekly Thu at the shown time re-anchors at 10:30.
        inner_snooze_reminder(r.id, "2026-11-09T10:30", at(NOW), &pool)
            .await
            .unwrap();
        let i = ReminderInput {
            weekdays: vec![4],
            ..repeating("d", "2026-11-09T10:30", "weekly")
        };
        let e = inner_update_reminder(r.id, &i, at(NOW), &pool)
            .await
            .unwrap();
        assert_eq!(e.repeat, "weekly");
        assert_eq!(e.time_of_day, "10:30");
        assert_eq!(e.remind_at, "2026-11-12T10:30:00.000");
        assert_eq!(e.state, "pending");
    }

    #[tokio::test]
    async fn edit_done_row_is_rejected() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        complete(r.id, NOW, &pool).await;
        assert_eq!(
            err_msg(
                inner_update_reminder(r.id, &input("y", "2026-11-09T09:00"), at(NOW), &pool).await
            ),
            "Done reminders can't be edited"
        );
    }

    #[tokio::test]
    async fn edit_changes_or_removes_the_link() {
        let pool = crate::test_db().await;
        let todo = new_todo(&pool).await;
        let note = new_note(&pool).await;
        let linked = ReminderInput {
            todo_id: Some(todo),
            ..input("x", "2026-11-09T09:00")
        };
        let r = create(linked, NOW, &pool).await;
        let to_note = ReminderInput {
            note_id: Some(note),
            ..input("x", "2026-11-09T09:00")
        };
        let e = inner_update_reminder(r.id, &to_note, at(NOW), &pool)
            .await
            .unwrap();
        assert_eq!((e.todo_id, e.note_id), (None, Some(note)));
        let e = inner_update_reminder(r.id, &input("x", "2026-11-09T09:00"), at(NOW), &pool)
            .await
            .unwrap();
        assert_eq!((e.todo_id, e.note_id), (None, None));
        let bad = ReminderInput {
            todo_id: Some(999),
            ..input("x", "2026-11-09T09:00")
        };
        assert_eq!(
            err_msg(inner_update_reminder(r.id, &bad, at(NOW), &pool).await),
            "Todo not found"
        );
    }

    #[tokio::test]
    async fn missing_reminder_is_not_found_and_delete_removes() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        inner_delete_reminder(r.id, &pool).await.unwrap();
        let nf = "Reminder not found";
        assert_eq!(err_msg(inner_delete_reminder(r.id, &pool).await), nf);
        assert_eq!(
            err_msg(inner_complete_reminder(r.id, at(NOW), &pool).await),
            nf
        );
        assert_eq!(
            err_msg(inner_snooze_reminder(r.id, "2026-11-10T09:00", at(NOW), &pool).await),
            nf
        );
        assert_eq!(
            err_msg(
                inner_update_reminder(r.id, &input("x", "2026-11-10T09:00"), at(NOW), &pool).await
            ),
            nf
        );
    }

    #[tokio::test]
    async fn list_orders_open_by_time_then_latest_done() {
        let pool = crate::test_db().await;
        let late = create(input("late", "2026-11-10T09:00"), NOW, &pool).await;
        let early = create(input("early", "2026-11-09T09:00"), NOW, &pool).await;
        let fired = create(input("fired", "2026-11-09T10:00"), NOW, &pool).await;
        set_state(fired.id, "fired", "2026-11-08T11:00", &pool).await;
        let done = create(input("done", "2026-11-09T08:00"), NOW, &pool).await;
        complete(done.id, NOW, &pool).await;
        for i in 0..55 {
            let r = create(input(&format!("old {i}"), "2026-11-09T08:00"), NOW, &pool).await;
            complete(r.id, NOW, &pool).await;
        }
        let list = inner_list_reminders(&pool).await.unwrap();
        let open: Vec<i64> = list.iter().take(3).map(|r| r.id).collect();
        assert_eq!(open, vec![fired.id, early.id, late.id]);
        assert_eq!(list.len(), 3 + 50);
        assert!(list[3..].iter().all(|r| r.state == "done"));
        // Latest done first (same UpdatedAt here, so by id).
        assert_eq!(list[3].text, "old 54");
    }

    #[tokio::test]
    async fn deleting_a_todo_or_note_deletes_its_reminders() {
        let pool = crate::test_db().await;
        let todo = new_todo(&pool).await;
        let note = new_note(&pool).await;
        let on_todo = ReminderInput {
            todo_id: Some(todo),
            ..input("t", "2026-11-09T09:00")
        };
        let on_note = ReminderInput {
            note_id: Some(note),
            ..input("n", "2026-11-09T09:00")
        };
        create(on_todo, NOW, &pool).await;
        create(on_note, NOW, &pool).await;
        create(input("free", "2026-11-09T09:00"), NOW, &pool).await;
        crate::commands::todos::inner_delete_todo(todo, &pool)
            .await
            .unwrap();
        crate::commands::notes::inner_delete_note(note, &pool)
            .await
            .unwrap();
        let left: Vec<String> = inner_list_reminders(&pool)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.text)
            .collect();
        assert_eq!(left, vec!["free"]);
    }

    #[tokio::test]
    async fn moving_a_todo_to_done_completes_its_open_reminders() {
        use crate::commands::todos::inner_move_todo;
        let pool = crate::test_db().await;
        let todo = new_todo(&pool).await;
        let other = new_todo(&pool).await;
        let link = |i: ReminderInput, t: i64| ReminderInput {
            todo_id: Some(t),
            ..i
        };
        let pending = create(link(input("p", "2026-11-09T09:00"), todo), NOW, &pool).await;
        let fired = create(link(input("f", "2026-11-09T09:00"), todo), NOW, &pool).await;
        set_state(fired.id, "fired", "2026-11-08T11:00", &pool).await;
        let series = create(
            link(repeating("s", "2026-11-09T09:00", "daily"), todo),
            NOW,
            &pool,
        )
        .await;
        let elsewhere = create(link(input("o", "2026-11-09T09:00"), other), NOW, &pool).await;

        inner_move_todo(todo, "in_progress", 0, &pool)
            .await
            .unwrap();
        let state = |id| {
            let pool = pool.clone();
            async move { inner_get_reminder(id, &pool).await.unwrap().state }
        };
        assert_eq!(state(pending.id).await, "pending");

        inner_move_todo(todo, "done", 0, &pool).await.unwrap();
        for id in [pending.id, fired.id, series.id] {
            assert_eq!(state(id).await, "done");
        }
        assert_eq!(state(elsewhere.id).await, "pending");

        // Reopening doesn't restore them; reordering inside done leaves new ones alone.
        inner_move_todo(todo, "not_started", 0, &pool)
            .await
            .unwrap();
        assert_eq!(state(pending.id).await, "done");
        inner_move_todo(other, "done", 0, &pool).await.unwrap();
        let fresh = create(link(input("n", "2026-11-09T09:00"), other), NOW, &pool).await;
        inner_move_todo(other, "done", 1, &pool).await.unwrap();
        assert_eq!(state(fresh.id).await, "pending");
    }

    // ── Fire due ──────────────────────────────────────────────────────────────

    async fn fire(now: &str, pool: &Pool<Sqlite>) -> Vec<Reminder> {
        inner_fire_due(pool, at(now)).await.unwrap()
    }

    fn texts(rows: &[Reminder]) -> Vec<&str> {
        rows.iter().map(|r| r.text.as_str()).collect()
    }

    #[tokio::test]
    async fn fire_due_fires_due_pending_rows_once() {
        let pool = crate::test_db().await;
        create(input("due 1", "2026-11-09T09:00"), NOW, &pool).await;
        create(input("due 2", "2026-11-09T09:30"), NOW, &pool).await;
        create(input("future", "2026-11-09T11:00"), NOW, &pool).await;
        let done = create(input("done", "2026-11-09T08:00"), NOW, &pool).await;
        complete(done.id, NOW, &pool).await;
        let old = create(input("already fired", "2026-11-08T13:00"), NOW, &pool).await;
        set_state(old.id, "fired", "2026-11-08T13:00", &pool).await;

        let fired = fire("2026-11-09T10:00", &pool).await;
        assert_eq!(texts(&fired), vec!["due 1", "due 2"]);
        assert!(fired.iter().all(|r| r.state == "fired"));
        assert!(fire("2026-11-09T10:00", &pool).await.is_empty());
        assert_eq!(inner_count_fired(&pool).await.unwrap(), 3);
        // Exactly at RemindAt counts as due.
        let fired = fire("2026-11-09T11:00", &pool).await;
        assert_eq!(texts(&fired), vec!["future"]);
    }

    #[tokio::test]
    async fn fire_due_skips_a_row_snoozed_first() {
        let pool = crate::test_db().await;
        let r = create(input("x", "2026-11-09T09:00"), NOW, &pool).await;
        inner_snooze_reminder(r.id, "2026-11-09T09:20", at("2026-11-09T08:59"), &pool)
            .await
            .unwrap();
        assert!(fire("2026-11-09T09:01", &pool).await.is_empty());
        assert_eq!(texts(&fire("2026-11-09T09:20", &pool).await), vec!["x"]);
    }

    #[tokio::test]
    async fn fire_due_collapses_missed_recurring_occurrences() {
        let pool = crate::test_db().await;
        let r = create(repeating("daily", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        // App closed until the 12th at noon: one fire, at the 12th's occurrence.
        let fired = fire("2026-11-12T12:00", &pool).await;
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, r.id);
        assert_eq!(fired[0].remind_at, "2026-11-12T09:00:00.000");
        assert!(fire("2026-11-12T12:00:30", &pool).await.is_empty());
    }

    #[tokio::test]
    async fn fire_due_refires_an_ignored_recurring_row_at_its_next_occurrence() {
        let pool = crate::test_db().await;
        let r = create(repeating("daily", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        assert_eq!(fire("2026-11-09T09:00:10", &pool).await.len(), 1);
        assert!(fire("2026-11-09T15:00", &pool).await.is_empty());
        let again = fire("2026-11-10T09:00:10", &pool).await;
        assert_eq!(again.len(), 1);
        assert_eq!(again[0].id, r.id);
        assert_eq!(again[0].remind_at, "2026-11-10T09:00:00.000");
        assert_eq!(again[0].state, "fired");
        assert!(fire("2026-11-10T09:00:40", &pool).await.is_empty());
        // Replaced, not duplicated.
        assert_eq!(inner_list_reminders(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn fire_due_keeps_a_snoozed_recurring_occurrence() {
        let pool = crate::test_db().await;
        let r = create(repeating("daily", "2026-11-09T09:00", "daily"), NOW, &pool).await;
        fire("2026-11-09T09:00:10", &pool).await;
        inner_snooze_reminder(r.id, "2026-11-09T10:00", at("2026-11-09T09:01"), &pool)
            .await
            .unwrap();
        let fired = fire("2026-11-09T10:00:10", &pool).await;
        assert_eq!(fired[0].remind_at, "2026-11-09T10:00:00.000");
        assert_eq!(fired[0].time_of_day, "09:00");
    }

    #[tokio::test]
    async fn fire_due_reports_a_database_error() {
        let pool = crate::test_db().await;
        pool.close().await;
        assert!(inner_fire_due(&pool, at(NOW)).await.is_err());
    }
}
