//! Semantic note search: an in-process embedding model (downloaded on opt-in), a derived
//! vector index of note chunks (`note_embeddings` + an in-memory cache), and hybrid
//! ranking with the keyword search.
//!
//! `SemanticState` is managed Tauri state, cheap to clone (an `Arc`). One background
//! worker (`index::run_worker`) embeds queued notes; note write commands enqueue ids
//! after their transaction commits.

pub mod chunker;
pub mod embedder;
pub mod fusion;
pub mod index;
pub mod model_files;

#[cfg(test)]
mod real_model_tests;

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use embedder::Embedder;

/// Tauri event carrying a `SemanticStatus` whenever it changes.
pub const STATUS_EVENT: &str = "semantic-status";

/// One embedded chunk in the cache.
#[derive(Debug, Clone)]
pub struct CachedChunk {
    pub index: i64,
    pub vector: Vec<f32>,
}

/// What the state machine is doing besides indexing.
#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Idle,
    Downloading {
        done: u64,
        total: u64,
    },
    /// Loading the model into memory.
    Loading,
    Error(String),
}

#[derive(Default)]
struct Queue {
    order: VecDeque<i64>,
    queued: HashSet<i64>,
    /// Progress of the current indexing run (reset when the queue drains).
    total: usize,
    done: usize,
}

type Emitter = Box<dyn Fn() + Send + Sync>;

pub(crate) struct Shared {
    pub models_dir: PathBuf,
    enabled: AtomicBool,
    /// Bumped (with the cache cleared) by restore, remove and rebuild: in-flight results
    /// computed under an older epoch are dropped.
    epoch: AtomicU64,
    cache: RwLock<HashMap<i64, Vec<CachedChunk>>>,
    embedder: RwLock<Option<Arc<dyn Embedder>>>,
    /// One inference at a time: queries wait for at most one indexing chunk.
    inference: Mutex<()>,
    queue: Mutex<Queue>,
    wake: tokio::sync::Notify,
    phase: Mutex<Phase>,
    /// Set to stop a running download.
    cancel: AtomicBool,
    pub(crate) downloading: AtomicBool,
    /// Serialises enable (download → enable → load), startup activation, remove and
    /// rebuild, so a download finishing during a remove can't re-enable without files.
    pub(crate) lifecycle: tokio::sync::Mutex<()>,
    emit: RwLock<Option<Emitter>>,
}

/// Managed state for semantic search. Clone freely.
#[derive(Clone)]
pub struct SemanticState(pub(crate) Arc<Shared>);

impl SemanticState {
    pub fn new(models_dir: PathBuf) -> Self {
        SemanticState(Arc::new(Shared {
            models_dir,
            enabled: AtomicBool::new(false),
            epoch: AtomicU64::new(0),
            cache: RwLock::new(HashMap::new()),
            embedder: RwLock::new(None),
            inference: Mutex::new(()),
            queue: Mutex::new(Queue::default()),
            wake: tokio::sync::Notify::new(),
            phase: Mutex::new(Phase::Idle),
            cancel: AtomicBool::new(false),
            downloading: AtomicBool::new(false),
            lifecycle: tokio::sync::Mutex::new(()),
            emit: RwLock::new(None),
        }))
    }

    pub fn models_dir(&self) -> &std::path::Path {
        &self.0.models_dir
    }

    /// Called (without arguments) whenever the status may have changed; the app wires it
    /// to a `STATUS_EVENT` emit.
    pub fn set_emitter(&self, emit: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut slot) = self.0.emit.write() {
            *slot = Some(Box::new(emit));
        }
    }

    pub fn notify(&self) {
        if let Ok(slot) = self.0.emit.read() {
            if let Some(emit) = slot.as_ref() {
                emit();
            }
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.0.enabled.load(Ordering::SeqCst)
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.0.enabled.store(enabled, Ordering::SeqCst);
    }

    pub fn phase(&self) -> Phase {
        self.0
            .phase
            .lock()
            .map(|p| p.clone())
            .unwrap_or(Phase::Idle)
    }

    pub fn set_phase(&self, phase: Phase) {
        if let Ok(mut p) = self.0.phase.lock() {
            *p = phase;
        }
        self.notify();
    }

    pub fn embedder(&self) -> Option<Arc<dyn Embedder>> {
        self.0.embedder.read().ok().and_then(|e| e.clone())
    }

    pub fn set_embedder(&self, embedder: Option<Arc<dyn Embedder>>) {
        if let Ok(mut slot) = self.0.embedder.write() {
            *slot = embedder;
        }
        self.0.wake.notify_one();
        self.notify();
    }

    /// Semantic search answers queries: enabled, model loaded.
    pub fn is_ready(&self) -> bool {
        self.is_enabled() && self.embedder().is_some()
    }

    pub fn epoch(&self) -> u64 {
        self.0.epoch.load(Ordering::SeqCst)
    }

    /// Invalidates in-flight work and empties the vector cache.
    pub fn bump_epoch(&self) {
        if let Ok(mut cache) = self.0.cache.write() {
            self.0.epoch.fetch_add(1, Ordering::SeqCst);
            cache.clear();
        } else {
            self.0.epoch.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Queues notes for (re)embedding; ids already queued keep their place.
    pub fn enqueue(&self, ids: impl IntoIterator<Item = i64>) {
        if !self.is_enabled() {
            return;
        }
        let mut added = false;
        if let Ok(mut q) = self.0.queue.lock() {
            for id in ids {
                if q.queued.insert(id) {
                    q.order.push_back(id);
                    q.total += 1;
                    added = true;
                }
            }
        }
        if added {
            self.0.wake.notify_one();
            self.notify();
        }
    }

    pub fn clear_queue(&self) {
        if let Ok(mut q) = self.0.queue.lock() {
            *q = Queue::default();
        }
    }

    /// Removes a deleted note's vectors from the cache (its rows cascade).
    pub fn evict(&self, id: i64) {
        self.evict_cache(id);
        if let Ok(mut q) = self.0.queue.lock() {
            if q.queued.remove(&id) {
                q.order.retain(|x| *x != id);
                q.total = q.total.saturating_sub(1);
            }
        }
    }

    /// Drops a note from the vector cache only (its queue entry stays).
    pub fn evict_cache(&self, id: i64) {
        if let Ok(mut cache) = self.0.cache.write() {
            cache.remove(&id);
        }
    }

    /// (done, total) of the current indexing run; (0, 0) when idle.
    pub fn progress(&self) -> (usize, usize) {
        self.0
            .queue
            .lock()
            .map(|q| (q.done, q.total))
            .unwrap_or((0, 0))
    }

    /// Queued note ids in processing order.
    #[cfg(test)]
    pub(crate) fn queued(&self) -> Vec<i64> {
        self.0
            .queue
            .lock()
            .map(|q| q.order.iter().copied().collect())
            .unwrap_or_default()
    }

    fn next_queued(&self) -> Option<i64> {
        let mut q = self.0.queue.lock().ok()?;
        let id = q.order.pop_front()?;
        q.queued.remove(&id);
        Some(id)
    }

    fn mark_done(&self) {
        if let Ok(mut q) = self.0.queue.lock() {
            q.done += 1;
            if q.order.is_empty() {
                q.done = 0;
                q.total = 0;
            }
        }
    }

    fn requeue_front(&self, id: i64) {
        if let Ok(mut q) = self.0.queue.lock() {
            if q.queued.insert(id) {
                q.order.push_front(id);
            }
        }
    }

    pub fn indexed_notes(&self) -> usize {
        self.0.cache.read().map(|c| c.len()).unwrap_or(0)
    }

    /// Runs `f` with the inference lock held (one model call at a time).
    fn with_inference<T>(&self, f: impl FnOnce() -> T) -> T {
        let _guard = self.0.inference.lock();
        f()
    }

    /// Embeds `texts` on a blocking thread under the inference lock.
    pub async fn embed(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, String> {
        let Some(embedder) = self.embedder() else {
            return Err("The model is not loaded".to_string());
        };
        let state = self.clone();
        tokio::task::spawn_blocking(move || state.with_inference(|| embedder.embed(&texts)))
            .await
            .map_err(|e| format!("Embedding task failed: {e}"))?
    }

    pub fn is_downloading(&self) -> bool {
        self.0.downloading.load(Ordering::SeqCst)
    }

    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.0.cancel
    }
}
