//! Text embedding: the `Embedder` trait, the Granite ONNX implementation (ONNX Runtime via
//! `ort`) and a deterministic fake for tests.
//!
//! Release builds use `panic = "abort"`, so nothing on the tokenizer or inference path
//! may `unwrap`/`expect`: every failure is an `Err` the caller logs and skips.

use std::borrow::Cow;
use std::path::Path;
use std::sync::Mutex;

use ort::session::builder::{AutoDevicePolicy, GraphOptimizationLevel};
use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

/// Inputs are truncated to this many tokens before inference. Granite accepts 32k, but
/// chunks are ~200 tokens and a longer query only costs time.
pub const MAX_TOKENS: usize = 512;
/// Inference threads: enough to be quick, few enough to leave the app responsive.
const MAX_THREADS: usize = 4;

/// Turns texts into L2-normalised vectors. One instance is shared behind a mutex.
pub trait Embedder: Send + Sync {
    /// Identifies the model (and format) that made a vector; stored with each row.
    fn model_id(&self) -> &str;
    /// Embeds each text (already passed through `query_text`/`passage_text`). A text's
    /// vector must not depend on the other texts in the call.
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String>;
    /// Number of tokens `text` takes, without special tokens.
    fn count_tokens(&self, text: &str) -> usize;
}

/// The model input for a search query. Granite R2 is trained without query or passage
/// prompts, so this (and `passage_text`) is the text itself; it is where a model's
/// prompt would go.
pub fn query_text(query: &str) -> String {
    query.to_string()
}

/// The model input for a note chunk (see `query_text`).
pub fn passage_text(passage: &str) -> String {
    passage.to_string()
}

/// Scales `v` to unit length (a zero vector stays zero).
pub fn l2_normalize(v: &mut [f32]) {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
}

pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// The first (CLS) token's vector of each input, L2-normalised. `hidden` is the flat
/// `[batch, seq, dim]` output.
fn cls_pool(hidden: &[f32], batch: usize, seq: usize, dim: usize) -> Vec<Vec<f32>> {
    (0..batch)
        .map(|b| {
            let mut v = hidden[b * seq * dim..(b * seq + 1) * dim].to_vec();
            l2_normalize(&mut v);
            v
        })
        .collect()
}

/// ONNX Runtime allows one environment per process: made on first load, then reused
/// (Remove model followed by a new download loads the model again).
fn environment() -> ort::Result<ort::environment::Environment> {
    static ENV: Mutex<Option<ort::environment::Environment>> = Mutex::new(None);
    let mut slot = ENV
        .lock()
        .map_err(|_| ort::Error::new("ONNX Runtime environment is unavailable"))?;
    if let Some(env) = slot.as_ref() {
        return Ok(env.clone());
    }
    let env = ort::init().with_name("timetracker").build()?;
    *slot = Some(env.clone());
    Ok(env)
}

/// The Granite (ModernBERT) encoder in ONNX Runtime with its Hugging Face tokenizer: CLS
/// pooling, then L2 normalisation.
///
/// Each text gets its own inference run, even when `embed` is given several. The int8
/// export quantises activations dynamically (`DynamicQuantizeLinear`, one scale per run),
/// so texts batched together change each other's vectors (cosine ~0.97-0.99 against the
/// same text alone): stored vectors would not be reproducible and would drift from
/// queries, which are always embedded alone.
pub struct OrtEmbedder {
    model_id: String,
    tokenizer: tokenizers::Tokenizer,
    /// `Session::run` needs `&mut`; callers already serialise inference.
    session: Mutex<Session>,
    /// Model input names (`input_ids`, `attention_mask`, maybe `token_type_ids`).
    inputs: Vec<String>,
}

impl OrtEmbedder {
    pub fn load(model_path: &Path, tokenizer_path: &Path, model_id: &str) -> Result<Self, String> {
        let mut tokenizer = tokenizers::Tokenizer::from_file(tokenizer_path)
            .map_err(|e| format!("Couldn't load the tokenizer: {e}"))?;
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: MAX_TOKENS,
                ..Default::default()
            }))
            .map_err(|e| format!("Couldn't configure the tokenizer: {e}"))?;
        // Inputs run one at a time, so there is never anything to pad to.
        tokenizer.with_padding(None);
        let threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(2)
            .clamp(1, MAX_THREADS);
        let load = || -> ort::Result<Session> {
            let env = environment()?;
            // CPU only. With `api-22`, ort's default policy registers CoreML on macOS; the
            // CPU provider is deterministic and is what the similarity floor was calibrated
            // on (GPU/NPU providers are out of scope). PreferCPU registers no other
            // provider, and the environment defines none to inherit.
            Session::builder(&env)?
                .with_auto_device(AutoDevicePolicy::PreferCPU)?
                .with_no_environment_execution_providers()?
                .with_optimization_level(GraphOptimizationLevel::Level3)?
                .with_intra_threads(threads)?
                .commit_from_file(model_path)
        };
        let session = load().map_err(|e| format!("Couldn't load the model: {e}"))?;
        let inputs = session
            .inputs()
            .iter()
            .map(|i| i.name().to_string())
            .collect();
        Ok(OrtEmbedder {
            model_id: model_id.to_string(),
            tokenizer,
            session: Mutex::new(session),
            inputs,
        })
    }

    /// One text, one inference run (see the type docs for why never more).
    fn run(&self, text: &str) -> Result<Vec<f32>, String> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| format!("tokenizer: {e}"))?;
        let ids: Vec<i64> = encoding.get_ids().iter().map(|&x| i64::from(x)).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&x| i64::from(x))
            .collect();
        let (batch, seq) = (1, ids.len());
        if seq == 0 || mask.len() != seq {
            return Err("tokenizer returned no tokens".to_string());
        }
        let ort_err = |e: ort::Error| e.to_string();
        let mut feed: Vec<(Cow<'_, str>, SessionInputValue<'_>)> = Vec::new();
        for name in &self.inputs {
            let data = if name.contains("mask") {
                mask.clone()
            } else if name.contains("type") {
                vec![0i64; batch * seq]
            } else {
                ids.clone()
            };
            let tensor = Tensor::from_array(([batch, seq], data)).map_err(ort_err)?;
            feed.push((Cow::Owned(name.clone()), SessionInputValue::from(tensor)));
        }
        let mut session = self
            .session
            .lock()
            .map_err(|_| "the model is unavailable".to_string())?;
        let outputs = session.run(feed).map_err(ort_err)?;
        if outputs.len() == 0 {
            return Err("model returned no output".to_string());
        }
        let (shape, hidden) = outputs[0].try_extract_tensor::<f32>().map_err(ort_err)?;
        let dims: Vec<usize> = shape
            .iter()
            .map(|&d| usize::try_from(d).unwrap_or(0))
            .collect();
        if dims.len() != 3
            || dims[0] != batch
            || dims[1] != seq
            || hidden.len() != batch * seq * dims[2]
        {
            return Err(format!("unexpected output shape {dims:?}"));
        }
        cls_pool(hidden, batch, seq, dims[2])
            .pop()
            .ok_or_else(|| "model returned no vector".to_string())
    }
}

impl Embedder for OrtEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        texts
            .iter()
            .map(|t| self.run(t).map_err(|e| format!("Embedding failed: {e}")))
            .collect()
    }

    fn count_tokens(&self, text: &str) -> usize {
        // Without special tokens and before truncation (the chunker needs the real size).
        match self.tokenizer.encode_fast(text, false) {
            Ok(e) => e.get_ids().len(),
            Err(_) => text.split_whitespace().count(),
        }
    }
}

/// Deterministic test embedder: bag of lowercased words hashed into `DIM` buckets.
/// Texts sharing words score high; disjoint texts score 0. Never touches the network.
#[cfg(test)]
pub struct FakeEmbedder {
    pub model_id: String,
    /// Texts passed to `embed`, in order (for asserting what got (re)embedded).
    pub calls: std::sync::Mutex<Vec<String>>,
    /// How many texts each `embed` call got, in order.
    pub call_sizes: std::sync::Mutex<Vec<usize>>,
    /// `embed` fails for any text containing this marker.
    pub fail_on: Option<String>,
}

#[cfg(test)]
impl FakeEmbedder {
    pub const DIM: usize = 64;

    pub fn new(model_id: &str) -> Self {
        FakeEmbedder {
            model_id: model_id.to_string(),
            calls: std::sync::Mutex::new(Vec::new()),
            call_sizes: std::sync::Mutex::new(Vec::new()),
            fail_on: None,
        }
    }

    pub fn vector(text: &str) -> Vec<f32> {
        let mut v = vec![0f32; Self::DIM];
        for word in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
        {
            let w = word.to_lowercase();
            let h = w
                .bytes()
                .fold(5381u32, |h, b| h.wrapping_mul(33) ^ u32::from(b));
            v[h as usize % Self::DIM] += 1.0;
        }
        l2_normalize(&mut v);
        v
    }

    pub fn embedded(&self) -> Vec<String> {
        self.calls.lock().map(|c| c.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
impl Embedder for FakeEmbedder {
    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        if let Some(marker) = &self.fail_on {
            if texts.iter().any(|t| t.contains(marker.as_str())) {
                return Err("fake failure".to_string());
            }
        }
        if let Ok(mut calls) = self.calls.lock() {
            calls.extend(texts.iter().cloned());
        }
        if let Ok(mut sizes) = self.call_sizes.lock() {
            sizes.push(texts.len());
        }
        Ok(texts.iter().map(|t| Self::vector(t)).collect())
    }

    fn count_tokens(&self, text: &str) -> usize {
        text.split_whitespace().count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l2_normalize_makes_unit_vectors_and_keeps_zero() {
        let mut v = vec![3.0, 4.0];
        l2_normalize(&mut v);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-6);
        let mut z = vec![0.0, 0.0];
        l2_normalize(&mut z);
        assert_eq!(z, vec![0.0, 0.0]);
    }

    #[test]
    fn cls_pool_takes_each_first_token_then_normalises() {
        // batch 2, seq 2, dim 2: only the first token of each input counts.
        let hidden = [3.0, 4.0, 100.0, 0.0, 0.0, 2.0, 0.0, 100.0];
        let v = cls_pool(&hidden, 2, 2, 2);
        assert_eq!(v, vec![vec![0.6, 0.8], vec![0.0, 1.0]]);
    }

    #[test]
    fn inputs_take_no_prompt() {
        assert_eq!(query_text("hej"), "hej");
        assert_eq!(passage_text("hej"), "hej");
    }

    #[test]
    fn fake_embedder_is_deterministic_and_word_based() {
        let e = FakeEmbedder::new("fake");
        let v = e
            .embed(&[passage_text("Budget plan"), query_text("budget")])
            .unwrap();
        assert!(dot(&v[0], &v[1]) > 0.5);
        assert_eq!(FakeEmbedder::vector("budget plan"), v[0]);
        assert_eq!(e.count_tokens("one two  three"), 3);
    }
}
