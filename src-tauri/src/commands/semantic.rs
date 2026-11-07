//! Semantic search lifecycle: status, enable (download → verify → load → index), cancel,
//! remove and rebuild. Progress reaches the frontend as `semantic::STATUS_EVENT` events
//! (no payload: the listener re-reads `get_semantic_status`).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

use serde::Serialize;
use sqlx::{Pool, Sqlite};
use tauri::State;

use crate::semantic::embedder::{Embedder, OrtEmbedder};
use crate::semantic::model_files::{self, DownloadError, RemoteFile, Source};
use crate::semantic::{Phase, SemanticState};
use crate::AppError;

const MISSING_MODEL: &str = "The model files are missing. Download them again.";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct SemanticStatus {
    /// `off`, `downloading`, `loading`, `indexing`, `ready` or `error`.
    pub state: String,
    pub enabled: bool,
    pub downloaded_bytes: u64,
    pub download_bytes: u64,
    /// Indexing run progress: notes processed of notes queued.
    pub indexing_done: usize,
    pub indexing_total: usize,
    pub error: Option<String>,
    pub model_name: String,
    /// Model files on disk.
    pub model_bytes: u64,
    /// Notes with vectors from the current model.
    pub indexed_notes: i64,
    pub notes: i64,
    pub index_bytes: i64,
}

fn state_name(state: &SemanticState) -> (&'static str, Option<String>) {
    match state.phase() {
        Phase::Downloading { .. } => ("downloading", None),
        Phase::Loading => ("loading", None),
        Phase::Error(message) => ("error", Some(message)),
        Phase::Idle if !state.is_enabled() => ("off", None),
        Phase::Idle if state.embedder().is_none() => ("loading", None),
        Phase::Idle if state.progress().1 > 0 => ("indexing", None),
        Phase::Idle => ("ready", None),
    }
}

pub async fn inner_semantic_status(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
) -> Result<SemanticStatus, AppError> {
    let (name, error) = state_name(state);
    let (downloaded_bytes, download_bytes) = match state.phase() {
        Phase::Downloading { done, total } => (done, total),
        _ => (0, model_files::FILES.iter().map(|f| f.size).sum()),
    };
    let (indexing_done, indexing_total) = state.progress();
    let (indexed_notes, notes): (i64, i64) = sqlx::query_as(
        "SELECT (SELECT COUNT(DISTINCT NoteId) FROM note_embeddings WHERE ModelId = ?),
                (SELECT COUNT(*) FROM notes)",
    )
    .bind(model_files::MODEL_ID)
    .fetch_one(pool)
    .await?;
    let (index_bytes,): (i64,) = sqlx::query_as(
        "SELECT COALESCE(SUM(pgsize), 0) FROM dbstat \
         WHERE name = 'note_embeddings' OR name LIKE 'sqlite\\_autoindex\\_note\\_embeddings%' ESCAPE '\\'",
    )
    .fetch_one(pool)
    .await?;
    Ok(SemanticStatus {
        state: name.to_string(),
        enabled: state.is_enabled(),
        downloaded_bytes,
        download_bytes,
        indexing_done,
        indexing_total,
        error,
        model_name: model_files::MODEL_NAME.to_string(),
        model_bytes: model_files::dir_bytes(&model_files::model_dir(state.models_dir())),
        indexed_notes,
        notes,
        index_bytes,
    })
}

/// Loads the model from `dir` (the app's loader; tests pass a fake).
pub type Loader = fn(&Path) -> Result<Arc<dyn Embedder>, String>;

pub fn load_model(dir: &Path) -> Result<Arc<dyn Embedder>, String> {
    let e = OrtEmbedder::load(
        &dir.join(model_files::MODEL_FILE),
        &dir.join(model_files::TOKENIZER_FILE),
        model_files::MODEL_ID,
    )?;
    Ok(Arc::new(e))
}

/// Loads the model on a blocking thread, fills the cache and queues a catch-up.
/// Errors end in the `error` state.
async fn activate(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
    dir: PathBuf,
    load: Loader,
) -> Result<(), String> {
    state.set_phase(Phase::Loading);
    let loaded = tokio::task::spawn_blocking(move || load(&dir))
        .await
        .map_err(|e| format!("Couldn't load the model: {e}"))
        .and_then(|r| r);
    let embedder = match loaded {
        Ok(e) => e,
        Err(message) => {
            tracing::error!("Semantic search: {message}");
            state.set_phase(Phase::Error(message.clone()));
            return Err(message);
        }
    };
    if !state.is_enabled() {
        // Removed while loading: drop the model again.
        state.set_phase(Phase::Idle);
        return Ok(());
    }
    let model_id = embedder.model_id().to_string();
    state.set_embedder(Some(embedder));
    if let Err(e) = state.load_cache(&model_id, pool).await {
        tracing::error!("Semantic search: couldn't load stored vectors: {e}");
    }
    state.set_phase(Phase::Idle);
    state
        .enqueue_all(pool)
        .await
        .map_err(|e| format!("Couldn't queue notes for indexing: {e}"))
}

/// Startup: drops stale `.part` files, then, when enabled, loads the model and catches
/// up on notes that changed (or appeared) since the vectors were made. Enabled with the
/// model missing → `error` state with a Download action, never a crash.
pub async fn startup(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
    files: &[RemoteFile],
    load: Loader,
) -> Result<(), String> {
    let _lifecycle = state.0.lifecycle.lock().await;
    let dir = model_files::model_dir(state.models_dir());
    model_files::clean_partial(&dir);
    let enabled = crate::app_settings::get_semantic_enabled(pool)
        .await
        .map_err(|e| e.to_string())?;
    state.set_enabled(enabled);
    if !enabled {
        return Ok(());
    }
    if !model_files::model_present(&dir, files) {
        state.set_phase(Phase::Error(MISSING_MODEL.to_string()));
        return Err(MISSING_MODEL.to_string());
    }
    activate(state, pool, dir, load).await
}

/// Downloads and verifies the model, then enables semantic search and indexes all notes.
/// The setting flips to enabled only after the files verify. Cancel → `off`.
pub async fn inner_enable_semantic_search<S: Source>(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
    source: &S,
    files: &[RemoteFile],
    load: Loader,
) -> Result<(), AppError> {
    if state.0.downloading.swap(true, Ordering::SeqCst) {
        return Err(AppError::Custom(
            "The model is already downloading".to_string(),
        ));
    }
    // Reset before waiting for the lifecycle lock: a remove that starts after this
    // point still cancels the download.
    state.cancel_flag().store(false, Ordering::SeqCst);
    let _lifecycle = state.0.lifecycle.lock().await;
    let total = files.iter().map(|f| f.size).sum();
    state.set_phase(Phase::Downloading { done: 0, total });
    let dir = model_files::model_dir(state.models_dir());
    let last_percent = std::sync::atomic::AtomicU64::new(u64::MAX);
    let progress = |done: u64, total: u64| {
        let percent = (done * 100).checked_div(total).unwrap_or(100);
        if last_percent.swap(percent, Ordering::SeqCst) != percent {
            state.set_phase(Phase::Downloading { done, total });
        }
    };
    let result = model_files::download(source, files, &dir, state.cancel_flag(), &progress).await;
    state.0.downloading.store(false, Ordering::SeqCst);
    match result {
        Err(DownloadError::Cancelled) => {
            state.set_phase(Phase::Idle);
            return Ok(());
        }
        Err(DownloadError::Failed(message)) => {
            tracing::error!("Semantic search download failed: {message}");
            state.set_phase(Phase::Error(message.clone()));
            return Err(AppError::Custom(message));
        }
        Ok(()) => {}
    }
    crate::app_settings::set_semantic_enabled(pool, true).await?;
    state.set_enabled(true);
    activate(state, pool, dir, load)
        .await
        .map_err(AppError::Custom)
}

pub fn inner_cancel_semantic_download(state: &SemanticState) {
    state.cancel_flag().store(true, Ordering::SeqCst);
}

/// Cancels a running download, waits for an in-flight enable or startup to finish, then
/// deletes the model files and all vectors and turns semantic search off.
pub async fn inner_remove_semantic_model(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    inner_cancel_semantic_download(state);
    let _lifecycle = state.0.lifecycle.lock().await;
    crate::app_settings::set_semantic_enabled(pool, false).await?;
    state.set_enabled(false);
    state.clear_queue();
    state.set_embedder(None);
    state.clear_index(pool).await?;
    let dir = model_files::model_dir(state.models_dir());
    let removed = tokio::task::spawn_blocking(move || model_files::remove_model(&dir))
        .await
        .map_err(|e| AppError::Custom(format!("Couldn't remove the model: {e}")))?;
    state.set_phase(Phase::Idle);
    removed.map_err(|e| AppError::Custom(format!("Couldn't remove the model files: {e}")))
}

/// Drops all vectors and re-embeds every note.
pub async fn inner_rebuild_semantic_index(
    state: &SemanticState,
    pool: &Pool<Sqlite>,
) -> Result<(), AppError> {
    let _lifecycle = state.0.lifecycle.lock().await;
    if !state.is_ready() {
        return Err(AppError::Custom("Semantic search is not ready".to_string()));
    }
    state.clear_queue();
    state.clear_index(pool).await?;
    state.enqueue_all(pool).await?;
    Ok(())
}

/// After a restore: in-flight work is stale, the cascade dropped the vectors, and every
/// restored note needs embedding.
pub async fn after_restore(state: &SemanticState, pool: &Pool<Sqlite>) {
    state.bump_epoch();
    state.clear_queue();
    if let Err(e) = state.enqueue_all(pool).await {
        tracing::error!("Semantic search: couldn't queue restored notes: {e}");
    }
}

// ── Tauri commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn get_semantic_status(
    state: State<'_, SemanticState>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SemanticStatus, AppError> {
    inner_semantic_status(state.inner(), pool.inner()).await
}

/// Starts the download in the background and returns at once; progress arrives as
/// status events.
#[tauri::command]
pub async fn enable_semantic_search(
    state: State<'_, SemanticState>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SemanticStatus, AppError> {
    if state.is_downloading() {
        return Err(AppError::Custom(
            "The model is already downloading".to_string(),
        ));
    }
    let source = model_files::HttpSource::new().map_err(AppError::Custom)?;
    let (task_state, task_pool) = (state.inner().clone(), pool.inner().clone());
    tauri::async_runtime::spawn(async move {
        // Errors are reported through the status.
        let _ = inner_enable_semantic_search(
            &task_state,
            &task_pool,
            &source,
            &model_files::FILES,
            load_model,
        )
        .await;
    });
    // Let the task flip the phase so the returned status already says downloading.
    for _ in 0..20 {
        if state.is_downloading() || state.phase() != Phase::Idle {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    inner_semantic_status(state.inner(), pool.inner()).await
}

#[tauri::command]
pub async fn cancel_semantic_download(state: State<'_, SemanticState>) -> Result<(), AppError> {
    inner_cancel_semantic_download(state.inner());
    Ok(())
}

#[tauri::command]
pub async fn remove_semantic_model(
    state: State<'_, SemanticState>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SemanticStatus, AppError> {
    inner_remove_semantic_model(state.inner(), pool.inner()).await?;
    inner_semantic_status(state.inner(), pool.inner()).await
}

#[tauri::command]
pub async fn rebuild_semantic_index(
    state: State<'_, SemanticState>,
    pool: State<'_, Pool<Sqlite>>,
) -> Result<SemanticStatus, AppError> {
    inner_rebuild_semantic_index(state.inner(), pool.inner()).await?;
    inner_semantic_status(state.inner(), pool.inner()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::notes;
    use crate::semantic::embedder::FakeEmbedder;
    use crate::semantic::model_files::tests::{fake_files, TempDir};

    fn fake_loader(_: &Path) -> Result<Arc<dyn Embedder>, String> {
        Ok(Arc::new(FakeEmbedder::new(model_files::MODEL_ID)))
    }

    fn failing_loader(_: &Path) -> Result<Arc<dyn Embedder>, String> {
        Err("Couldn't load the model: broken".to_string())
    }

    async fn status(state: &SemanticState, pool: &Pool<Sqlite>) -> SemanticStatus {
        inner_semantic_status(state, pool).await.unwrap()
    }

    async fn enabled_setting(pool: &Pool<Sqlite>) -> bool {
        crate::app_settings::get_semantic_enabled(pool)
            .await
            .unwrap()
    }

    async fn embedding_rows(pool: &Pool<Sqlite>) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM note_embeddings")
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn starts_off_and_enable_downloads_then_indexes_everything() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("enable");
        let state = SemanticState::new(tmp.0.clone());
        let a = notes::inner_create_note_titled("Budget", None, &pool)
            .await
            .unwrap();
        notes::inner_create_note_titled("Garden", None, &pool)
            .await
            .unwrap();
        assert_eq!(status(&state, &pool).await.state, "off");

        let (files, source) = fake_files();
        inner_enable_semantic_search(&state, &pool, &source, &files, fake_loader)
            .await
            .unwrap();
        assert!(enabled_setting(&pool).await);
        let s = status(&state, &pool).await;
        assert_eq!((s.state.as_str(), s.indexing_total), ("indexing", 2));
        assert!(s.model_bytes > 0);

        state.drain(&pool).await;
        let s = status(&state, &pool).await;
        assert_eq!(s.state, "ready");
        assert_eq!((s.indexed_notes, s.notes), (2, 2));
        assert!(s.index_bytes > 0);
        let _ = a;
    }

    #[tokio::test]
    async fn failed_download_reports_error_and_stays_disabled() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("enable-fail");
        let state = SemanticState::new(tmp.0.clone());
        let (files, mut source) = fake_files();
        source.files.clear();
        let err = inner_enable_semantic_search(&state, &pool, &source, &files, fake_loader)
            .await
            .unwrap_err();
        assert_eq!(
            err.to_string(),
            "The download failed (HTTP 404). Retry later."
        );
        let s = status(&state, &pool).await;
        assert_eq!(s.state, "error");
        assert_eq!(
            s.error.as_deref(),
            Some("The download failed (HTTP 404). Retry later.")
        );
        assert!(!enabled_setting(&pool).await);
        assert_eq!(s.model_bytes, 0);
    }

    #[tokio::test]
    async fn cancel_before_body_ends_in_off_with_no_files() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("enable-cancel2");
        let state = SemanticState::new(tmp.0.clone());
        let (files, source) = fake_files();
        // The user cancels as the request goes out; the download sees it on the first chunk.
        struct CancelOnGet<'a>(
            &'a SemanticState,
            crate::semantic::model_files::tests::FakeSource,
        );
        impl Source for CancelOnGet<'_> {
            type Body = crate::semantic::model_files::tests::FakeBody;
            async fn get(&self, url: &str) -> Result<Self::Body, String> {
                inner_cancel_semantic_download(self.0);
                self.1.get(url).await
            }
        }
        let src = CancelOnGet(&state, source);
        inner_enable_semantic_search(&state, &pool, &src, &files, fake_loader)
            .await
            .unwrap();
        let s = status(&state, &pool).await;
        assert_eq!(s.state, "off");
        assert!(!enabled_setting(&pool).await);
        assert_eq!(s.model_bytes, 0);
    }

    #[tokio::test]
    async fn load_failure_after_download_is_an_error_state() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("enable-load");
        let state = SemanticState::new(tmp.0.clone());
        let (files, source) = fake_files();
        let err = inner_enable_semantic_search(&state, &pool, &source, &files, failing_loader)
            .await
            .unwrap_err();
        assert_eq!(err.to_string(), "Couldn't load the model: broken");
        assert_eq!(status(&state, &pool).await.state, "error");
    }

    #[tokio::test]
    async fn startup_states() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("startup");
        let (files, source) = fake_files();

        // Disabled: off, stale .part removed.
        let dir = model_files::model_dir(&tmp.0);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model_quantized.onnx.part"), b"half").unwrap();
        let state = SemanticState::new(tmp.0.clone());
        startup(&state, &pool, &files, fake_loader).await.unwrap();
        assert_eq!(status(&state, &pool).await.state, "off");
        assert!(!dir.join("model_quantized.onnx.part").exists());

        // Enabled but files missing: error with the Download hint, no crash.
        crate::app_settings::set_semantic_enabled(&pool, true)
            .await
            .unwrap();
        let state = SemanticState::new(tmp.0.clone());
        assert_eq!(
            startup(&state, &pool, &files, fake_loader).await,
            Err(MISSING_MODEL.to_string())
        );
        let s = status(&state, &pool).await;
        assert_eq!(
            (s.state.as_str(), s.error.as_deref()),
            ("error", Some(MISSING_MODEL))
        );

        // Enabled and present: loads and catches up on notes.
        let n = notes::inner_create_note_titled("Plan", None, &pool)
            .await
            .unwrap();
        model_files::download(&source, &files, &dir, &Default::default(), &|_, _| {})
            .await
            .unwrap();
        let state = SemanticState::new(tmp.0.clone());
        startup(&state, &pool, &files, fake_loader).await.unwrap();
        state.drain(&pool).await;
        assert_eq!(status(&state, &pool).await.state, "ready");
        assert_eq!(embedding_rows(&pool).await, 1);

        // Next start: the stored vectors load into the cache; nothing re-embeds.
        let state = SemanticState::new(tmp.0.clone());
        startup(&state, &pool, &files, fake_loader).await.unwrap();
        assert_eq!(state.indexed_notes(), 1);
        state.drain(&pool).await;
        let _ = n;
    }

    #[tokio::test]
    async fn remove_deletes_files_and_vectors_and_disables() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("remove");
        let state = SemanticState::new(tmp.0.clone());
        notes::inner_create_note_titled("Plan", None, &pool)
            .await
            .unwrap();
        let (files, source) = fake_files();
        inner_enable_semantic_search(&state, &pool, &source, &files, fake_loader)
            .await
            .unwrap();
        state.drain(&pool).await;
        assert_eq!(embedding_rows(&pool).await, 1);

        inner_remove_semantic_model(&state, &pool).await.unwrap();
        let s = status(&state, &pool).await;
        assert_eq!(s.state, "off");
        assert_eq!((s.model_bytes, s.indexed_notes), (0, 0));
        assert_eq!(embedding_rows(&pool).await, 0);
        assert_eq!(state.indexed_notes(), 0);
        assert!(!enabled_setting(&pool).await);
        assert!(!model_files::model_dir(&tmp.0).exists());
        // Notes edited while off are not queued.
        state.enqueue([1]);
        assert_eq!(state.progress(), (0, 0));
    }

    /// A load slow enough for a remove to arrive mid-enable.
    fn slow_loader(dir: &Path) -> Result<Arc<dyn Embedder>, String> {
        std::thread::sleep(std::time::Duration::from_millis(300));
        fake_loader(dir)
    }

    #[tokio::test]
    async fn remove_during_enable_waits_and_leaves_everything_off() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("remove-race");
        let state = SemanticState::new(tmp.0.clone());
        notes::inner_create_note_titled("Plan", None, &pool)
            .await
            .unwrap();
        let (files, source) = fake_files();
        let order = std::sync::Mutex::new(Vec::new());
        let enable = async {
            let r = inner_enable_semantic_search(&state, &pool, &source, &files, slow_loader).await;
            order.lock().unwrap().push("enable");
            r
        };
        let remove = async {
            // The download has verified and the model is loading: the riskiest moment.
            while state.phase() != Phase::Loading {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
            let r = inner_remove_semantic_model(&state, &pool).await;
            order.lock().unwrap().push("remove");
            r
        };
        let (enabled, removed) = tokio::join!(enable, remove);
        enabled.unwrap();
        removed.unwrap();
        assert_eq!(*order.lock().unwrap(), vec!["enable", "remove"]);

        let s = status(&state, &pool).await;
        assert_eq!((s.state.as_str(), s.enabled), ("off", false));
        assert!(!enabled_setting(&pool).await);
        assert!(state.embedder().is_none());
        assert!(!model_files::model_dir(&tmp.0).exists());
        assert_eq!(embedding_rows(&pool).await, 0);
        assert_eq!(state.progress(), (0, 0));
    }

    #[tokio::test]
    async fn rebuild_reembeds_everything_and_needs_a_ready_model() {
        let pool = crate::test_db().await;
        let tmp = TempDir::new("rebuild");
        let state = SemanticState::new(tmp.0.clone());
        assert_eq!(
            inner_rebuild_semantic_index(&state, &pool)
                .await
                .unwrap_err()
                .to_string(),
            "Semantic search is not ready"
        );
        notes::inner_create_note_titled("Plan", None, &pool)
            .await
            .unwrap();
        let (files, source) = fake_files();
        inner_enable_semantic_search(&state, &pool, &source, &files, fake_loader)
            .await
            .unwrap();
        state.drain(&pool).await;
        inner_rebuild_semantic_index(&state, &pool).await.unwrap();
        assert_eq!(embedding_rows(&pool).await, 0);
        assert_eq!(state.progress(), (0, 1));
        state.drain(&pool).await;
        assert_eq!(embedding_rows(&pool).await, 1);
    }
}
