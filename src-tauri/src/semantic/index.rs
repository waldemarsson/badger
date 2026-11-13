//! The derived vector index: `note_embeddings` rows, the in-memory cache, and the worker
//! that keeps both in sync with notes.
//!
//! Like `notes_fts`, the index is derived and not part of backups. A note's rows record
//! the hash of the text they were made from and the model id; the worker skips notes
//! whose hash and model are unchanged, so enqueueing everything is a cheap catch-up.

use sha2::{Digest, Sha256};
use sqlx::{Pool, Sqlite};

use super::chunker::{chunk_note, CHUNK_TOKENS};
use super::embedder::{dot, passage_text};
use super::fusion;
use super::{CachedChunk, SemanticState};

/// Chunks per inference call. One: the int8 model's vectors depend on their batch-mates
/// (see `OrtEmbedder`), and the inference lock is released between calls, so a query
/// waits for at most one chunk (~350 ms for the int8 model on a 2-vCPU x86 host).
pub const BATCH: usize = 1;
/// Bump when chunking, prefixes or how inputs are run change: every note's hash changes,
/// so all re-embed. 2: one input per inference run (vectors made in batches differ).
const INDEX_VERSION: &str = "2";

/// What happened to a queued note.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Embedded(usize),
    /// Hash and model unchanged.
    Unchanged,
    /// The note no longer exists.
    Missing,
    /// Restore/remove/rebuild or an edit overtook the work; the result was dropped.
    Stale,
    /// No model loaded; the note was left queued.
    NotReady,
    /// A JSON note: never embedded (any rows it has are deleted).
    Skipped,
}

pub fn content_hash(title: &str, body: &str) -> String {
    let mut h = Sha256::new();
    h.update(INDEX_VERSION.as_bytes());
    h.update([0]);
    h.update(title.as_bytes());
    h.update([0]);
    h.update(body.as_bytes());
    h.finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>()
}

pub fn encode_vector(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

pub fn decode_vector(bytes: &[u8]) -> Vec<f32> {
    bytes
        .chunks_exact(4)
        .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .collect()
}

/// What gets embedded: `Embed(title, body)` for a markdown note, `Json` for a JSON note
/// (its body is not read).
enum NoteText {
    Embed(String, String),
    Json,
}

async fn note_text(id: i64, conn: &mut sqlx::SqliteConnection) -> sqlx::Result<Option<NoteText>> {
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT Title, CASE WHEN Kind = 'json' THEN '' ELSE Body END, Kind FROM notes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(conn)
    .await?;
    Ok(row.map(|(title, body, kind)| {
        if kind == crate::commands::notes::KIND_JSON {
            NoteText::Json
        } else {
            NoteText::Embed(title, body)
        }
    }))
}

/// Deletes a JSON note's leftover rows (from before it was switched, or written by an
/// older build). The switch deletes them too; this covers every other path.
async fn delete_rows(id: i64, pool: &Pool<Sqlite>) -> sqlx::Result<()> {
    sqlx::query("DELETE FROM note_embeddings WHERE NoteId = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

impl SemanticState {
    /// Embeds one note if its text or the model changed, then replaces its rows and cache
    /// entry. The write re-checks the epoch, the note's hash and its kind inside its
    /// transaction. JSON notes are skipped, before embedding and again at the write.
    pub async fn process_note(&self, id: i64, pool: &Pool<Sqlite>) -> Result<Outcome, String> {
        let epoch = self.epoch();
        let Some(embedder) = self.embedder() else {
            return Ok(Outcome::NotReady);
        };
        let model_id = embedder.model_id().to_string();
        let db = |e: sqlx::Error| e.to_string();

        let mut conn = pool.acquire().await.map_err(db)?;
        let (title, body) = match note_text(id, &mut conn).await.map_err(db)? {
            None => {
                self.evict(id);
                return Ok(Outcome::Missing);
            }
            Some(NoteText::Json) => {
                drop(conn);
                self.evict_cache(id);
                delete_rows(id, pool).await.map_err(db)?;
                return Ok(Outcome::Skipped);
            }
            Some(NoteText::Embed(title, body)) => (title, body),
        };
        let hash = content_hash(&title, &body);
        let stored: Vec<(String, String)> =
            sqlx::query_as("SELECT ContentHash, ModelId FROM note_embeddings WHERE NoteId = ?")
                .bind(id)
                .fetch_all(&mut *conn)
                .await
                .map_err(db)?;
        drop(conn);
        let cached = self
            .0
            .cache
            .read()
            .map(|c| c.contains_key(&id))
            .unwrap_or(false);
        if !stored.is_empty() && stored.iter().all(|(h, m)| *h == hash && *m == model_id) {
            if !cached {
                self.load_note_cache(id, &model_id, epoch, pool).await?;
            }
            return Ok(Outcome::Unchanged);
        }

        let counter = embedder.clone();
        let chunks = tokio::task::spawn_blocking(move || {
            chunk_note(&title, &body, CHUNK_TOKENS, &|t| counter.count_tokens(t))
        })
        .await
        .map_err(|e| format!("Chunking failed: {e}"))?;
        let mut vectors = Vec::with_capacity(chunks.len());
        for batch in chunks.chunks(BATCH) {
            let inputs = batch.iter().map(|c| passage_text(&c.input)).collect();
            vectors.extend(self.embed(inputs).await?);
            if self.epoch() != epoch {
                return Ok(Outcome::Stale);
            }
        }
        if vectors.len() != chunks.len() {
            return Err("The model returned the wrong number of vectors".to_string());
        }

        let mut tx = pool.begin().await.map_err(db)?;
        // Write first so the transaction holds the write lock before it re-reads.
        sqlx::query("DELETE FROM note_embeddings WHERE NoteId = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        if self.epoch() != epoch {
            return Ok(Outcome::Stale); // rollback on drop
        }
        match note_text(id, &mut tx).await.map_err(db)? {
            None => {
                self.evict(id);
                return Ok(Outcome::Missing);
            }
            // Switched to JSON while it was being embedded: keep the switch's delete.
            Some(NoteText::Json) => {
                drop(tx);
                self.evict_cache(id);
                return Ok(Outcome::Skipped);
            }
            Some(NoteText::Embed(t, b)) if content_hash(&t, &b) != hash => {
                drop(tx);
                self.requeue_front(id);
                return Ok(Outcome::Stale);
            }
            Some(_) => {}
        }
        for (i, (chunk, vector)) in chunks.iter().zip(&vectors).enumerate() {
            let result = sqlx::query(
                "INSERT INTO note_embeddings (NoteId, ChunkIndex, ContentHash, ModelId, ChunkText, Vector) \
                 VALUES (?, ?, ?, ?, ?, ?)",
            )
            .bind(id)
            .bind(i as i64)
            .bind(&hash)
            .bind(&model_id)
            .bind(&chunk.text)
            .bind(encode_vector(vector))
            .execute(&mut *tx)
            .await;
            match result {
                Ok(_) => {}
                // The note vanished under us: a skip, not an error.
                Err(sqlx::Error::Database(e)) if e.is_foreign_key_violation() => {
                    return Ok(Outcome::Missing)
                }
                Err(e) => return Err(e.to_string()),
            }
        }

        // Cache before commit, while the transaction still holds the write lock: a delete
        // can't land between the two, and its eviction runs after our entry is in.
        let entry: Vec<CachedChunk> = vectors
            .into_iter()
            .enumerate()
            .map(|(i, vector)| CachedChunk {
                index: i as i64,
                vector,
            })
            .collect();
        let n = entry.len();
        self.put_cache(id, entry, epoch);
        if let Err(e) = tx.commit().await {
            self.evict_cache(id);
            return Err(e.to_string());
        }
        Ok(Outcome::Embedded(n))
    }

    /// Inserts a cache entry unless the epoch moved on (the bump clears under this lock).
    fn put_cache(&self, id: i64, entry: Vec<CachedChunk>, epoch: u64) {
        if let Ok(mut cache) = self.0.cache.write() {
            if self.epoch() == epoch {
                cache.insert(id, entry);
            }
        }
    }

    async fn load_note_cache(
        &self,
        id: i64,
        model_id: &str,
        epoch: u64,
        pool: &Pool<Sqlite>,
    ) -> Result<(), String> {
        let rows: Vec<(i64, Vec<u8>)> = sqlx::query_as(
            "SELECT ChunkIndex, Vector FROM note_embeddings WHERE NoteId = ? AND ModelId = ? \
             ORDER BY ChunkIndex",
        )
        .bind(id)
        .bind(model_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        let entry = rows
            .into_iter()
            .map(|(index, v)| CachedChunk {
                index,
                vector: decode_vector(&v),
            })
            .collect();
        self.put_cache(id, entry, epoch);
        Ok(())
    }

    /// Fills the cache from stored rows of `model_id` (startup). Rows of JSON notes are
    /// left out.
    pub async fn load_cache(&self, model_id: &str, pool: &Pool<Sqlite>) -> Result<(), String> {
        let epoch = self.epoch();
        let rows: Vec<(i64, i64, Vec<u8>)> = sqlx::query_as(
            "SELECT e.NoteId, e.ChunkIndex, e.Vector FROM note_embeddings e \
             JOIN notes n ON n.id = e.NoteId \
             WHERE e.ModelId = ? AND n.Kind = 'markdown' \
             ORDER BY e.NoteId, e.ChunkIndex",
        )
        .bind(model_id)
        .fetch_all(pool)
        .await
        .map_err(|e| e.to_string())?;
        let mut map: std::collections::HashMap<i64, Vec<CachedChunk>> =
            std::collections::HashMap::new();
        for (note, index, v) in rows {
            map.entry(note).or_default().push(CachedChunk {
                index,
                vector: decode_vector(&v),
            });
        }
        if let Ok(mut cache) = self.0.cache.write() {
            if self.epoch() == epoch {
                *cache = map;
            }
        }
        Ok(())
    }

    /// Processes queued notes until the queue is empty or no model is loaded. Errors on
    /// one note are logged and skipped.
    pub async fn drain(&self, pool: &Pool<Sqlite>) {
        while let Some(id) = self.next_queued() {
            match self.process_note(id, pool).await {
                Ok(Outcome::NotReady) => {
                    self.requeue_front(id);
                    return;
                }
                Ok(_) => {}
                Err(e) => tracing::error!("Semantic index: note {id} skipped: {e}"),
            }
            self.mark_done();
            self.notify();
        }
    }

    /// Queues every markdown note (catch-up: unchanged notes are skipped by hash). JSON
    /// notes are never embedded: their leftover rows (from an older build) are deleted in
    /// one statement and their cache entries dropped instead of queueing each one.
    pub async fn enqueue_all(&self, pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
        let stale: Vec<i64> = sqlx::query_scalar(
            "DELETE FROM note_embeddings \
             WHERE NoteId IN (SELECT id FROM notes WHERE Kind = 'json') RETURNING NoteId",
        )
        .fetch_all(pool)
        .await?;
        for id in stale {
            self.evict_cache(id);
        }
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT id FROM notes WHERE Kind = 'markdown' ORDER BY UpdatedAt DESC, id",
        )
        .fetch_all(pool)
        .await?;
        self.enqueue(ids);
        Ok(())
    }

    /// Drops all vectors (rows and cache) and invalidates in-flight work.
    pub async fn clear_index(&self, pool: &Pool<Sqlite>) -> Result<(), sqlx::Error> {
        self.bump_epoch();
        sqlx::query("DELETE FROM note_embeddings")
            .execute(pool)
            .await?;
        Ok(())
    }

    /// Every indexed note's best chunk against `query`, best first:
    /// (note id, best chunk index, score).
    pub fn best_chunks(&self, query: &[f32]) -> Vec<(i64, i64, f32)> {
        let Ok(cache) = self.0.cache.read() else {
            return Vec::new();
        };
        let mut hits: Vec<(i64, i64, f32)> = cache
            .iter()
            .filter_map(|(id, chunks)| {
                chunks
                    .iter()
                    .map(|c| (c.index, dot(query, &c.vector)))
                    .max_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(index, score)| (*id, index, score))
            })
            .collect();
        hits.sort_by(|a, b| b.2.total_cmp(&a.2).then(a.0.cmp(&b.0)));
        hits
    }

    /// Notes whose best chunk passes [`fusion::similarity_floor`] for `query` (computed over
    /// every indexed note), best first: (note id, best chunk index, score).
    pub fn nearest(&self, query: &[f32]) -> Vec<(i64, i64, f32)> {
        let mut hits = self.best_chunks(query);
        let scores: Vec<f32> = hits.iter().map(|h| h.2).collect();
        let floor = fusion::similarity_floor(&scores);
        hits.retain(|h| h.2 >= floor);
        hits
    }
}

/// Background worker: embeds queued notes as they arrive. Runs for the app's lifetime.
pub async fn run_worker(state: SemanticState, pool: Pool<Sqlite>) {
    loop {
        state.0.wake.notified().await;
        state.drain(&pool).await;
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::commands::notes;
    use crate::semantic::embedder::FakeEmbedder;
    use std::sync::Arc;

    pub(crate) async fn ready_state(pool: &Pool<Sqlite>) -> (SemanticState, Arc<FakeEmbedder>) {
        let _ = pool;
        let state = SemanticState::new(std::path::PathBuf::from("/nonexistent"));
        let fake = Arc::new(FakeEmbedder::new("fake-1"));
        state.set_enabled(true);
        state.set_embedder(Some(fake.clone()));
        (state, fake)
    }

    async fn rows(pool: &Pool<Sqlite>) -> Vec<(i64, i64, String, String)> {
        sqlx::query_as(
            "SELECT NoteId, ChunkIndex, ModelId, ChunkText FROM note_embeddings ORDER BY NoteId, ChunkIndex",
        )
        .fetch_all(pool)
        .await
        .unwrap()
    }

    async fn note(title: &str, body: &str, pool: &Pool<Sqlite>) -> i64 {
        let n = notes::inner_create_note_titled(title, None, pool)
            .await
            .unwrap();
        notes::inner_update_note_body(n.id, body, pool)
            .await
            .unwrap();
        n.id
    }

    #[test]
    fn vectors_round_trip_through_blobs() {
        let v = vec![0.25f32, -1.5, 3.0e-7];
        assert_eq!(decode_vector(&encode_vector(&v)), v);
    }

    #[test]
    fn content_hash_changes_with_title_or_body() {
        let a = content_hash("A", "body");
        assert_eq!(a, content_hash("A", "body"));
        assert_ne!(a, content_hash("B", "body"));
        assert_ne!(a, content_hash("A", "body2"));
        assert_ne!(content_hash("ab", "c"), content_hash("a", "bc"));
    }

    #[tokio::test]
    async fn create_update_rename_enqueue_and_reembed_unchanged_skipped() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let id = note("Budget", "Plan the **budget** for 2027.", &pool).await;
        state.enqueue([id]);
        state.drain(&pool).await;
        assert_eq!(
            rows(&pool).await,
            vec![(id, 0, "fake-1".into(), "Plan the budget for 2027.".into())]
        );
        assert_eq!(fake.embedded(), vec!["Budget\n\nPlan the budget for 2027."]);
        assert_eq!(state.indexed_notes(), 1);

        // Unchanged: skipped.
        state.enqueue([id]);
        state.drain(&pool).await;
        assert_eq!(fake.embedded().len(), 1);

        // Body update re-embeds.
        notes::inner_update_note_body(id, "New text", &pool)
            .await
            .unwrap();
        state.enqueue([id]);
        state.drain(&pool).await;
        assert_eq!(fake.embedded().len(), 2);
        assert_eq!(rows(&pool).await[0].3, "New text");

        // Rename re-embeds (the title is in the prefix).
        notes::inner_rename_note(id, "Budget 2027", &pool)
            .await
            .unwrap();
        state.enqueue([id]);
        state.drain(&pool).await;
        assert_eq!(fake.embedded().last().unwrap(), "Budget 2027\n\nNew text");
        assert_eq!(state.progress(), (0, 0));
    }

    #[tokio::test]
    async fn queue_dedupes_rapid_saves() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let id = note("A", "x", &pool).await;
        state.enqueue([id, id, id]);
        state.enqueue([id]);
        assert_eq!(state.progress(), (0, 1));
        state.drain(&pool).await;
        assert_eq!(fake.embedded().len(), 1);
    }

    #[tokio::test]
    async fn disabled_state_ignores_enqueue_and_unloaded_model_keeps_queue() {
        let pool = crate::test_db().await;
        let id = note("A", "x", &pool).await;
        let state = SemanticState::new("/nonexistent".into());
        state.enqueue([id]);
        assert_eq!(state.progress(), (0, 0));

        state.set_enabled(true);
        state.enqueue([id]);
        state.drain(&pool).await; // no model yet
        assert_eq!(state.progress(), (0, 1));
        assert!(rows(&pool).await.is_empty());

        state.set_embedder(Some(Arc::new(FakeEmbedder::new("fake-1"))));
        state.drain(&pool).await;
        assert_eq!(rows(&pool).await.len(), 1);
    }

    #[tokio::test]
    async fn delete_cascades_rows_and_evicts_cache() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        let b = note("B", "beta", &pool).await;
        state.enqueue([a, b]);
        state.drain(&pool).await;
        assert_eq!(state.indexed_notes(), 2);

        notes::inner_delete_note(a, &pool).await.unwrap();
        state.evict(a);
        assert_eq!(state.indexed_notes(), 1);
        let left: Vec<i64> = rows(&pool).await.into_iter().map(|r| r.0).collect();
        assert_eq!(left, vec![b]);
    }

    #[tokio::test]
    async fn deleted_note_in_queue_is_skipped_not_an_error() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        state.enqueue([a, 999]);
        notes::inner_delete_note(a, &pool).await.unwrap();
        assert_eq!(state.process_note(a, &pool).await, Ok(Outcome::Missing));
        state.drain(&pool).await;
        assert!(fake.embedded().is_empty());
        assert_eq!(state.progress(), (0, 0));
    }

    #[tokio::test]
    async fn model_id_mismatch_reembeds_everything() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        let b = note("B", "beta", &pool).await;
        state.enqueue_all(&pool).await.unwrap();
        state.drain(&pool).await;

        let next = Arc::new(FakeEmbedder::new("fake-2"));
        state.set_embedder(Some(next.clone()));
        state.enqueue_all(&pool).await.unwrap();
        state.drain(&pool).await;
        assert_eq!(next.embedded().len(), 2);
        let models: Vec<String> = rows(&pool).await.into_iter().map(|r| r.2).collect();
        assert_eq!(models, vec!["fake-2", "fake-2"]);
        let _ = (a, b);
    }

    #[tokio::test]
    async fn long_note_embeds_each_chunk_in_its_own_call() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let body: Vec<String> = (0..2000).map(|i| format!("w{i}")).collect();
        let id = note("Long", &body.join(" "), &pool).await;
        state.enqueue([id]);
        state.drain(&pool).await;
        let n = rows(&pool).await.len();
        assert!(n > 2, "{n} chunks");
        assert_eq!(fake.embedded().len(), n);
        // Batch-mates change int8 vectors, so the indexer never sends more than one.
        assert_eq!(*fake.call_sizes.lock().unwrap(), vec![1; n]);
    }

    #[tokio::test]
    async fn empty_and_unicode_notes_index() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let a = notes::inner_create_note(None, &pool).await.unwrap().id;
        let b = note("Möte 🚀", "Åsa räknar på äpplen ✅", &pool).await;
        state.enqueue([a, b]);
        state.drain(&pool).await;
        assert_eq!(
            fake.embedded(),
            vec!["Untitled", "Möte 🚀\n\nÅsa räknar på äpplen ✅"]
        );
    }

    #[tokio::test]
    async fn inference_error_skips_the_note_and_continues() {
        let pool = crate::test_db().await;
        let state = SemanticState::new("/nonexistent".into());
        let mut fake = FakeEmbedder::new("fake-1");
        fake.fail_on = Some("poison".into());
        state.set_enabled(true);
        state.set_embedder(Some(Arc::new(fake)));
        let a = note("A", "poison", &pool).await;
        let b = note("B", "fine", &pool).await;
        state.enqueue([a, b]);
        state.drain(&pool).await;
        let ids: Vec<i64> = rows(&pool).await.into_iter().map(|r| r.0).collect();
        assert_eq!(ids, vec![b]);
        assert_eq!(state.progress(), (0, 0), "status not stuck");
    }

    #[tokio::test]
    async fn epoch_bump_drops_in_flight_result() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        // Simulate a bump between embedding and the write: wrap the embedder.
        struct Bumping(SemanticState, FakeEmbedder);
        impl crate::semantic::embedder::Embedder for Bumping {
            fn model_id(&self) -> &str {
                self.1.model_id()
            }
            fn embed(&self, t: &[String]) -> Result<Vec<Vec<f32>>, String> {
                self.0.bump_epoch();
                self.1.embed(t)
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.1.count_tokens(t)
            }
        }
        state.set_embedder(Some(Arc::new(Bumping(
            state.clone(),
            FakeEmbedder::new("f"),
        ))));
        assert_eq!(state.process_note(a, &pool).await, Ok(Outcome::Stale));
        assert!(rows(&pool).await.is_empty());
        assert_eq!(state.indexed_notes(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn edit_during_embedding_drops_result_and_requeues() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        struct Editing(Pool<Sqlite>, i64, FakeEmbedder);
        impl crate::semantic::embedder::Embedder for Editing {
            fn model_id(&self) -> &str {
                self.2.model_id()
            }
            fn embed(&self, t: &[String]) -> Result<Vec<Vec<f32>>, String> {
                let (pool, id) = (self.0.clone(), self.1);
                // Runs on a blocking thread inside the runtime: an edit lands mid-flight.
                tokio::runtime::Handle::current().block_on(async {
                    sqlx::query("UPDATE notes SET Body = 'edited' WHERE id = ?")
                        .bind(id)
                        .execute(&pool)
                        .await
                        .unwrap();
                });
                self.2.embed(t)
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.2.count_tokens(t)
            }
        }
        state.set_embedder(Some(Arc::new(Editing(
            pool.clone(),
            a,
            FakeEmbedder::new("f"),
        ))));
        assert_eq!(state.process_note(a, &pool).await, Ok(Outcome::Stale));
        assert!(rows(&pool).await.is_empty());
        assert_eq!(state.progress().1, 0);
        assert_eq!(state.next_queued(), Some(a), "re-queued");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn switch_to_json_during_embedding_writes_nothing() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        struct Switching(Pool<Sqlite>, i64, FakeEmbedder);
        impl crate::semantic::embedder::Embedder for Switching {
            fn model_id(&self) -> &str {
                self.2.model_id()
            }
            fn embed(&self, t: &[String]) -> Result<Vec<Vec<f32>>, String> {
                let (pool, id) = (self.0.clone(), self.1);
                // The body is unchanged, so only the kind re-check can stop the write.
                tokio::runtime::Handle::current().block_on(async {
                    notes::inner_set_note_kind(id, "json", None, &pool)
                        .await
                        .unwrap();
                });
                self.2.embed(t)
            }
            fn count_tokens(&self, t: &str) -> usize {
                self.2.count_tokens(t)
            }
        }
        state.set_embedder(Some(Arc::new(Switching(
            pool.clone(),
            a,
            FakeEmbedder::new("f"),
        ))));
        assert_eq!(state.process_note(a, &pool).await, Ok(Outcome::Skipped));
        assert!(rows(&pool).await.is_empty());
        assert_eq!(state.indexed_notes(), 0);
        assert_eq!(state.next_queued(), None, "not re-queued");
    }

    #[tokio::test]
    async fn json_rows_left_by_an_older_build_are_not_loaded_and_get_deleted() {
        let pool = crate::test_db().await;
        let (state, fake) = ready_state(&pool).await;
        let a = note("A", "alpha", &pool).await;
        let b = note("B", "beta", &pool).await;
        state.enqueue([a, b]);
        state.drain(&pool).await;
        // As if an older build embedded b after it became JSON.
        sqlx::query("UPDATE notes SET Kind = 'json' WHERE id = ?")
            .bind(b)
            .execute(&pool)
            .await
            .unwrap();
        let fresh = SemanticState::new("/nonexistent".into());
        fresh.load_cache("fake-1", &pool).await.unwrap();
        assert_eq!(fresh.indexed_notes(), 1);

        let before = fake.embedded().len();
        state.enqueue_all(&pool).await.unwrap();
        // Deleted by enqueue_all itself; only the markdown note is queued.
        assert_eq!(state.queued(), vec![a]);
        let ids: Vec<i64> = rows(&pool).await.into_iter().map(|r| r.0).collect();
        assert_eq!(ids, vec![a]);
        assert_eq!(state.indexed_notes(), 1);
        state.drain(&pool).await;
        assert_eq!(fake.embedded().len(), before);
        let ids: Vec<i64> = rows(&pool).await.into_iter().map(|r| r.0).collect();
        assert_eq!(ids, vec![a]);
        assert_eq!(state.indexed_notes(), 1);
    }

    #[tokio::test]
    async fn startup_cache_load_scores_every_note_by_its_best_chunk() {
        let pool = crate::test_db().await;
        let (state, _) = ready_state(&pool).await;
        let a = note("Budget", "money plan", &pool).await;
        let b = note("Garden", "tomato plants", &pool).await;
        state.enqueue([a, b]);
        state.drain(&pool).await;

        let fresh = SemanticState::new("/nonexistent".into());
        fresh.load_cache("fake-1", &pool).await.unwrap();
        assert_eq!(fresh.indexed_notes(), 2);
        let q = FakeEmbedder::vector("budget money");
        let scored = fresh.best_chunks(&q);
        assert_eq!(scored.len(), 2);
        assert_eq!((scored[0].0, scored[0].1), (a, 0));
        assert!(scored[0].2 > scored[1].2);
        assert_eq!(scored[1].0, b);

        let other = SemanticState::new("/nonexistent".into());
        other.load_cache("other-model", &pool).await.unwrap();
        assert_eq!(other.indexed_notes(), 0);
    }
}
