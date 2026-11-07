//! Dev-only database seeding. Excluded from release builds via #[cfg(debug_assertions)].

#[cfg(debug_assertions)]
use chrono::{Datelike, Duration, Local, NaiveDate, Utc, Weekday};
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
async fn seed_trip(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    date: NaiveDate,
    from_place: &str,
    to_place: &str,
    km: Option<f64>,
    amount: Option<f64>,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO travel (Date, FromPlace, ToPlace, Km, Amount) VALUES (?, ?, ?, ?, ?)")
        .bind(date.format("%Y-%m-%d").to_string())
        .bind(from_place)
        .bind(to_place)
        .bind(km)
        .bind(amount)
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

    if existing_customers.0 == 0 {
        seed_core_data(pool, today).await?;
    }
    seed_notes_if_empty(pool).await
}

/// Customers, projects, issues, time entries, and trips.
#[cfg(debug_assertions)]
async fn seed_core_data(pool: &Pool<Sqlite>, today: NaiveDate) -> Result<(), sqlx::Error> {
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

    let planning_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Backlog triage and sprint planning")
            .fetch_one(&mut *tx)
            .await?;

    let reporting_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Weekly and monthly reporting improvements")
            .fetch_one(&mut *tx)
            .await?;

    let settings_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(desktop_project_id)
            .bind("Settings polish and QA fixes")
            .fetch_one(&mut *tx)
            .await?;

    let landing_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(marketing_project_id)
            .bind("Landing page refresh")
            .fetch_one(&mut *tx)
            .await?;

    let content_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(marketing_project_id)
            .bind("Content updates and analytics review")
            .fetch_one(&mut *tx)
            .await?;

    let admin_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(operations_project_id)
            .bind("Admin workflow cleanup")
            .fetch_one(&mut *tx)
            .await?;

    let export_issue_id: i64 =
        sqlx::query_scalar("INSERT INTO issues (ProjectId, Title) VALUES (?, ?) RETURNING id")
            .bind(operations_project_id)
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

                if week_index % 2 == 0 {
                    seed_trip(
                        &mut tx,
                        date,
                        "Malmö",
                        "Karlskrona",
                        Some(500.0),
                        Some(1300.0),
                    )
                    .await?;
                } else if week_index % 4 == 1 {
                    seed_trip(&mut tx, date, "Malmö", "Lund", Some(42.5), None).await?;
                }
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
const SEED_NOTE_WELCOME: &str = "# Welcome to Notes

Notes are **markdown** with a *live preview*: markers like `**` hide unless the cursor is on the line.

## Inline styles

**Bold**, *italic*, ~~strikethrough~~, `inline code`, and a [link](https://tauri.app) with its URL hidden.

> Blockquotes look like this.
> They can span lines.

---

### Lists

- Bullet one
- Bullet two
  - Nested bullet
1. First
2. Second

- [ ] Open task
- [x] Finished task

![Images stay as raw markdown](https://example.com/image.png)
";

#[cfg(debug_assertions)]
const SEED_NOTE_TABLES: &str = "Tables render as a grid until the cursor enters them.

| Task | Owner | Hours |
| :--- | :---: | ----: |
| **Release** notes | Åsa | 1.5 |
| `cargo test` run | Erik | 0.25 |
| ~~Old item~~ | *nobody* | 0 |
| Short row |

Rows with too few cells are padded.
";

#[cfg(debug_assertions)]
const SEED_NOTE_CODE: &str = "Setext heading
==============

Smaller setext heading
----------------------

```rust
fn main() {
    println!(\"Hello from a fenced block\");
}
```

```bash
npm run vite:build
```
";

#[cfg(debug_assertions)]
const SEED_NOTE_MEETING: &str = "## Weekly sync

- Demo the *Notes* page
- [ ] Follow up on billing export
- [x] Ship the Today overview

> Decision: keep notes local-first.
";

#[cfg(debug_assertions)]
const SEED_NOTE_ARCHIVED: &str = "# Old roadmap

Lives in **Projects / Archive**, a nested folder.
";

/// Creates `Projects / Archive` when there are no folders yet; otherwise reuses an existing
/// seeded `Archive` (so reseeding after deleting every note doesn't duplicate folders).
/// Returns the Archive folder, or `None` (seed at the root) when other folders exist.
#[cfg(debug_assertions)]
async fn seed_folders(tx: &mut sqlx::Transaction<'_, Sqlite>) -> Result<Option<i64>, sqlx::Error> {
    let existing_folders: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM folders")
        .fetch_one(&mut **tx)
        .await?;
    if existing_folders.0 > 0 {
        return sqlx::query_scalar(
            "SELECT c.id FROM folders c JOIN folders p ON p.id = c.ParentId \
             WHERE c.Name = 'Archive' AND p.Name = 'Projects' AND p.ParentId IS NULL \
             ORDER BY c.id LIMIT 1",
        )
        .fetch_optional(&mut **tx)
        .await;
    }

    let projects_folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (ParentId, Name, Position) VALUES (NULL, ?, 0) RETURNING id",
    )
    .bind("Projects")
    .fetch_one(&mut **tx)
    .await?;
    let archive_folder_id: i64 = sqlx::query_scalar(
        "INSERT INTO folders (ParentId, Name, Position) VALUES (?, ?, 0) RETURNING id",
    )
    .bind(projects_folder_id)
    .bind("Archive")
    .fetch_one(&mut **tx)
    .await?;
    Ok(Some(archive_folder_id))
}

/// Sample notes covering every live-preview element, plus a folder with a nested child.
/// Runs whenever the `notes` table is empty, independent of the customer seed.
#[cfg(debug_assertions)]
async fn seed_notes_if_empty(pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
    let existing_notes: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes")
        .fetch_one(pool)
        .await?;
    if existing_notes.0 > 0 {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let archive_folder_id = seed_folders(&mut tx).await?;

    let mut long_note =
        String::from("# Long note\n\nScroll down: the table at the end still renders.\n\n");
    for i in 1..=300 {
        long_note.push_str(&format!("Line {i} of filler text.\n"));
    }
    long_note.push_str("\n| Far | Down |\n| --- | --- |\n| still | rendered |\n");

    let now = Local::now();
    let notes: [(Option<i64>, &str, &str, bool); 6] = [
        (None, "Welcome to Notes", SEED_NOTE_WELCOME, true),
        (None, "Markdown tables", SEED_NOTE_TABLES, false),
        (None, "Code and setext headings", SEED_NOTE_CODE, false),
        (None, "Weekly sync", SEED_NOTE_MEETING, false),
        (None, "Long note", &long_note, false),
        (archive_folder_id, "Old roadmap", SEED_NOTE_ARCHIVED, false),
    ];
    for (age_hours, (folder_id, title, body, pinned)) in (0_i64..).zip(notes) {
        let ts = (now - Duration::hours(age_hours * 7))
            .format("%Y-%m-%dT%H:%M:%S%.3f")
            .to_string();
        sqlx::query(
            "INSERT INTO notes (FolderId, Title, Body, Pinned, CreatedAt, UpdatedAt) \
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(folder_id)
        .bind(title)
        .bind(body)
        .bind(pinned)
        .bind(&ts)
        .bind(&ts)
        .execute(&mut *tx)
        .await?;
    }
    crate::commands::notes::rebuild_index(&mut tx).await?;

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
pub async fn seed_database(
    pool: State<'_, Pool<Sqlite>>,
    semantic: State<'_, crate::semantic::SemanticState>,
) -> Result<String, AppError> {
    let message = inner_seed_database(&pool).await?;
    semantic.enqueue_all(pool.inner()).await?;
    Ok(message)
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
        let trip_count: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM travel")
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
        assert_eq!(trip_count.0, 7, "expected seeded trips");
        assert_eq!(
            defaults.len(),
            1,
            "exactly one default customer is expected"
        );
        assert_eq!(defaults[0].0, "Northwind Studio");
        assert_eq!(seeded_range.0, "2026-02-02");
        assert_eq!(seeded_range.1, "2026-04-06");
        assert_eq!(count(&pool, "notes").await, 6, "expected seeded notes");
        assert_eq!(count(&pool, "folders").await, 2, "expected seeded folders");
        let nested: (String, String, String) = sqlx::query_as(
            "SELECT p.Name, c.Name, n.Title FROM notes n
             JOIN folders c ON c.id = n.FolderId
             JOIN folders p ON p.id = c.ParentId",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            nested,
            ("Projects".into(), "Archive".into(), "Old roadmap".into())
        );
        let pinned: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM notes WHERE Pinned = 1")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(pinned.0, 1, "expected one pinned note");
    }

    async fn count(pool: &Pool<Sqlite>, table: &str) -> i64 {
        let row: (i64,) = sqlx::query_as(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(pool)
            .await
            .unwrap();
        row.0
    }

    #[tokio::test]
    async fn test_seed_notes_skips_when_notes_exist() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO notes (Title, CreatedAt, UpdatedAt) VALUES ('Mine', 'x', 'x')")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 1);
        assert_eq!(count(&pool, "folders").await, 0);
        assert_eq!(
            count(&pool, "customers").await,
            2,
            "customers seed independently"
        );
    }

    #[tokio::test]
    async fn test_reseeding_notes_reuses_seeded_folders() {
        let pool = crate::test_db().await;
        let anchor = NaiveDate::from_ymd_opt(2026, 4, 6).unwrap();
        seed_test_data_if_empty_with_anchor(&pool, anchor)
            .await
            .unwrap();
        sqlx::query("DELETE FROM notes")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, anchor)
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 6);
        assert_eq!(
            count(&pool, "folders").await,
            2,
            "folders are not duplicated"
        );
        let nested: (String, String) = sqlx::query_as(
            "SELECT c.Name, n.Title FROM notes n
             JOIN folders c ON c.id = n.FolderId
             JOIN folders p ON p.id = c.ParentId AND p.Name = 'Projects'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(nested, ("Archive".into(), "Old roadmap".into()));
    }

    #[tokio::test]
    async fn test_reseeding_notes_with_other_folders_seeds_at_root() {
        let pool = crate::test_db().await;
        sqlx::query("INSERT INTO folders (ParentId, Name, Position) VALUES (NULL, 'Mine', 0)")
            .execute(&pool)
            .await
            .unwrap();

        seed_test_data_if_empty_with_anchor(&pool, NaiveDate::from_ymd_opt(2026, 4, 6).unwrap())
            .await
            .unwrap();

        assert_eq!(count(&pool, "notes").await, 6);
        assert_eq!(count(&pool, "folders").await, 1, "no folders added");
        let in_folders: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM notes WHERE FolderId IS NOT NULL")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(in_folders.0, 0, "seeded notes go to the root");
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
        assert_eq!(
            count(&pool, "notes").await,
            6,
            "notes seed even when customers exist"
        );
        assert_eq!(count(&pool, "folders").await, 2);
    }

    #[tokio::test]
    async fn test_inner_seed_database_returns_ok() {
        let pool = crate::test_db().await;
        let result = inner_seed_database(&pool).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "Seed complete");
    }
}
