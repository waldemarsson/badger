//! Recurrence rules and naive local wall-clock time, shared by tasks (repeat) and their
//! reminders.
//!
//! Times are stored as `%Y-%m-%dT%H:%M:%S%.3f` local strings, so string comparison orders
//! them and "09:00" stays 09:00 across DST and time-zone changes. A repeating series keeps
//! its `HH:MM` anchor; every occurrence returns to it, moved out of a DST gap when needed.

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Timelike};

use crate::AppError;

fn custom(msg: &str) -> AppError {
    AppError::Custom(msg.to_string())
}

/// Storage format of every timestamp (tasks, reminders and notes alike).
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
