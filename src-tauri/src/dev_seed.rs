//! Dev-only database seeding. Excluded from release builds via #[cfg(debug_assertions)].

#[cfg(debug_assertions)]
use chrono::{Datelike, Duration, NaiveDate, Utc, Weekday};
#[cfg(debug_assertions)]
use sqlx::{Pool, Sqlite};
#[cfg(debug_assertions)]
use tauri::State;

#[cfg(debug_assertions)]
use crate::AppError;

#[cfg(debug_assertions)]
fn seed_start_date(today: NaiveDate) -> NaiveDate {
    let mut year = today.year();
    let mut month = today.month() as i32 - 2;

    while month <= 0 {
        month += 12;
        year -= 1;
    }

    NaiveDate::from_ymd_opt(year, month as u32, 1)
        .expect("seed start date must be a valid month boundary")
}

#[cfg(debug_assertions)]
async fn seed_time_entry(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    issue_id: i64,
    date: NaiveDate,
    hours: f64,
    notes: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO time_entries (IssueId, Date, Hours, Notes) VALUES (?, ?, ?, ?)")
        .bind(issue_id)
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(hours)
        .bind(notes)
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) async fn seed_test_data_if_empty_with_anchor(
    pool: &Pool<Sqlite>,
    today: NaiveDate,
) -> Result<(), sqlx::Error> {
    let existing_customers: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
        .fetch_one(pool)
        .await?;

    if existing_customers.0 > 0 {
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    let northwind_customer_id: i64 =
        sqlx::query_scalar("INSERT INTO customers (Name, UseAsDefault) VALUES (?, ?) RETURNING id")
            .bind("Northwind Studio")
            .bind(1_i64)
            .fetch_one(&mut *tx)
            .await?;

    let blue_harbor_customer_id: i64 =
        sqlx::query_scalar("INSERT INTO customers (Name, UseAsDefault) VALUES (?, ?) RETURNING id")
            .bind("Blue Harbor Labs")
            .bind(0_i64)
            .fetch_one(&mut *tx)
            .await?;

    let desktop_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(northwind_customer_id)
    .bind("TimeTracker Desktop")
    .bind(1_i64)
    .fetch_one(&mut *tx)
    .await?;

    let marketing_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(northwind_customer_id)
    .bind("Northwind Marketing Site")
    .bind(0_i64)
    .fetch_one(&mut *tx)
    .await?;

    let operations_project_id: i64 = sqlx::query_scalar(
        "INSERT INTO projects (CustomerId, Name, UseAsDefault) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(blue_harbor_customer_id)
    .bind("Operations Dashboard")
    .bind(1_i64)
    .fetch_one(&mut *tx)
    .await?;

    let planning_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(desktop_project_id)
    .bind("101")
    .bind("Backlog triage and sprint planning")
    .fetch_one(&mut *tx)
    .await?;

    let reporting_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(desktop_project_id)
    .bind("204")
    .bind("Weekly and monthly reporting improvements")
    .fetch_one(&mut *tx)
    .await?;

    let settings_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(desktop_project_id)
    .bind("233")
    .bind("Settings polish and QA fixes")
    .fetch_one(&mut *tx)
    .await?;

    let landing_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(marketing_project_id)
    .bind("12")
    .bind("Landing page refresh")
    .fetch_one(&mut *tx)
    .await?;

    let content_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(marketing_project_id)
    .bind("28")
    .bind("Content updates and analytics review")
    .fetch_one(&mut *tx)
    .await?;

    let admin_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(operations_project_id)
    .bind("7")
    .bind("Admin workflow cleanup")
    .fetch_one(&mut *tx)
    .await?;

    let export_issue_id: i64 = sqlx::query_scalar(
        "INSERT INTO issues (ProjectId, ExternalId, Title) VALUES (?, ?, ?) RETURNING id",
    )
    .bind(operations_project_id)
    .bind("19")
    .bind("Billing export fixes")
    .fetch_one(&mut *tx)
    .await?;

    let start_date = seed_start_date(today);
    let total_days = (today - start_date).num_days();

    for day_offset in 0..=total_days {
        let date = start_date + Duration::days(day_offset);
        let week_index = day_offset / 7;

        match date.weekday() {
            Weekday::Mon => {
                seed_time_entry(
                    &mut tx,
                    planning_issue_id,
                    date,
                    2.5 + (week_index % 3) as f64 * 0.5,
                    "Sprint planning, backlog grooming, and team sync.",
                )
                .await?;
            }
            Weekday::Tue => {
                seed_time_entry(
                    &mut tx,
                    reporting_issue_id,
                    date,
                    3.5 + (week_index % 2) as f64 * 0.5,
                    "Worked through weekly report edge cases and review feedback.",
                )
                .await?;

                if week_index % 2 == 0 {
                    seed_time_entry(
                        &mut tx,
                        settings_issue_id,
                        date,
                        1.0,
                        "Polished settings UX and verified form behaviour.",
                    )
                    .await?;
                }
            }
            Weekday::Wed => {
                seed_time_entry(
                    &mut tx,
                    landing_issue_id,
                    date,
                    3.0 + (week_index % 3) as f64 * 0.5,
                    "Updated landing page sections, imagery, and copy layout.",
                )
                .await?;
            }
            Weekday::Thu => {
                seed_time_entry(
                    &mut tx,
                    admin_issue_id,
                    date,
                    4.0 + (week_index % 2) as f64 * 0.5,
                    "Refined admin workflow steps and handled follow-up fixes.",
                )
                .await?;

                if week_index % 3 == 1 {
                    seed_time_entry(
                        &mut tx,
                        content_issue_id,
                        date,
                        1.5,
                        "Reviewed content tweaks and analytics event coverage.",
                    )
                    .await?;
                }
            }
            Weekday::Fri => {
                seed_time_entry(
                    &mut tx,
                    export_issue_id,
                    date,
                    2.5 + (week_index % 4) as f64 * 0.5,
                    "Closed out export bugs, validation, and stakeholder notes.",
                )
                .await?;
            }
            Weekday::Sat | Weekday::Sun => {}
        }
    }

    tx.commit().await?;
    Ok(())
}

#[cfg(debug_assertions)]
pub async fn seed_test_data_if_empty(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    seed_test_data_if_empty_with_anchor(pool, Utc::now().date_naive()).await
}

/// Testable inner function — seeds demo data if DB is empty.
#[cfg(debug_assertions)]
pub async fn inner_seed_database(pool: &Pool<Sqlite>) -> Result<String, AppError> {
    seed_test_data_if_empty(pool).await?;
    Ok("Seed complete".to_string())
}

#[cfg(debug_assertions)]
#[tauri::command]
pub async fn seed_database(pool: State<'_, Pool<Sqlite>>) -> Result<String, AppError> {
    inner_seed_database(&pool).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    #[test]
    fn test_seed_start_date_covers_three_calendar_months() {
        let start = seed_start_date(NaiveDate::from_ymd_opt(2026, 4, 6).unwrap());
        assert_eq!(start.to_string(), "2026-02-01");
    }

    #[tokio::test]
    async fn test_seed_data_populates_empty_database() {
        let pool = crate::test_db().await;

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        let customer_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM customers")
            .fetch_one(&pool)
            .await
            .unwrap();
        let project_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM projects")
            .fetch_one(&pool)
            .await
            .unwrap();
        let issue_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM issues")
            .fetch_one(&pool)
            .await
            .unwrap();
        let entry_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM time_entries")
            .fetch_one(&pool)
            .await
            .unwrap();
        let defaults: Vec<(String, i64)> = sqlx::query_as(
            "SELECT Name, UseAsDefault FROM customers WHERE UseAsDefault = 1 ORDER BY Name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let seeded_range: (String, String) =
            sqlx::query_as("SELECT MIN(Date), MAX(Date) FROM time_entries")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(customer_count.0, 2, "expected two seeded customers");
        assert_eq!(project_count.0, 3, "expected three seeded projects");
        assert_eq!(
            issue_count.0, 7,
            "expected seeded issues across all projects"
        );
        assert_eq!(entry_count.0, 54, "expected the anchored seed volume");
        assert_eq!(
            defaults.len(),
            1,
            "exactly one default customer is expected"
        );
        assert_eq!(defaults[0].0, "Northwind Studio");
        assert_eq!(seeded_range.0, "2026-02-02");
        assert_eq!(seeded_range.1, "2026-04-06");
    }

    #[tokio::test]
    async fn test_seed_data_skips_non_empty_database() {
        let pool = crate::test_db().await;

        sqlx::query("INSERT INTO customers (Name, UseAsDefault) VALUES ('Existing Co', 0)")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        let customers: Vec<(String,)> = sqlx::query_as("SELECT Name FROM customers ORDER BY Name")
            .fetch_all(&pool)
            .await
            .unwrap();

        assert_eq!(customers, vec![("Existing Co".to_string(),)]);
    }

    #[tokio::test]
    async fn test_inner_seed_database_returns_ok() {
        let pool = crate::test_db().await;
        let result = inner_seed_database(&pool).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Seed complete");
    }
}
