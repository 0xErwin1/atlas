use async_trait::async_trait;
use atlas_domain::{
    DomainError,
    semantic_search::{EmbeddingInput, EmbeddingProvider},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::time::Duration;

use crate::config::EmbeddingConfig;

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
    config: EmbeddingConfig,
}

impl OpenAiCompatibleEmbeddingProvider {
    pub fn new(config: EmbeddingConfig) -> Result<Self, DomainError> {
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

#[async_trait]
impl EmbeddingProvider for OpenAiCompatibleEmbeddingProvider {
    async fn embed(&self, inputs: &[EmbeddingInput]) -> Result<Vec<Vec<f32>>, DomainError> {
        if inputs.is_empty() {
            return Ok(Vec::new());
        }

        let api_key = self
            .config
            .api_key
            .as_deref()
            .ok_or_else(|| DomainError::InvalidInput {
                message: "ATLAS_EMBEDDINGS_API_KEY is required for openai_compatible embeddings"
                    .to_owned(),
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
            .map_err(|e| DomainError::Internal {
                message: format!("embedding request failed: {e}"),
            })?;

        if !response.status().is_success() {
            return Err(DomainError::Internal {
                message: format!("embedding provider returned {}", response.status()),
            });
        }

        let parsed: EmbeddingResponse =
            response.json().await.map_err(|e| DomainError::Internal {
                message: format!("parse embedding response: {e}"),
            })?;
        if parsed.data.len() != inputs.len() {
            return Err(DomainError::Internal {
                message: format!(
                    "embedding provider returned {} vectors for {} inputs",
                    parsed.data.len(),
                    inputs.len()
                ),
            });
        }

        let vectors: Vec<Vec<f32>> = parsed.data.into_iter().map(|item| item.embedding).collect();
        for vector in &vectors {
            if vector.len() != self.config.dimensions {
                return Err(DomainError::Internal {
                    message: format!(
                        "embedding dimension mismatch: expected {}, got {}",
                        self.config.dimensions,
                        vector.len()
                    ),
                });
            }
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
