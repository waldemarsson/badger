use crate::AppError;
use chrono::{Datelike, Duration, NaiveDate, Weekday};
use serde::Serialize;
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

// ── DB-only intermediate struct ───────────────────────────────────────────────

#[derive(Debug, FromRow)]
struct WeeklyRawRow {
    customer_id: i64,
    customer_name: String,
    project_id: i64,
    project_name: String,
    issue_id: i64,
    issue_title: String,
    mon: f64,
    tue: f64,
    wed: f64,
    thu: f64,
    fri: f64,
    sat: f64,
    sun: f64,
    total: f64,
}

// ── Public output structs ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct WeeklyReportRow {
    pub customer_id: i64,
    pub customer_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub issue_id: i64,
    pub issue_title: String,
    pub mon: f64,
    pub tue: f64,
    pub wed: f64,
    pub thu: f64,
    pub fri: f64,
    pub sat: f64,
    pub sun: f64,
    pub total: f64,
    pub adjacent_days: Vec<String>, // day names ("mon".."sun") outside Monday's calendar month
}

#[derive(Debug, Clone, Serialize, FromRow)]
pub struct MonthlyReportRow {
    pub customer_id: i64,
    pub customer_name: String,
    pub project_id: i64,
    pub project_name: String,
    pub issue_id: i64,
    pub issue_title: String,
    pub hours: f64,
}

// ── Helper functions ──────────────────────────────────────────────────────────

/// Returns (Monday, Sunday) NaiveDate for a given ISO year+week.
/// Uses chrono's from_isoywd_opt which correctly handles year-spanning weeks
/// (e.g. ISO week 1 of 2026 starts Mon 2025-12-29).
fn iso_week_bounds(year: i32, week: u32) -> (NaiveDate, NaiveDate) {
    let monday = NaiveDate::from_isoywd_opt(year, week, Weekday::Mon)
        .expect("invalid ISO year/week combination");
    let sunday = monday + Duration::days(6);
    (monday, sunday)
}

/// Returns weekday names ("mon".."sun") that fall outside Monday's calendar month.
/// Frontend renders "—" for these days instead of a zero or blank.
fn adjacent_days_for_week(monday: NaiveDate) -> Vec<String> {
    let primary_month = monday.month();
    ["mon", "tue", "wed", "thu", "fri", "sat", "sun"]
        .iter()
        .enumerate()
        .filter_map(|(i, name)| {
            let day = monday + Duration::days(i as i64);
            if day.month() != primary_month {
                Some(name.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Returns ("YYYY-MM-01", "YYYY-MM-01" of next month) for use in WHERE te.Date >= ? AND te.Date < ?
fn month_bounds(year: i32, month: u32) -> (String, String) {
    let start = format!("{:04}-{:02}-01", year, month);
    let (ny, nm) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let end = format!("{:04}-{:02}-01", ny, nm);
    (start, end)
}

// ── Inner async functions ─────────────────────────────────────────────────────

pub async fn inner_weekly_report(
    year: i32,
    week: u32,
    pool: &Pool<Sqlite>,
) -> Result<Vec<WeeklyReportRow>, AppError> {
    let (monday, sunday) = iso_week_bounds(year, week);
    let fmt = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let mon_s = fmt(monday);
    let tue_s = fmt(monday + Duration::days(1));
    let wed_s = fmt(monday + Duration::days(2));
    let thu_s = fmt(monday + Duration::days(3));
    let fri_s = fmt(monday + Duration::days(4));
    let sat_s = fmt(monday + Duration::days(5));
    let sun_s = fmt(sunday);
    let adjacent = adjacent_days_for_week(monday);

    let raw_rows = sqlx::query_as::<_, WeeklyRawRow>(
        "SELECT c.id      AS customer_id,
                c.Name    AS customer_name,
                p.id      AS project_id,
                p.Name    AS project_name,
                i.id      AS issue_id,
                i.Title   AS issue_title,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS mon,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS tue,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS wed,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS thu,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS fri,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS sat,
                COALESCE(SUM(CASE WHEN te.Date = ? THEN te.Hours END), 0.0) AS sun,
                SUM(te.Hours)                                                  AS total
         FROM   time_entries te
         JOIN   issues    i ON i.id  = te.IssueId
         JOIN   projects  p ON p.id  = i.ProjectId
         JOIN   customers c ON c.id  = p.CustomerId
         WHERE  te.Date >= ? AND te.Date <= ?
         GROUP  BY i.id
         ORDER  BY c.Name, p.Name, i.Title",
    )
    .bind(&mon_s)
    .bind(&tue_s)
    .bind(&wed_s)
    .bind(&thu_s)
    .bind(&fri_s)
    .bind(&sat_s)
    .bind(&sun_s)
    .bind(&mon_s)
    .bind(&sun_s)
    .fetch_all(pool)
    .await?;

    let rows = raw_rows
        .into_iter()
        .map(|r| WeeklyReportRow {
            customer_id: r.customer_id,
            customer_name: r.customer_name,
            project_id: r.project_id,
            project_name: r.project_name,
            issue_id: r.issue_id,
            issue_title: r.issue_title,
            mon: r.mon,
            tue: r.tue,
            wed: r.wed,
            thu: r.thu,
            fri: r.fri,
            sat: r.sat,
            sun: r.sun,
            total: r.total,
            adjacent_days: adjacent.clone(),
        })
        .collect();

    Ok(rows)
}

pub async fn inner_monthly_report(
    year: i32,
    month: u32,
    pool: &Pool<Sqlite>,
) -> Result<Vec<MonthlyReportRow>, AppError> {
    let (start, end) = month_bounds(year, month);

    let rows = sqlx::query_as::<_, MonthlyReportRow>(
        "SELECT c.id      AS customer_id,
                c.Name    AS customer_name,
                p.id      AS project_id,
                p.Name    AS project_name,
                i.id      AS issue_id,
                i.Title   AS issue_title,
                SUM(te.Hours) AS hours
         FROM   time_entries te
         JOIN   issues   i ON i.id   = te.IssueId
         JOIN   projects p ON p.id   = i.ProjectId
         JOIN   customers c ON c.id  = p.CustomerId
         WHERE  te.Date >= ? AND te.Date < ?
         GROUP  BY i.id
         ORDER  BY c.Name, p.Name, i.Title",
    )
    .bind(&start)
    .bind(&end)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn weekly_report(
    year: i32,
    week: u32,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<WeeklyReportRow>, AppError> {
    inner_weekly_report(year, week, pool.inner()).await
}

#[tauri::command]
pub async fn monthly_report(
    year: i32,
    month: u32,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<MonthlyReportRow>, AppError> {
    inner_monthly_report(year, month, pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::customers::inner_create_customer;
    use crate::commands::projects::inner_create_project;
    use crate::commands::time_entries::inner_create_time_entry;

    async fn setup(pool: &Pool<Sqlite>) -> (i64, i64) {
        let c = inner_create_customer("Acme Corp", pool).await.unwrap();
        let p = inner_create_project(c.id, "Website", pool).await.unwrap();
        (c.id, p.id)
    }

    // --- ISO week boundary tests (pure logic, no DB needed) ---

    #[tokio::test]
    async fn test_iso_week_bounds_mid_year() {
        let (mon, sun) = iso_week_bounds(2026, 15);
        assert_eq!(mon.to_string(), "2026-04-06");
        assert_eq!(sun.to_string(), "2026-04-12");
    }

    #[tokio::test]
    async fn test_iso_week_bounds_year_start() {
        // ISO week 1 of 2026 starts in the previous year (Dec 29, 2025)
        let (mon, sun) = iso_week_bounds(2026, 1);
        assert_eq!(mon.to_string(), "2025-12-29");
        assert_eq!(sun.to_string(), "2026-01-04");
    }

    #[tokio::test]
    async fn test_adjacent_days_week1_2026() {
        // Monday is Dec 29 → primary month = December
        // Thu Jan 1, Fri Jan 2, Sat Jan 3, Sun Jan 4 are in January → adjacent
        let monday = NaiveDate::from_ymd_opt(2025, 12, 29).unwrap();
        let adj = adjacent_days_for_week(monday);
        assert_eq!(adj, vec!["thu", "fri", "sat", "sun"]);
    }

    // --- Aggregation tests (require DB) ---

    #[tokio::test]
    async fn test_weekly_report_aggregation() {
        let pool = crate::test_db().await;
        let (_, project_id) = setup(&pool).await;

        // Insert entries on Monday (2026-04-06) and Wednesday (2026-04-08) of week 15
        inner_create_time_entry(project_id, "Task Alpha", "2026-04-06", 2.5, None, &pool)
            .await
            .unwrap();
        inner_create_time_entry(project_id, "Task Alpha", "2026-04-08", 1.0, None, &pool)
            .await
            .unwrap();

        let rows = inner_weekly_report(2026, 15, &pool).await.unwrap();
        assert_eq!(rows.len(), 1, "one issue row expected");
        let row = &rows[0];
        assert_eq!(row.mon, 2.5, "Monday hours");
        assert_eq!(row.wed, 1.0, "Wednesday hours");
        assert_eq!(row.tue, 0.0, "Tuesday must be 0 (no entry)");
        assert_eq!(row.total, 3.5, "total must be sum of all days");
        // No adjacent days — week 15 falls entirely within April
        assert!(row.adjacent_days.is_empty());
    }

    #[tokio::test]
    async fn test_monthly_report_aggregation() {
        let pool = crate::test_db().await;
        let (customer_id, project_id) = setup(&pool).await;

        inner_create_time_entry(project_id, "Task Beta", "2026-04-06", 2.0, None, &pool)
            .await
            .unwrap();
        inner_create_time_entry(project_id, "Task Beta", "2026-04-20", 3.0, None, &pool)
            .await
            .unwrap();
        // Entry in different month — must be excluded
        inner_create_time_entry(project_id, "Task Beta", "2026-05-01", 10.0, None, &pool)
            .await
            .unwrap();

        let rows = inner_monthly_report(2026, 4, &pool).await.unwrap();
        assert_eq!(rows.len(), 1, "one issue row for April");
        let row = &rows[0];
        assert_eq!(row.hours, 5.0, "sum of April entries only");
        assert_eq!(row.customer_id, customer_id);
        assert_eq!(row.project_id, project_id);
        assert_eq!(row.issue_title, "Task Beta");
    }
}
