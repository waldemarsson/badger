//! Running playground scripts: one run at a time app-wide, each in a fresh QuickJS runtime
//! on a blocking thread (see `js_runner`). `stop_script` cancels the run it names; a stale
//! or unknown id is a no-op. `note(title)` reads notes through the pool.
use crate::js_runner::{self, NoteValue, RunOptions, RunOutcome};
use crate::AppError;
use serde::Serialize;
use sqlx::{Pool, Sqlite};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use tauri::State;

const BUSY: &str = "A script is already running";

/// The finished run: its id (as `script_running` reported it) and what it produced.
#[derive(Debug, Clone, Serialize)]
pub struct RunResult {
    pub run_id: u64,
    pub outcome: RunOutcome,
}

/// The run in progress: its id and its own Stop flag (a fresh flag per run, so a late Stop
/// for an earlier run can't reach the next one).
struct Current {
    id: u64,
    cancel: Arc<AtomicBool>,
}

/// Managed state: the run in progress, if any.
#[derive(Default)]
pub struct RunState {
    current: Mutex<Option<Current>>,
    next_id: AtomicU64,
}

impl RunState {
    fn slot(&self) -> MutexGuard<'_, Option<Current>> {
        // A poisoned lock still holds a valid Option; never panic over it.
        self.current.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Claims the single run slot: a new id and a cleared Stop flag.
    fn begin(&self) -> Result<(u64, Arc<AtomicBool>), AppError> {
        let mut slot = self.slot();
        if slot.is_some() {
            return Err(AppError::Custom(BUSY.to_string()));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst) + 1;
        let cancel = Arc::new(AtomicBool::new(false));
        *slot = Some(Current {
            id,
            cancel: cancel.clone(),
        });
        Ok((id, cancel))
    }

    fn end(&self, id: u64) {
        let mut slot = self.slot();
        if slot.as_ref().is_some_and(|c| c.id == id) {
            *slot = None;
        }
    }

    /// The id of the run in progress.
    pub fn running(&self) -> Option<u64> {
        self.slot().as_ref().map(|c| c.id)
    }

    /// Stops run `id` if it is the one in progress; otherwise does nothing.
    pub fn stop(&self, id: u64) {
        if let Some(c) = self.slot().as_ref().filter(|c| c.id == id) {
            c.cancel.store(true, Ordering::SeqCst);
        }
    }
}

/// Frees the run slot when the run ends, however it ends.
struct Claim<'a> {
    state: &'a RunState,
    id: u64,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        self.state.end(self.id);
    }
}

/// `note(title)`: the note whose title matches, folding case in Rust like the notes
/// duplicate check (`to_lowercase`, so `Å` matches `å`). Surrounding spaces are ignored.
pub async fn lookup_note(title: &str, pool: &Pool<Sqlite>) -> Result<Option<NoteValue>, AppError> {
    let wanted = title.trim().to_lowercase();
    let rows: Vec<(i64, String)> = sqlx::query_as("SELECT id, Title FROM notes")
        .fetch_all(pool)
        .await?;
    let Some((id, _)) = rows.into_iter().find(|(_, t)| t.to_lowercase() == wanted) else {
        return Ok(None);
    };
    let row: Option<(String, String)> =
        sqlx::query_as("SELECT Kind AS kind, Body AS body FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    Ok(row.map(|(kind, body)| {
        if kind == crate::commands::notes::KIND_JSON {
            NoteValue::Json(body)
        } else {
            NoteValue::Markdown(body)
        }
    }))
}

// ── Inner functions (testable without Tauri) ──────────────────────────────────

/// Runs `code` (the editor's current text) with `opts`. Fails only when another run is in
/// progress; everything the script does wrong is in the outcome.
pub async fn inner_run_script(
    code: String,
    opts: RunOptions,
    state: &RunState,
    pool: &Pool<Sqlite>,
) -> Result<RunResult, AppError> {
    let (run_id, cancel) = state.begin()?;
    let _claim = Claim { state, id: run_id };
    let handle = tokio::runtime::Handle::current();
    let pool = pool.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let lookup = move |title: &str| {
            handle
                .block_on(lookup_note(title, &pool))
                .map_err(|e| e.to_string())
        };
        js_runner::run(&code, &opts, cancel, lookup)
    })
    .await
    .map_err(|_| AppError::Custom("The script stopped unexpectedly".to_string()))?;
    Ok(RunResult { run_id, outcome })
}

// ── Tauri command wrappers ────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_script(
    code: String,
    state: State<'_, RunState>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<RunResult, AppError> {
    inner_run_script(code, RunOptions::default(), state.inner(), pool.inner()).await
}

#[tauri::command]
pub fn stop_script(run_id: u64, state: State<'_, RunState>) {
    state.stop(run_id);
}

#[tauri::command]
pub fn script_running(state: State<'_, RunState>) -> Option<u64> {
    state.running()
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::notes;
    use crate::js_runner::ErrorKind;
    use std::time::{Duration, Instant};

    fn quick() -> RunOptions {
        RunOptions {
            timeout: Duration::from_millis(400),
            ..RunOptions::default()
        }
    }

    async fn wait_until_running(state: &RunState) -> u64 {
        let start = Instant::now();
        loop {
            if let Some(id) = state.running() {
                return id;
            }
            assert!(
                start.elapsed() < Duration::from_secs(5),
                "run never started"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn note_reads_notes_from_the_database() {
        let pool = crate::test_db().await;
        let json = notes::inner_create_note_titled("Åsa Data", None, &pool)
            .await
            .unwrap();
        notes::inner_set_note_kind(json.id, notes::KIND_JSON, Some(r#"{"n":[1,2,3]}"#), &pool)
            .await
            .unwrap();
        let md = notes::inner_create_note_titled("Plan", None, &pool)
            .await
            .unwrap();
        notes::inner_update_note_body(md.id, "# Plan\n- [ ] ship", &pool)
            .await
            .unwrap();
        let state = RunState::default();
        let code = "const d = note('åsa data'); console.log(typeof note('PLAN'));\n\
                    [d.n.length, note(' Plan ').split('\\n')[0]]";
        let r = inner_run_script(code.to_string(), quick(), &state, &pool)
            .await
            .unwrap();
        assert_eq!(r.outcome.error, None);
        assert_eq!(r.outcome.lines[0].text, "string");
        assert_eq!(r.outcome.value.as_deref(), Some("[3, \"# Plan\"]"));

        let r = inner_run_script("note('Missing')".to_string(), quick(), &state, &pool)
            .await
            .unwrap();
        let e = r.outcome.error.unwrap();
        assert_eq!(e.message, "Uncaught Error: Note \"Missing\" not found");
        assert_eq!(state.running(), None, "slot freed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_run_while_one_is_running_is_rejected() {
        let pool = crate::test_db().await;
        let state = Arc::new(RunState::default());
        let first = {
            let (state, pool) = (state.clone(), pool.clone());
            tokio::spawn(async move {
                inner_run_script("for (;;) {}".to_string(), quick(), &state, &pool).await
            })
        };
        let id = wait_until_running(&state).await;
        let err = inner_run_script("1".to_string(), quick(), &state, &pool)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "A script is already running");
        assert_eq!(
            state.running(),
            Some(id),
            "the rejected run doesn't free the slot"
        );
        let done = first.await.unwrap().unwrap();
        assert_eq!(done.run_id, id);
        assert_eq!(done.outcome.error.unwrap().kind, ErrorKind::Timeout);
        assert_eq!(state.running(), None);
        // And the next run works.
        let r = inner_run_script("1 + 1".to_string(), quick(), &state, &pool)
            .await
            .unwrap();
        assert_eq!(r.outcome.value.as_deref(), Some("2"));
        assert!(r.run_id > id);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stop_cancels_only_the_named_run() {
        let pool = crate::test_db().await;
        let state = Arc::new(RunState::default());
        // Idle and stale stops are no-ops.
        state.stop(1);
        state.stop(999);
        let old = inner_run_script("'old'".to_string(), quick(), &state, &pool)
            .await
            .unwrap();
        state.stop(old.run_id);

        let spawn_loop = |opts: RunOptions| {
            let (state, pool) = (state.clone(), pool.clone());
            tokio::spawn(async move {
                inner_run_script("for (;;) {}".to_string(), opts, &state, &pool).await
            })
        };
        // A stale id while the next run is going doesn't stop it: it runs to the timeout.
        let run = spawn_loop(quick());
        let id = wait_until_running(&state).await;
        assert_ne!(id, old.run_id);
        state.stop(old.run_id);
        state.stop(id + 1);
        let r = run.await.unwrap().unwrap();
        assert_eq!(r.outcome.error.unwrap().kind, ErrorKind::Timeout);

        // Its own id stops it, long before its (10 s) timeout, however loaded the machine is.
        let run = spawn_loop(RunOptions::default());
        let id = wait_until_running(&state).await;
        let start = Instant::now();
        state.stop(id);
        let r = run.await.unwrap().unwrap();
        let e = r.outcome.error.unwrap();
        assert_eq!(
            (e.kind, e.message.as_str()),
            (ErrorKind::Stopped, "Stopped")
        );
        assert!(start.elapsed() < Duration::from_secs(5));

        // A Stop for the finished run doesn't leak into the next one.
        state.stop(id);
        let r = inner_run_script("'fine'".to_string(), quick(), &state, &pool)
            .await
            .unwrap();
        assert_eq!(r.outcome.error, None);
        assert_eq!(r.outcome.value.as_deref(), Some("fine"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn running_reports_the_current_id() {
        let pool = crate::test_db().await;
        let state = Arc::new(RunState::default());
        assert_eq!(state.running(), None);
        let run = {
            let (state, pool) = (state.clone(), pool.clone());
            tokio::spawn(async move {
                inner_run_script("for (;;) {}".to_string(), quick(), &state, &pool).await
            })
        };
        let id = wait_until_running(&state).await;
        state.stop(id);
        assert_eq!(run.await.unwrap().unwrap().run_id, id);
        assert_eq!(state.running(), None);
    }
}
