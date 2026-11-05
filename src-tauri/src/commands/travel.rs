use crate::AppError;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, Pool, Sqlite};
use tauri::State;

use super::reports::month_bounds;

/// A trip. `km` and `amount` are null when not entered.
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct TravelRow {
    pub id: i64,
    pub date: String, // YYYY-MM-DD
    pub from_place: String,
    pub to_place: String,
    pub km: Option<f64>,
    pub amount: Option<f64>,
}

const SELECT_TRAVEL: &str = "SELECT id,
        Date      AS date,
        FromPlace AS from_place,
        ToPlace   AS to_place,
        Km        AS km,
        Amount    AS amount
 FROM travel";

/// Validated trip fields, ready to bind.
struct TravelInput {
    date: String,
    from_place: String,
    to_place: String,
    km: Option<f64>,
    amount: Option<f64>,
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn custom(msg: &str) -> AppError {
    AppError::Custom(msg.to_string())
}

fn clean_place(place: &str, missing: &str) -> Result<String, AppError> {
    let p = place.trim();
    if p.is_empty() {
        return Err(custom(missing));
    }
    Ok(p.to_string())
}

fn clean_input(
    date: &str,
    from_place: &str,
    to_place: &str,
    km: Option<f64>,
    amount: Option<f64>,
) -> Result<TravelInput, AppError> {
    let date = date.trim();
    if date.is_empty() {
        return Err(custom("Date is required"));
    }
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").map_err(|_| custom("Invalid date"))?;
    let from_place = clean_place(from_place, "From is required")?;
    let to_place = clean_place(to_place, "To is required")?;
    if km.is_some_and(|k| !k.is_finite() || k <= 0.0) {
        return Err(custom("Km must be greater than 0"));
    }
    if amount.is_some_and(|a| !a.is_finite() || a < 0.0) {
        return Err(custom("Amount cannot be negative"));
    }
    Ok(TravelInput {
        date: date.to_string(),
        from_place,
        to_place,
        km,
        amount,
    })
}

fn not_found() -> AppError {
    custom("Trip not found")
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// Trips in the given month, ordered by date then id.
pub async fn inner_list_travel(
    year: i32,
    month: u32,
    pool: &Pool<Sqlite>,
) -> Result<Vec<TravelRow>, AppError> {
    if !(1..=12).contains(&month) {
        return Err(custom("Invalid month"));
    }
    let (start, end) = month_bounds(year, month);
    let sql = format!("{SELECT_TRAVEL} WHERE Date >= ? AND Date < ? ORDER BY Date, id");
    Ok(sqlx::query_as::<_, TravelRow>(&sql)
        .bind(start)
        .bind(end)
        .fetch_all(pool)
        .await?)
}

async fn get_travel(id: i64, pool: &Pool<Sqlite>) -> Result<TravelRow, AppError> {
    let sql = format!("{SELECT_TRAVEL} WHERE id = ?");
    sqlx::query_as::<_, TravelRow>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(not_found)
}

pub async fn inner_create_travel(
    date: &str,
    from_place: &str,
    to_place: &str,
    km: Option<f64>,
    amount: Option<f64>,
    pool: &Pool<Sqlite>,
) -> Result<TravelRow, AppError> {
    let t = clean_input(date, from_place, to_place, km, amount)?;
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO travel (Date, FromPlace, ToPlace, Km, Amount) VALUES (?, ?, ?, ?, ?) \
         RETURNING id",
    )
    .bind(&t.date)
    .bind(&t.from_place)
    .bind(&t.to_place)
    .bind(t.km)
    .bind(t.amount)
    .fetch_one(pool)
    .await?;
    get_travel(row.0, pool).await
}

pub async fn inner_update_travel(
    id: i64,
    date: &str,
    from_place: &str,
    to_place: &str,
    km: Option<f64>,
    amount: Option<f64>,
    pool: &Pool<Sqlite>,
) -> Result<TravelRow, AppError> {
    let t = clean_input(date, from_place, to_place, km, amount)?;
    let result = sqlx::query(
        "UPDATE travel SET Date = ?, FromPlace = ?, ToPlace = ?, Km = ?, Amount = ? WHERE id = ?",
    )
    .bind(&t.date)
    .bind(&t.from_place)
    .bind(&t.to_place)
    .bind(t.km)
    .bind(t.amount)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    get_travel(id, pool).await
}

pub async fn inner_delete_travel(id: i64, pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let result = sqlx::query("DELETE FROM travel WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    if result.rows_affected() == 0 {
        return Err(not_found());
    }
    Ok(())
}

/// Distinct from/to places across all trips, for autocomplete.
pub async fn inner_list_travel_places(pool: &Pool<Sqlite>) -> Result<Vec<String>, AppError> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT FromPlace FROM travel UNION SELECT ToPlace FROM travel ORDER BY 1")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().map(|(p,)| p).collect())
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn list_travel(
    year: i32,
    month: u32,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<Vec<TravelRow>, AppError> {
    inner_list_travel(year, month, pool.inner()).await
}

#[tauri::command]
pub async fn create_travel(
    date: String,
    from_place: String,
    to_place: String,
    km: Option<f64>,
    amount: Option<f64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TravelRow, AppError> {
    inner_create_travel(&date, &from_place, &to_place, km, amount, pool.inner()).await
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_travel(
    id: i64,
    date: String,
    from_place: String,
    to_place: String,
    km: Option<f64>,
    amount: Option<f64>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<TravelRow, AppError> {
    inner_update_travel(id, &date, &from_place, &to_place, km, amount, pool.inner()).await
}

#[tauri::command]
pub async fn delete_travel(id: i64, pool: State<'_, Pool<Sqlite>>) -> Result<(), AppError> {
    inner_delete_travel(id, pool.inner()).await
}

#[tauri::command]
pub async fn list_travel_places(pool: State<'_, Pool<Sqlite>>) -> Result<Vec<String>, AppError> {
    inner_list_travel_places(pool.inner()).await
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn err_msg<T: std::fmt::Debug>(r: Result<T, AppError>) -> String {
        r.unwrap_err().to_string()
    }

    async fn trip(date: &str, from: &str, to: &str, pool: &Pool<Sqlite>) -> TravelRow {
        inner_create_travel(date, from, to, Some(500.0), Some(1300.0), pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn create_trims_places_and_keeps_unicode() {
        let pool = crate::test_db().await;
        let t = inner_create_travel(
            " 2026-10-05 ",
            "  Malmö ",
            " Karlskrona ",
            Some(12.5),
            Some(99.5),
            &pool,
        )
        .await
        .unwrap();
        assert_eq!(t.date, "2026-10-05");
        assert_eq!(t.from_place, "Malmö");
        assert_eq!(t.to_place, "Karlskrona");
        assert_eq!(t.km, Some(12.5));
        assert_eq!(t.amount, Some(99.5));
    }

    #[tokio::test]
    async fn km_and_amount_are_optional_and_stored_as_null() {
        let pool = crate::test_db().await;
        let t = inner_create_travel("2026-10-05", "A", "B", None, None, &pool)
            .await
            .unwrap();
        assert_eq!(t.km, None);
        assert_eq!(t.amount, None);
        let zero = inner_create_travel("2026-10-05", "A", "B", None, Some(0.0), &pool)
            .await
            .unwrap();
        assert_eq!(zero.amount, Some(0.0));
    }

    #[tokio::test]
    async fn validation_rejects_bad_input() {
        let pool = crate::test_db().await;
        let cases: [(&str, &str, &str, Option<f64>, Option<f64>, &str); 7] = [
            ("", "A", "B", None, None, "Date is required"),
            ("2026-02-30", "A", "B", None, None, "Invalid date"),
            ("2026-10-05", "  ", "B", None, None, "From is required"),
            ("2026-10-05", "A", "", None, None, "To is required"),
            (
                "2026-10-05",
                "A",
                "B",
                Some(0.0),
                None,
                "Km must be greater than 0",
            ),
            (
                "2026-10-05",
                "A",
                "B",
                Some(-1.0),
                None,
                "Km must be greater than 0",
            ),
            (
                "2026-10-05",
                "A",
                "B",
                None,
                Some(-0.5),
                "Amount cannot be negative",
            ),
        ];
        for (date, from, to, km, amount, msg) in cases {
            let r = inner_create_travel(date, from, to, km, amount, &pool).await;
            assert_eq!(err_msg(r), msg);
        }
        assert!(inner_list_travel(2026, 10, &pool).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn list_returns_only_the_month_in_date_order() {
        let pool = crate::test_db().await;
        trip("2026-10-31", "Last", "Day", &pool).await;
        trip("2026-10-01", "First", "Day", &pool).await;
        trip("2026-09-30", "Prev", "Month", &pool).await;
        trip("2026-11-01", "Next", "Month", &pool).await;
        let rows = inner_list_travel(2026, 10, &pool).await.unwrap();
        let froms: Vec<_> = rows.iter().map(|t| t.from_place.as_str()).collect();
        assert_eq!(froms, ["First", "Last"]);
    }

    #[tokio::test]
    async fn list_handles_december() {
        let pool = crate::test_db().await;
        trip("2026-12-31", "Dec", "X", &pool).await;
        trip("2027-01-01", "Jan", "X", &pool).await;
        let dec = inner_list_travel(2026, 12, &pool).await.unwrap();
        assert_eq!(dec.len(), 1);
        assert_eq!(dec[0].from_place, "Dec");
        assert_eq!(
            err_msg(inner_list_travel(2026, 13, &pool).await),
            "Invalid month"
        );
    }

    #[tokio::test]
    async fn update_replaces_fields_and_can_clear_km_amount() {
        let pool = crate::test_db().await;
        let t = trip("2026-10-05", "Malmö", "Karlskrona", &pool).await;
        let u = inner_update_travel(t.id, "2026-10-06", " Lund ", "Ystad", None, None, &pool)
            .await
            .unwrap();
        assert_eq!(u.date, "2026-10-06");
        assert_eq!(u.from_place, "Lund");
        assert_eq!(u.to_place, "Ystad");
        assert_eq!(u.km, None);
        assert_eq!(u.amount, None);
    }

    #[tokio::test]
    async fn update_validates_and_reports_missing_trip() {
        let pool = crate::test_db().await;
        let t = trip("2026-10-05", "A", "B", &pool).await;
        let r = inner_update_travel(t.id, "2026-10-05", "", "B", None, None, &pool).await;
        assert_eq!(err_msg(r), "From is required");
        let r = inner_update_travel(999, "2026-10-05", "A", "B", None, None, &pool).await;
        assert_eq!(err_msg(r), "Trip not found");
    }

    #[tokio::test]
    async fn delete_removes_trip_and_reports_missing() {
        let pool = crate::test_db().await;
        let t = trip("2026-10-05", "A", "B", &pool).await;
        inner_delete_travel(t.id, &pool).await.unwrap();
        assert!(inner_list_travel(2026, 10, &pool).await.unwrap().is_empty());
        assert_eq!(
            err_msg(inner_delete_travel(t.id, &pool).await),
            "Trip not found"
        );
    }

    #[tokio::test]
    async fn places_are_distinct_and_sorted() {
        let pool = crate::test_db().await;
        trip("2026-10-05", "Malmö", "Karlskrona", &pool).await;
        trip("2026-10-10", "Karlskrona", "Malmö", &pool).await;
        trip("2026-10-12", "Malmö", "Lund", &pool).await;
        let places = inner_list_travel_places(&pool).await.unwrap();
        assert_eq!(places, ["Karlskrona", "Lund", "Malmö"]);
    }
}
