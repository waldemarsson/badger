//! Menu-bar (tray) item: the app icon alone (no title), with a native menu showing today's
//! logged hours, the tasks needing attention (overdue, due today, or with a sent reminder,
//! like Today's Tasks card) and quick actions.
//!
//! [`nudge`] asks [`run`] to rebuild the menu from [`TrayModel`]: on every scheduler tick
//! (so the date rolls over), after task writes (`task_scheduler::tasks_refreshed`) and after
//! time entry writes. An unchanged model is not rebuilt, so an open menu is not disturbed. Actions the frontend
//! performs (new task, log time) go out as [`ACTION_EVENT`] after the window is shown.
use std::sync::Mutex;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

use crate::recurrence::local_now;
use crate::AppError;

const TRAY_ID: &str = "badger-tray";
/// Frontend action request: `"new-task"`, `"log-time"` or `"palette"`.
pub const ACTION_EVENT: &str = "app-action";
/// Tasks listed in the menu; the rest are summarised in a "more" item.
const MAX_TASKS: usize = 5;
/// Longest task title shown (characters) before an ellipsis.
const MAX_TITLE_CHARS: usize = 40;

const ID_TASK_PREFIX: &str = "tray-task-";
const ID_ALL_TASKS: &str = "tray-all-tasks";
const ID_NEW_TASK: &str = "tray-new-task";
const ID_LOG_TIME: &str = "tray-log-time";
const ID_OPEN: &str = "tray-open";
const ID_QUIT: &str = "tray-quit";

#[derive(Debug, Clone, Serialize)]
struct ActionPayload {
    action: String,
    /// The app path for `goto` (from the floating palette).
    #[serde(skip_serializing_if = "Option::is_none")]
    href: Option<String>,
}

/// A task row in the menu.
#[derive(Debug, Clone, PartialEq)]
pub struct TrayTask {
    pub id: i64,
    pub title: String,
    pub due_date: Option<String>,
    pub due_time: Option<String>,
}

/// Everything the menu shows; compared to skip needless rebuilds.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrayModel {
    pub hours_today: f64,
    pub daily_target: f64,
    pub tasks: Vec<TrayTask>,
    /// Tasks needing attention beyond `tasks`.
    pub more: usize,
}

/// The last model shown, managed state.
#[derive(Default)]
pub struct TrayState(Mutex<Option<TrayModel>>);

/// `5.5`, `8`, `0.25`: at most two decimals, no trailing zeros.
pub fn format_hours(h: f64) -> String {
    let s = format!("{:.2}", (h * 100.0).round() / 100.0);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The disabled summary line at the top of the menu.
pub fn summary(model: &TrayModel) -> String {
    if model.daily_target > 0.0 {
        format!(
            "Today: {} of {} h logged",
            format_hours(model.hours_today),
            format_hours(model.daily_target)
        )
    } else {
        format!("Today: {} h logged", format_hours(model.hours_today))
    }
}

/// A task row: an "Overdue" or due-time prefix, then the (shortened) title.
pub fn task_label(task: &TrayTask, today: &str) -> String {
    let mut title: String = task.title.chars().take(MAX_TITLE_CHARS).collect();
    if task.title.chars().count() > MAX_TITLE_CHARS {
        title = format!("{}…", title.trim_end());
    }
    match (task.due_date.as_deref(), task.due_time.as_deref()) {
        (Some(d), _) if d < today => format!("Overdue · {title}"),
        (Some(_), Some(t)) => format!("{t} · {title}"),
        _ => title,
    }
}

/// Reads today's hours, the daily target and the tasks needing attention.
pub async fn load_model(
    pool: &Pool<Sqlite>,
    now: chrono::NaiveDateTime,
) -> Result<TrayModel, AppError> {
    let today = now.date().format("%Y-%m-%d").to_string();
    let hours_today: f64 =
        sqlx::query_scalar("SELECT COALESCE(SUM(Hours), 0.0) FROM time_entries WHERE Date = ?")
            .bind(&today)
            .fetch_one(pool)
            .await?;
    let daily_target = crate::app_settings::get_daily_target(pool).await?;
    let rows: Vec<(i64, String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT t.id, t.Title, t.DueDate, t.DueTime FROM tasks t WHERE t.Done = 0 AND ( \
             (t.DueDate IS NOT NULL AND t.DueDate <= ?) \
             OR EXISTS (SELECT 1 FROM task_reminders r WHERE r.TaskId = t.id AND r.State = 'sent')) \
         ORDER BY t.DueDate IS NULL, t.DueDate, t.DueTime IS NULL, t.DueTime, t.id",
    )
    .bind(&today)
    .fetch_all(pool)
    .await?;
    let more = rows.len().saturating_sub(MAX_TASKS);
    let tasks = rows
        .into_iter()
        .take(MAX_TASKS)
        .map(|(id, title, due_date, due_time)| TrayTask {
            id,
            title,
            due_date,
            due_time,
        })
        .collect();
    Ok(TrayModel {
        hours_today,
        daily_target,
        tasks,
        more,
    })
}

fn build_menu(app: &AppHandle, model: &TrayModel, today: &str) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(
        app,
        "tray-summary",
        summary(model),
        false,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    if model.tasks.is_empty() {
        menu.append(&MenuItem::with_id(
            app,
            "tray-no-tasks",
            "Nothing due today",
            false,
            None::<&str>,
        )?)?;
    }
    for task in &model.tasks {
        let id = format!("{ID_TASK_PREFIX}{}", task.id);
        menu.append(&MenuItem::with_id(
            app,
            id,
            task_label(task, today),
            true,
            None::<&str>,
        )?)?;
    }
    if model.more > 0 {
        let label = format!("{} more…", model.more);
        menu.append(&MenuItem::with_id(
            app,
            ID_ALL_TASKS,
            label,
            true,
            None::<&str>,
        )?)?;
    }
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_NEW_TASK,
        "New Task…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_LOG_TIME,
        "Log Time…",
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_OPEN,
        "Open Badger",
        true,
        None::<&str>,
    )?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(
        app,
        ID_QUIT,
        "Quit Badger",
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

/// Shows, unminimizes and focuses the main window.
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        for (step, result) in [
            ("show", window.show()),
            ("unminimize", window.unminimize()),
            ("focus", window.set_focus()),
        ] {
            if let Err(e) = result {
                tracing::debug!("Tray: window {} failed: {}", step, e);
            }
        }
    }
}

/// Shows the window and asks the frontend to perform `action`.
pub fn request_action(app: &AppHandle, action: impl Into<String>, href: Option<String>) {
    show_main_window(app);
    let payload = ActionPayload {
        action: action.into(),
        href,
    };
    if let Err(e) = app.emit_to("main", ACTION_EVENT, payload) {
        tracing::debug!("Tray: {} not sent: {}", ACTION_EVENT, e);
    }
}

fn on_menu_event(app: &AppHandle, id: &str) {
    match id {
        ID_NEW_TASK => request_action(app, "new-task", None),
        ID_LOG_TIME => request_action(app, "log-time", None),
        ID_OPEN => show_main_window(app),
        ID_ALL_TASKS => crate::task_scheduler::open_tasks(app, None),
        ID_QUIT => app.exit(0), // goes through RunEvent::ExitRequested → flush guard
        _ => {
            if let Some(task_id) = id
                .strip_prefix(ID_TASK_PREFIX)
                .and_then(|s| s.parse::<i64>().ok())
            {
                crate::task_scheduler::open_tasks(app, Some(task_id));
            }
        }
    }
}

/// Creates the tray item (icon only until the first refresh fills it).
pub fn install(app: &AppHandle) -> tauri::Result<()> {
    let menu = build_menu(app, &TrayModel::default(), "")?;
    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Badger")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| on_menu_event(app, event.id().as_ref()));
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

static NUDGE: tokio::sync::Notify = tokio::sync::Notify::const_new();

/// Asks [`run`] to refresh the menu; nudges while a refresh runs coalesce into one more.
pub fn nudge() {
    NUDGE.notify_one();
}

/// Refreshes the menu on each nudge, for the app's lifetime.
pub async fn run(app: AppHandle, pool: Pool<Sqlite>) {
    loop {
        NUDGE.notified().await;
        refresh(&app, &pool).await;
    }
}

/// Rebuilds the menu when what it shows changed. Never fails: errors are logged.
async fn refresh(app: &AppHandle, pool: &Pool<Sqlite>) {
    let now = local_now();
    let model = match load_model(pool, now).await {
        Ok(model) => model,
        Err(e) => {
            tracing::error!("Tray: couldn't load the menu: {}", e);
            return;
        }
    };
    let Some(state) = app.try_state::<TrayState>() else {
        return;
    };
    {
        let Ok(mut last) = state.0.lock() else {
            return;
        };
        if last.as_ref() == Some(&model) {
            return;
        }
        *last = Some(model.clone());
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    let today = now.date().format("%Y-%m-%d").to_string();
    let result = build_menu(app, &model, &today).and_then(|menu| tray.set_menu(Some(menu)));
    if let Err(e) = result {
        tracing::error!("Tray: couldn't update the menu: {}", e);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn task(title: &str, due_date: Option<&str>, due_time: Option<&str>) -> TrayTask {
        TrayTask {
            id: 1,
            title: title.to_string(),
            due_date: due_date.map(String::from),
            due_time: due_time.map(String::from),
        }
    }

    #[test]
    fn labels() {
        assert_eq!(format_hours(5.5), "5.5");
        assert_eq!(format_hours(8.0), "8");
        assert_eq!(format_hours(0.25), "0.25");
        assert_eq!(format_hours(1.0 / 3.0), "0.33");
        let mut model = TrayModel {
            daily_target: 8.0,
            ..TrayModel::default()
        };
        assert_eq!(summary(&model), "Today: 0 of 8 h logged");
        model.hours_today = 5.5;
        model.daily_target = 0.0;
        assert_eq!(summary(&model), "Today: 5.5 h logged");

        let today = "2026-11-26";
        assert_eq!(
            task_label(&task("Call Anna", Some("2026-11-25"), Some("09:00")), today),
            "Overdue · Call Anna"
        );
        assert_eq!(
            task_label(&task("Call Anna", Some(today), Some("14:30")), today),
            "14:30 · Call Anna"
        );
        assert_eq!(
            task_label(&task("Call Anna", Some(today), None), today),
            "Call Anna"
        );
        assert_eq!(task_label(&task("Pinged", None, None), today), "Pinged");
        let long = "å".repeat(50);
        assert_eq!(
            task_label(&task(&long, None, None), today),
            format!("{}…", "å".repeat(40))
        );
    }

    #[tokio::test]
    async fn model_lists_tasks_needing_attention_and_todays_hours() {
        let pool = crate::test_db().await;
        sqlx::raw_sql(
            "INSERT INTO customers (id, Name) VALUES (1, 'Acme');
             INSERT INTO projects (id, CustomerId, Name) VALUES (1, 1, 'Web');
             INSERT INTO issues (id, ProjectId, Title) VALUES (1, 1, 'Login');
             INSERT INTO time_entries (IssueId, Date, Hours) VALUES
                 (1, '2026-11-26', 2.5), (1, '2026-11-26', 1.0), (1, '2026-11-25', 8.0);",
        )
        .execute(&pool)
        .await
        .unwrap();
        let insert = |title: &str, done: i64, date: Option<&str>, time: Option<&str>| {
            sqlx::query(
                "INSERT INTO tasks (Title, Done, DueDate, DueTime, CreatedAt, UpdatedAt) \
                 VALUES (?, ?, ?, ?, '2026-11-01T00:00:00', '2026-11-01T00:00:00')",
            )
            .bind(title.to_string())
            .bind(done)
            .bind(date.map(String::from))
            .bind(time.map(String::from))
        };
        for q in [
            insert("Later", 0, Some("2026-11-27"), None),
            insert("Done", 1, Some("2026-11-20"), None),
            insert("Today 14", 0, Some("2026-11-26"), Some("14:00")),
            insert("Overdue", 0, Some("2026-11-20"), None),
            insert("Undated", 0, None, None),
        ] {
            q.execute(&pool).await.unwrap();
        }
        let now = NaiveDate::from_ymd_opt(2026, 11, 26)
            .unwrap()
            .and_hms_opt(9, 0, 0)
            .unwrap();
        let model = load_model(&pool, now).await.unwrap();
        assert_eq!(model.hours_today, 3.5);
        assert_eq!(model.daily_target, 8.0);
        let titles: Vec<&str> = model.tasks.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, vec!["Overdue", "Today 14"]);
        assert_eq!(model.more, 0);
    }
}
