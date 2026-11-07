//! The pinned embedding model: where it comes from, where it lives, and how it gets there.
//!
//! Files are fetched with plain GETs from Hugging Face at a pinned commit, streamed to
//! `<file>.part` while hashing, verified against the SHA-256 below, then renamed. Any
//! failure or cancel removes every model file, so a half-downloaded model never loads.

use sha2::{Digest, Sha256};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::io::AsyncWriteExt;

/// Recorded with every vector; a change re-embeds all notes.
pub const MODEL_ID: &str = "multilingual-e5-base-int8";
pub const MODEL_NAME: &str = "multilingual-e5-base (int8)";
pub const MODEL_FILE: &str = "model_quantized.onnx";
pub const TOKENIZER_FILE: &str = "tokenizer.json";
const REPO: &str = "Xenova/multilingual-e5-base";
const COMMIT: &str = "1ec9243030a27d1a115d5c340572074c125b58b2";
const PART_SUFFIX: &str = ".part";
/// How often a pending network read re-checks the cancel flag.
const CANCEL_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// A file of the model: local name, path in the repo, size and SHA-256.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RemoteFile {
    pub local: &'static str,
    pub remote: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const FILES: [RemoteFile; 2] = [
    RemoteFile {
        local: TOKENIZER_FILE,
        remote: "tokenizer.json",
        size: 17_082_660,
        sha256: "62c24cdc13d4c9952d63718d6c9fa4c287974249e16b7ade6d5a85e7bbb75626",
    },
    RemoteFile {
        local: MODEL_FILE,
        remote: "onnx/model_quantized.onnx",
        size: 278_647_662,
        sha256: "df7a9a29309e3ad491e1783adf8baee710262cc06079c7cbab63c630277fac94",
    },
];

pub fn url(file: &RemoteFile) -> String {
    format!(
        "https://huggingface.co/{REPO}/resolve/{COMMIT}/{}",
        file.remote
    )
}

/// `<models_dir>/<MODEL_ID>/`
pub fn model_dir(models_dir: &Path) -> PathBuf {
    models_dir.join(MODEL_ID)
}

fn part_path(path: &Path) -> PathBuf {
    let mut p = path.as_os_str().to_owned();
    p.push(PART_SUFFIX);
    PathBuf::from(p)
}

/// All model files are present at their expected size.
pub fn model_present(dir: &Path, files: &[RemoteFile]) -> bool {
    files.iter().all(|f| {
        std::fs::metadata(dir.join(f.local))
            .map(|m| m.len() == f.size)
            .unwrap_or(false)
    })
}

/// Bytes on disk under `dir` (0 when missing).
pub fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// Deletes stale `.part` files (a download interrupted by quitting).
pub fn clean_partial(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.to_string_lossy().ends_with(PART_SUFFIX) {
            if let Err(e) = std::fs::remove_file(&path) {
                tracing::warn!("Couldn't remove stale download {path:?}: {e}");
            }
        }
    }
}

/// Deletes the model directory and everything in it.
pub fn remove_model(dir: &Path) -> std::io::Result<()> {
    match std::fs::remove_dir_all(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

/// A response body read chunk by chunk.
pub trait Body: Send {
    fn chunk(&mut self) -> impl Future<Output = Result<Option<Vec<u8>>, String>> + Send;
}

/// Where model files come from: Hugging Face in the app, an in-memory fake in tests.
pub trait Source: Sync {
    type Body: Body;
    fn get(&self, url: &str) -> impl Future<Output = Result<Self::Body, String>> + Send;
}

/// GETs over HTTPS (rustls); follows Hugging Face's redirects to its CDN.
pub struct HttpSource {
    client: reqwest::Client,
}

impl HttpSource {
    pub fn new() -> Result<Self, String> {
        let client = reqwest::Client::builder()
            .https_only(true)
            .connect_timeout(std::time::Duration::from_secs(20))
            .read_timeout(std::time::Duration::from_secs(60))
            .user_agent(concat!("TimeTracker/", env!("CARGO_PKG_VERSION")))
            .build()
            .map_err(|e| format!("Couldn't start the download: {e}"))?;
        Ok(HttpSource { client })
    }
}

pub struct HttpBody(reqwest::Response);

impl Body for HttpBody {
    async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
        self.0
            .chunk()
            .await
            .map(|c| c.map(|b| b.to_vec()))
            .map_err(|_| {
                "The download was interrupted. Check your connection and retry.".to_string()
            })
    }
}

impl Source for HttpSource {
    type Body = HttpBody;

    async fn get(&self, url: &str) -> Result<HttpBody, String> {
        let response = self.client.get(url).send().await.map_err(|_| {
            "Couldn't reach huggingface.co. Check your connection and retry.".to_string()
        })?;
        let status = response.status();
        if !status.is_success() {
            return Err(format!(
                "The download failed (HTTP {}). Retry later.",
                status.as_u16()
            ));
        }
        Ok(HttpBody(response))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadError {
    Cancelled,
    Failed(String),
}

/// Downloads `files` into `dir`, reporting (bytes done, bytes total). Files already
/// present and verified are kept. On failure or cancel every file in `files` (and its
/// `.part`) is removed.
pub async fn download<S: Source>(
    source: &S,
    files: &[RemoteFile],
    dir: &Path,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), DownloadError> {
    let result = download_inner(source, files, dir, cancel, progress).await;
    if result.is_err() {
        for f in files {
            let path = dir.join(f.local);
            let _ = tokio::fs::remove_file(part_path(&path)).await;
            let _ = tokio::fs::remove_file(&path).await;
        }
    }
    result
}

async fn download_inner<S: Source>(
    source: &S,
    files: &[RemoteFile],
    dir: &Path,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), DownloadError> {
    let save_err =
        |e: std::io::Error| DownloadError::Failed(format!("Couldn't save the model: {e}"));
    tokio::fs::create_dir_all(dir).await.map_err(save_err)?;
    let total: u64 = files.iter().map(|f| f.size).sum();
    let mut done = 0u64;
    progress(done, total);
    for f in files {
        let path = dir.join(f.local);
        if verified(&path, f).await {
            done += f.size;
            progress(done, total);
            continue;
        }
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        let part = part_path(&path);
        let mut body = until_cancelled(source.get(&url(f)), cancel)
            .await?
            .map_err(DownloadError::Failed)?;
        let mut out = tokio::fs::File::create(&part).await.map_err(save_err)?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        while let Some(bytes) = until_cancelled(body.chunk(), cancel)
            .await?
            .map_err(DownloadError::Failed)?
        {
            written += bytes.len() as u64;
            if written > f.size {
                return Err(mismatch());
            }
            hasher.update(&bytes);
            out.write_all(&bytes).await.map_err(save_err)?;
            done += bytes.len() as u64;
            progress(done, total);
        }
        out.flush().await.map_err(save_err)?;
        out.sync_all().await.map_err(save_err)?;
        drop(out);
        if written != f.size || hex(&hasher.finalize()) != f.sha256 {
            return Err(mismatch());
        }
        tokio::fs::rename(&part, &path).await.map_err(save_err)?;
    }
    Ok(())
}

/// Runs `fut` until it finishes or `cancel` is set (checked every `CANCEL_POLL`), so a
/// stalled connection doesn't hold Cancel (or a remove waiting on it) until the read
/// timeout.
async fn until_cancelled<T>(
    fut: impl Future<Output = T>,
    cancel: &AtomicBool,
) -> Result<T, DownloadError> {
    tokio::pin!(fut);
    loop {
        if cancel.load(Ordering::SeqCst) {
            return Err(DownloadError::Cancelled);
        }
        tokio::select! {
            out = &mut fut => return Ok(out),
            _ = tokio::time::sleep(CANCEL_POLL) => {}
        }
    }
}

fn mismatch() -> DownloadError {
    DownloadError::Failed(
        "The downloaded model didn't match its checksum. Retry the download.".to_string(),
    )
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The file exists with the expected size and hash.
async fn verified(path: &Path, f: &RemoteFile) -> bool {
    let path = path.to_path_buf();
    let expected = f.sha256;
    let size = f.size;
    tokio::task::spawn_blocking(move || {
        let Ok(meta) = std::fs::metadata(&path) else {
            return false;
        };
        if meta.len() != size {
            return false;
        }
        let Ok(mut file) = std::fs::File::open(&path) else {
            return false;
        };
        let mut hasher = Sha256::new();
        if std::io::copy(&mut file, &mut hasher).is_err() {
            return false;
        }
        hex(&hasher.finalize()) == expected
    })
    .await
    .unwrap_or(false)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Unique temp directory, removed on drop.
    pub(crate) struct TempDir(pub PathBuf);

    impl TempDir {
        pub fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let dir = std::env::temp_dir()
                .join(format!("timetracker-{tag}-{}-{nanos}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Serves fixed bytes per URL in small chunks; unknown URLs fail like HTTP 404.
    #[derive(Default)]
    pub(crate) struct FakeSource {
        pub files: HashMap<String, Vec<u8>>,
        /// Fail the body after this many chunks (a dropped connection).
        pub break_after: Option<usize>,
        /// Set the cancel flag after this many chunks.
        pub cancel_after: Option<(usize, &'static AtomicBool)>,
        /// Never answer after this many chunks (a stalled connection).
        pub stall_after: Option<usize>,
        pub requested: Mutex<Vec<String>>,
    }

    pub(crate) struct FakeBody {
        data: Vec<u8>,
        pos: usize,
        n: usize,
        break_after: Option<usize>,
        cancel_after: Option<(usize, &'static AtomicBool)>,
        stall_after: Option<usize>,
    }

    impl Body for FakeBody {
        async fn chunk(&mut self) -> Result<Option<Vec<u8>>, String> {
            if Some(self.n) == self.stall_after {
                std::future::pending::<()>().await;
            }
            if Some(self.n) == self.break_after {
                return Err("The download was interrupted.".into());
            }
            if let Some((n, flag)) = self.cancel_after {
                if self.n == n {
                    flag.store(true, Ordering::SeqCst);
                }
            }
            self.n += 1;
            if self.pos >= self.data.len() {
                return Ok(None);
            }
            let end = (self.pos + 4).min(self.data.len());
            let out = self.data[self.pos..end].to_vec();
            self.pos = end;
            Ok(Some(out))
        }
    }

    impl Source for FakeSource {
        type Body = FakeBody;
        async fn get(&self, url: &str) -> Result<FakeBody, String> {
            self.requested.lock().unwrap().push(url.to_string());
            let data = self
                .files
                .get(url)
                .cloned()
                .ok_or_else(|| "The download failed (HTTP 404). Retry later.".to_string())?;
            Ok(FakeBody {
                data,
                pos: 0,
                n: 0,
                break_after: self.break_after,
                cancel_after: self.cancel_after,
                stall_after: self.stall_after,
            })
        }
    }

    pub(crate) fn sha(data: &[u8]) -> &'static str {
        Box::leak(hex(&Sha256::digest(data)).into_boxed_str())
    }

    pub(crate) fn fake_files() -> (Vec<RemoteFile>, FakeSource) {
        let tok = b"{\"tokenizer\": true}".to_vec();
        let model = b"onnx model bytes, 32 of them...".to_vec();
        let files = vec![
            RemoteFile {
                local: TOKENIZER_FILE,
                remote: "tokenizer.json",
                size: tok.len() as u64,
                sha256: sha(&tok),
            },
            RemoteFile {
                local: MODEL_FILE,
                remote: "onnx/model_quantized.onnx",
                size: model.len() as u64,
                sha256: sha(&model),
            },
        ];
        let mut source = FakeSource::default();
        source.files.insert(url(&files[0]), tok);
        source.files.insert(url(&files[1]), model);
        (files, source)
    }

    fn listing(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|e| {
                e.filter_map(Result::ok)
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    fn no_progress(_: u64, _: u64) {}

    #[test]
    fn urls_pin_the_commit_and_files_are_https_hugging_face() {
        for f in &FILES {
            let u = url(f);
            assert!(u.starts_with("https://huggingface.co/Xenova/multilingual-e5-base/resolve/"));
            assert!(u.contains(COMMIT));
            assert_eq!(f.sha256.len(), 64);
        }
        let total: u64 = FILES.iter().map(|f| f.size).sum();
        assert!(total <= 700_000_000, "download cap is 0.7 GB");
    }

    #[tokio::test]
    async fn downloads_verifies_renames_and_reports_progress() {
        let tmp = TempDir::new("dl-ok");
        let dir = tmp.0.join("m");
        let (files, source) = fake_files();
        let seen = Mutex::new(Vec::new());
        let progress = |d: u64, t: u64| seen.lock().unwrap().push((d, t));
        download(&source, &files, &dir, &AtomicBool::new(false), &progress)
            .await
            .unwrap();
        assert_eq!(listing(&dir), vec![MODEL_FILE, TOKENIZER_FILE]);
        assert!(model_present(&dir, &files));
        let seen = seen.lock().unwrap();
        let total = files[0].size + files[1].size;
        assert_eq!(seen.first(), Some(&(0, total)));
        assert_eq!(seen.last(), Some(&(total, total)));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));

        // A second run keeps verified files and fetches nothing.
        source.requested.lock().unwrap().clear();
        download(&source, &files, &dir, &AtomicBool::new(false), &no_progress)
            .await
            .unwrap();
        assert!(source.requested.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn hash_mismatch_leaves_no_model_file() {
        let tmp = TempDir::new("dl-hash");
        let (mut files, mut source) = fake_files();
        files[1].sha256 = sha(b"something else");
        source
            .files
            .insert(url(&files[1]), b"onnx model bytes, 32 of them...".to_vec());
        let err = download(
            &source,
            &files,
            &tmp.0,
            &AtomicBool::new(false),
            &no_progress,
        )
        .await
        .unwrap_err();
        assert_eq!(err, mismatch());
        assert!(listing(&tmp.0).is_empty(), "{:?}", listing(&tmp.0));
    }

    #[tokio::test]
    async fn oversized_body_is_a_mismatch() {
        let tmp = TempDir::new("dl-size");
        let (mut files, _) = fake_files();
        files[0].size = 3;
        let (_, source) = fake_files();
        let err = download(
            &source,
            &files,
            &tmp.0,
            &AtomicBool::new(false),
            &no_progress,
        )
        .await
        .unwrap_err();
        assert_eq!(err, mismatch());
        assert!(listing(&tmp.0).is_empty());
    }

    #[tokio::test]
    async fn http_error_and_dropped_connection_leave_no_model_file() {
        let tmp = TempDir::new("dl-http");
        let (files, mut source) = fake_files();
        source.files.remove(&url(&files[1]));
        let err = download(
            &source,
            &files,
            &tmp.0,
            &AtomicBool::new(false),
            &no_progress,
        )
        .await
        .unwrap_err();
        assert_eq!(
            err,
            DownloadError::Failed("The download failed (HTTP 404). Retry later.".into())
        );
        assert!(listing(&tmp.0).is_empty(), "tokenizer removed too");

        let (files, mut source) = fake_files();
        source.break_after = Some(2);
        let err = download(
            &source,
            &files,
            &tmp.0,
            &AtomicBool::new(false),
            &no_progress,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, DownloadError::Failed(_)));
        assert!(listing(&tmp.0).is_empty());
    }

    #[tokio::test]
    async fn cancel_mid_download_leaves_no_model_file() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let tmp = TempDir::new("dl-cancel");
        let (files, mut source) = fake_files();
        source.cancel_after = Some((2, &CANCEL));
        let err = download(&source, &files, &tmp.0, &CANCEL, &no_progress)
            .await
            .unwrap_err();
        assert_eq!(err, DownloadError::Cancelled);
        assert!(listing(&tmp.0).is_empty());
    }

    #[tokio::test]
    async fn cancel_during_a_stalled_read_returns_promptly() {
        static CANCEL: AtomicBool = AtomicBool::new(false);
        let tmp = TempDir::new("dl-stall");
        let (files, mut source) = fake_files();
        // The first file's body stalls after two chunks: its .part already holds bytes.
        source.stall_after = Some(2);
        let part = part_path(&tmp.0.join(TOKENIZER_FILE));
        let started = std::time::Instant::now();
        let canceller = tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(150)).await;
            let part_written = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            CANCEL.store(true, Ordering::SeqCst);
            part_written
        });
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            download(&source, &files, &tmp.0, &CANCEL, &no_progress),
        )
        .await
        .expect("cancel must not wait for the 60 s read timeout");
        assert_eq!(result.unwrap_err(), DownloadError::Cancelled);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(canceller.await.unwrap(), 8, "stalled mid-body");
        assert!(listing(&tmp.0).is_empty(), "{:?}", listing(&tmp.0));
    }

    #[tokio::test]
    async fn write_error_is_reported_and_cleaned_up() {
        let tmp = TempDir::new("dl-write");
        // The target "directory" is a file: nothing can be written below it.
        let blocker = tmp.0.join("blocked");
        std::fs::write(&blocker, b"x").unwrap();
        let (files, source) = fake_files();
        let err = download(
            &source,
            &files,
            &blocker.join("m"),
            &AtomicBool::new(false),
            &no_progress,
        )
        .await
        .unwrap_err();
        assert!(
            matches!(&err, DownloadError::Failed(m) if m.starts_with("Couldn't save the model")),
            "{err:?}"
        );
    }

    /// Opt-in, needs network: GETs the pinned files from Hugging Face into
    /// `TIMETRACKER_DOWNLOAD_DIR` and verifies them (redirects, TLS, hashes).
    /// `TIMETRACKER_DOWNLOAD_DIR=/tmp/x cargo test -p time-tracker real_download -- --ignored`
    #[tokio::test]
    #[ignore = "downloads ~296 MB; see doc comment"]
    async fn real_download_from_hugging_face() {
        let dir = PathBuf::from(
            std::env::var_os("TIMETRACKER_DOWNLOAD_DIR").expect("set TIMETRACKER_DOWNLOAD_DIR"),
        );
        let source = HttpSource::new().unwrap();
        download(&source, &FILES, &dir, &AtomicBool::new(false), &|_, _| {})
            .await
            .unwrap();
        assert!(model_present(&dir, &FILES));
    }

    #[test]
    fn clean_partial_and_remove_model() {
        let tmp = TempDir::new("files");
        let dir = tmp.0.join("m");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model_quantized.onnx.part"), b"half").unwrap();
        std::fs::write(dir.join(TOKENIZER_FILE), b"tok").unwrap();
        clean_partial(&dir);
        assert_eq!(listing(&dir), vec![TOKENIZER_FILE]);
        assert_eq!(dir_bytes(&dir), 3);
        clean_partial(&tmp.0.join("missing")); // no-op

        remove_model(&dir).unwrap();
        assert!(!dir.exists());
        remove_model(&dir).unwrap(); // idempotent
        assert_eq!(dir_bytes(&dir), 0);
    }
}
