//! LLM + embeddings clients.
//!
//! * [`LlmClient`] wraps rig's xAI provider client for chat/completions.
//! * [`Embedder`] is a small trait over batch embedding; [`XaiEmbedder`]
//!   hits `POST {base_url}/embeddings` directly via reqwest because
//!   `rig-core` 0.35 does not expose embeddings for its xAI provider.
//! * [`MockEmbedder`] is a deterministic test double (hash-to-vector).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use rig::providers::openai;
use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::http_retry::RetryingHttp;

/// Shared LLM client + the resolved model name.
///
/// We talk to xAI through rig's **OpenAI chat-completions** client
/// (xAI is OpenAI-compatible at `/v1/chat/completions`) rather than
/// rig's dedicated xAI client. Reason: rig's xAI provider uses the
/// `/v1/responses` endpoint and its streaming parser only recognises
/// `response.reasoning_summary_text.delta` events, but xAI actually
/// emits `response.reasoning_text.delta` for reasoning models, so all
/// reasoning tokens get silently dropped. The OpenAI chat-completions
/// streaming parser handles `reasoning_content` deltas correctly,
/// which xAI does emit on that endpoint.
///
/// Requests go through [`RetryingHttp`], so a transient 5xx or 429 from
/// xAI is retried instead of failing the whole agent run.
#[derive(Clone)]
pub struct LlmClient {
    pub config: Config,
    pub client: openai::CompletionsClient<RetryingHttp>,
    pub model: String,
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("model", &self.model)
            .field("base_url", &self.config.base_url)
            .field("xai_api_key_set", &self.config.xai_api_key.is_some())
            .finish()
    }
}

impl LlmClient {
    pub fn new(config: Config) -> Result<Self> {
        let api_key = config.require_api_key()?.to_string();
        let base = config.base_url.trim_end_matches('/').to_string();
        let client = openai::CompletionsClient::builder()
            .http_client(RetryingHttp::default())
            .api_key(&api_key)
            .base_url(&base)
            .build()
            .context("building xAI (openai-compat) client")?;
        let model = config.model.clone();
        Ok(Self {
            config,
            client,
            model,
        })
    }

    /// Test-only constructor: succeeds with a dummy key even when
    /// `MAPERTUI_XAI_TOKEN` is unset.
    pub fn dummy() -> Result<Self> {
        let mut config = Config::from_env();
        if config.xai_api_key.is_none() {
            config.xai_api_key = Some("dummy-key".into());
        }
        Self::new(config)
    }
}

// -- Embeddings -------------------------------------------------------------

/// Batch-capable embedder. Implementations must return one vector per input.
// `async_trait` adds its own `#[must_use]`, which clippy flags as doubled.
#[allow(clippy::double_must_use)]
#[async_trait]
pub trait Embedder: Send + Sync + std::fmt::Debug {
    /// Dimensionality of the produced vectors.
    fn dims(&self) -> usize;
    /// Name identifying the backing model (for diagnostics).
    fn model(&self) -> &str;
    /// Embed a batch of texts. `Ok(v)` has `v.len() == texts.len()`.
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

/// Convenience: embed a single text.
pub async fn embed_one(embedder: &dyn Embedder, text: &str) -> Result<Vec<f32>> {
    let mut v = embedder.embed(&[text.to_string()]).await?;
    v.pop()
        .ok_or_else(|| anyhow!("embedder returned zero vectors for one input"))
}

#[derive(Clone, Debug)]
pub struct XaiEmbedder {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
    model: String,
    dims: usize,
}

impl XaiEmbedder {
    /// Build from resolved config. Returns an error if no API key is set.
    pub fn from_config(cfg: &Config) -> Result<Self> {
        let api_key = cfg.require_api_key()?.to_string();
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("building reqwest client for embeddings")?;
        Ok(Self {
            http,
            base_url: cfg.base_url.trim_end_matches('/').to_string(),
            api_key,
            model: cfg.embedding_model.clone(),
            dims: cfg.embedding_dims,
        })
    }

    /// Convenience: wrap as `Arc<dyn Embedder>`.
    pub fn into_arc(self) -> Arc<dyn Embedder> {
        Arc::new(self)
    }
}

#[derive(Serialize)]
struct EmbReq<'a> {
    model: &'a str,
    input: &'a [String],
}

#[derive(Deserialize)]
struct EmbResp {
    data: Vec<EmbDatum>,
}

#[derive(Deserialize)]
struct EmbDatum {
    embedding: Vec<f32>,
}

#[async_trait]
impl Embedder for XaiEmbedder {
    fn dims(&self) -> usize {
        self.dims
    }
    fn model(&self) -> &str {
        &self.model
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let url = format!("{}/embeddings", self.base_url);
        let body = EmbReq {
            model: &self.model,
            input: texts,
        };
        // Brief retry on transient upstream errors (xAI occasionally 502/503).
        let delays_ms = [0u64, 750, 2000, 5000];
        let mut last_err: Option<anyhow::Error> = None;
        for delay_ms in delays_ms {
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            match self.try_embed(&url, &body, texts.len()).await {
                Ok(v) => return Ok(v),
                Err(e) => {
                    let msg = format!("{e:#}");
                    let transient = msg.contains("502")
                        || msg.contains("503")
                        || msg.contains("504")
                        || msg.contains("timeout")
                        || msg.contains("Connection refused")
                        || msg.contains("reset");
                    last_err = Some(e);
                    if !transient {
                        break;
                    }
                }
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("embed failed without captured error")))
    }
}

impl XaiEmbedder {
    async fn try_embed(
        &self,
        url: &str,
        body: &EmbReq<'_>,
        expected_len: usize,
    ) -> Result<Vec<Vec<f32>>> {
        let resp = self
            .http
            .post(url)
            .bearer_auth(&self.api_key)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        let bytes = resp.bytes().await.context("reading embeddings response")?;
        if !status.is_success() {
            let snippet = String::from_utf8_lossy(&bytes);
            return Err(anyhow!(
                "xAI embeddings returned {status}: {}",
                snippet.chars().take(500).collect::<String>()
            ));
        }
        let parsed: EmbResp = serde_json::from_slice(&bytes).with_context(|| {
            format!(
                "parsing embeddings response: {}",
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(500)
                    .collect::<String>()
            )
        })?;
        if parsed.data.len() != expected_len {
            return Err(anyhow!(
                "embeddings response had {} vectors for {} inputs",
                parsed.data.len(),
                expected_len
            ));
        }
        if let Some(first) = parsed.data.first() {
            if first.embedding.len() != self.dims {
                return Err(anyhow!(
                    "embedding dims mismatch: config says {} but model {} returned {}",
                    self.dims,
                    self.model,
                    first.embedding.len()
                ));
            }
        }
        Ok(parsed.data.into_iter().map(|d| d.embedding).collect())
    }
}

/// Deterministic hash-based embedder used by offline tests.
#[derive(Clone, Debug)]
pub struct MockEmbedder {
    dims: usize,
}

impl MockEmbedder {
    pub fn new(dims: usize) -> Self {
        Self { dims }
    }

    /// Simple bag-of-lowercase-alphanumeric-tokens vector.
    /// Each token hashes to a fixed index, contributes `+1.0`.
    fn tokens_to_vec(&self, text: &str) -> Vec<f32> {
        let mut v = vec![0.0f32; self.dims];
        for tok in text
            .split(|c: char| !c.is_alphanumeric())
            .filter(|t| !t.is_empty())
        {
            let t = tok.to_ascii_lowercase();
            let idx = (blake3::hash(t.as_bytes()).as_bytes()[0] as usize
                ^ (blake3::hash(t.as_bytes()).as_bytes()[1] as usize).wrapping_mul(257))
                % self.dims;
            v[idx] += 1.0;
        }
        // Normalise so cosine behaves.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in v.iter_mut() {
                *x /= norm;
            }
        }
        v
    }
}

#[async_trait]
impl Embedder for MockEmbedder {
    fn dims(&self) -> usize {
        self.dims
    }
    fn model(&self) -> &str {
        "mock-embedder"
    }
    async fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|t| self.tokens_to_vec(t)).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_builds_from_dummy_key() {
        let c = LlmClient::dummy().unwrap();
        assert!(!c.model.is_empty());
    }

    #[test]
    fn missing_key_is_a_clear_error() {
        let prev = std::env::var("MAPERTUI_XAI_TOKEN").ok();
        std::env::remove_var("MAPERTUI_XAI_TOKEN");
        let cfg = Config::from_env();
        let err = LlmClient::new(cfg).unwrap_err();
        assert!(format!("{err}")
            .to_lowercase()
            .contains("mapertui_xai_token"));
        if let Some(v) = prev {
            std::env::set_var("MAPERTUI_XAI_TOKEN", v);
        }
    }

    #[tokio::test]
    async fn mock_embedder_deterministic_and_normalised() {
        let e = MockEmbedder::new(64);
        let v1 = embed_one(&e, "hello world").await.unwrap();
        let v2 = embed_one(&e, "hello world").await.unwrap();
        assert_eq!(v1, v2);
        let norm: f32 = v1.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[tokio::test]
    async fn mock_embedder_batches_match_singles() {
        let e = MockEmbedder::new(32);
        let batch = e
            .embed(&["foo bar".into(), "baz quux".into()])
            .await
            .unwrap();
        let a = embed_one(&e, "foo bar").await.unwrap();
        let b = embed_one(&e, "baz quux").await.unwrap();
        assert_eq!(batch[0], a);
        assert_eq!(batch[1], b);
    }
}
