use async_trait::async_trait;
use atlas_acta::semantic_search::EmbeddingInput;
use atlas_acta::semantic_search::EmbeddingProvider;
use atlas_core::error::DomainError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::config::SearchSemanticConfig;

#[derive(Debug, Clone)]
pub struct DeterministicEmbeddingProvider {
    model: String,
    dimensions: usize,
}

impl DeterministicEmbeddingProvider {
    pub fn new(model: impl Into<String>, dimensions: usize) -> Result<Self, DomainError> {
        if dimensions == 0 {
            return Err(DomainError::InvalidInput {
                message: "embedding dimensions must be greater than zero".to_owned(),
            });
        }
        let model = model.into();
        if model.trim().is_empty() {
            return Err(DomainError::InvalidInput {
                message: "embedding model must not be empty".to_owned(),
            });
        }
        Ok(Self { model, dimensions })
    }

    fn embed_one(&self, text: &str) -> Vec<f32> {
        (0..self.dimensions)
            .map(|idx| {
                let mut hasher = Sha256::new();
                hasher.update(self.model.as_bytes());
                hasher.update([0]);
                hasher.update(text.as_bytes());
                hasher.update(idx.to_le_bytes());
                let digest = hasher.finalize();
                let mut prefix = [0_u8; 4];
                if let Some(bytes) = digest.get(..4) {
                    prefix.copy_from_slice(bytes);
                }
                let raw = u32::from_le_bytes(prefix);
                (raw as f32 / u32::MAX as f32) * 2.0 - 1.0
            })
            .collect()
    }
}

#[async_trait]
impl EmbeddingProvider for DeterministicEmbeddingProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        Ok(inputs
            .iter()
            .map(|input| self.embed_one(&input.text))
            .collect())
    }

    fn model(&self) -> &str {
        &self.model
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }
}

#[derive(Clone)]
pub struct OpenAiCompatibleEmbeddingProvider {
    client: reqwest::Client,
    config: SearchSemanticConfig,
}

impl OpenAiCompatibleEmbeddingProvider {
    pub fn new(config: SearchSemanticConfig) -> Result<Self, DomainError> {
        config
            .validate_for_provider()
            .map_err(|message| DomainError::InvalidInput { message })?;
        let client = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms))
            .build()
            .map_err(|e| DomainError::Internal {
                message: format!("build embedding HTTP client: {e}"),
            })?;
        Ok(Self { client, config })
    }
}

#[derive(Serialize)]
struct EmbeddingRequest<'a> {
    model: &'a str,
    input: Vec<&'a str>,
}

#[derive(Deserialize)]
struct EmbeddingResponse {
    data: Vec<EmbeddingData>,
}

#[derive(Deserialize)]
struct EmbeddingData {
    embedding: Vec<f32>,
}

/// A failed embedding request, split by whether repeating it could help.
///
/// A rejected key or a malformed body fails the same way every time, so only
/// transport failures and the provider's own temporary refusals are worth the
/// caller's `retry_attempts`.
enum EmbeddingAttemptError {
    Transient(DomainError),
    Permanent(DomainError),
}

impl EmbeddingAttemptError {
    fn into_inner(self) -> DomainError {
        match self {
            Self::Transient(error) | Self::Permanent(error) => error,
        }
    }
}

impl OpenAiCompatibleEmbeddingProvider {
    /// Sends one batch, retrying transient failures with exponential backoff.
    async fn embed_batch(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        const BASE_BACKOFF_MS: u64 = 200;

        let mut attempt = 0;
        loop {
            match self.request_batch(inputs).await {
                Ok(vectors) => return Ok(vectors),
                Err(EmbeddingAttemptError::Permanent(error)) => return Err(error),
                Err(error) if attempt >= self.config.retry_attempts => {
                    return Err(error.into_inner());
                }
                Err(_) => {
                    let backoff = BASE_BACKOFF_MS.saturating_mul(1 << attempt.min(5));
                    tokio::time::sleep(Duration::from_millis(backoff)).await;
                    attempt += 1;
                }
            }
        }
    }

    async fn request_batch(
        &self,
        inputs: &[EmbeddingInput],
    ) -> Result<Vec<Vec<f32>>, EmbeddingAttemptError> {
        let api_key = self
            .config
            .api_key
            .as_ref()
            .map(|key| key.expose().as_str())
            .ok_or_else(|| {
                EmbeddingAttemptError::Permanent(DomainError::InvalidInput {
                    message:
                        "ATLAS_EMBEDDINGS_API_KEY is required for openai_compatible embeddings"
                            .to_owned(),
                })
            })?;
        let body = EmbeddingRequest {
            model: &self.config.model,
            input: inputs.iter().map(|input| input.text.as_str()).collect(),
        };
        let url = format!("{}/embeddings", self.config.base_url.trim_end_matches('/'));
        let response = self
            .client
            .post(url)
            .bearer_auth(api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                EmbeddingAttemptError::Transient(DomainError::Internal {
                    message: format!("embedding request failed: {e}"),
                })
            })?;

        let status = response.status();
        if !status.is_success() {
            let error = DomainError::Internal {
                message: format!("embedding provider returned {status}"),
            };
            return Err(
                if status.is_server_error() || status == reqwest::StatusCode::TOO_MANY_REQUESTS {
                    EmbeddingAttemptError::Transient(error)
                } else {
                    EmbeddingAttemptError::Permanent(error)
                },
            );
        }

        let parsed: EmbeddingResponse = response.json().await.map_err(|e| {
            EmbeddingAttemptError::Transient(DomainError::Internal {
                message: format!("parse embedding response: {e}"),
            })
        })?;
        if parsed.data.len() != inputs.len() {
            return Err(EmbeddingAttemptError::Permanent(DomainError::Internal {
                message: format!(
                    "embedding provider returned {} vectors for {} inputs",
                    parsed.data.len(),
                    inputs.len()
                ),
            }));
        }

        let vectors: Vec<Vec<f32>> = parsed.data.into_iter().map(|item| item.embedding).collect();
        for vector in &vectors {
            if vector.len() != self.config.dimensions {
                return Err(EmbeddingAttemptError::Permanent(DomainError::Internal {
                    message: format!(
                        "embedding dimension mismatch: expected {}, got {}",
                        self.config.dimensions,
                        vector.len()
                    ),
                }));
            }
        }
        Ok(vectors)
    }
}

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let batch_size = self.config.batch_size.max(1);
        let mut vectors = Vec::with_capacity(inputs.len());
        for batch in inputs.chunks(batch_size) {
            vectors.extend(self.embed_batch(batch).await?);
        }
        Ok(vectors)
    }

    fn model(&self) -> &str {
        &self.config.model
    }

    fn dimensions(&self) -> usize {
        self.config.dimensions
    }
}
