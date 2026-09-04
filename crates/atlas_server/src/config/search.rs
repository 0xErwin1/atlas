//! `search.postgres_fts` / `search.pgvector_embeddings` Module configuration.
//!
//! `SearchLexicalConfig` (registry declaration: `ConfigDeclaration::new(
//! "SearchLexicalConfig", "ATLAS_SEARCH_", true)`) and `SearchSemanticConfig`
//! (registry declaration: `ConfigDeclaration::new("SearchSemanticConfig",
//! "ATLAS_EMBEDDINGS_", false)`) are the split of today's `EmbeddingConfig`/
//! `SearchConfig` into their owning Modules (design D1.3). Per the
//! orchestrator's E11-S1 decision, `ATLAS_SEARCH_RRF_K`/
//! `ATLAS_SEARCH_HYBRID_POOL` fusion tuning moves onto
//! `SearchSemanticConfig` — RRF only runs when semantic search is present —
//! leaving `SearchLexicalConfig` with no fields of its own after this move.

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

use super::{EmbeddingConfig, EmbeddingProviderKind, load_search_config};

/// Typed configuration owned by `search.postgres_fts`. Carries no fields:
/// every V1 variable this Module could plausibly own
/// (`ATLAS_SEARCH_RRF_K`/`ATLAS_SEARCH_HYBRID_POOL`) moved to
/// [`SearchSemanticConfig`] instead, per the orchestrator's decision that
/// fusion tuning belongs with the module that gates it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SearchLexicalConfig;

impl ComponentConfig for SearchLexicalConfig {
    fn from_env(_source: &dyn EnvSource) -> Result<Self, ConfigError> {
        Ok(Self)
    }
}

/// Typed configuration owned by `search.pgvector_embeddings`: the
/// embeddings provider plus hybrid-search fusion tuning.
#[derive(Clone, Debug)] // safe: `api_key` is `Option<Secret<String>>`.
pub struct SearchSemanticConfig {
    pub enabled: bool,
    pub provider: EmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub api_key: Option<Secret<String>>,
    pub base_url: String,
    pub batch_size: usize,
    pub timeout_ms: u64,
    pub retry_attempts: u32,
    /// RRF damping constant. Smaller lets one arm's top hit dominate; larger
    /// weighs the two arms more evenly.
    pub rrf_k: f32,
    /// Candidates each arm contributes before fusion.
    pub hybrid_pool: usize,
}

impl Default for SearchSemanticConfig {
    fn default() -> Self {
        let embeddings = EmbeddingConfig::default();
        let search = super::SearchConfig::default();

        Self {
            enabled: embeddings.enabled,
            provider: embeddings.provider,
            model: embeddings.model,
            dimensions: embeddings.dimensions,
            api_key: None,
            base_url: embeddings.base_url,
            batch_size: embeddings.batch_size,
            timeout_ms: embeddings.timeout_ms,
            retry_attempts: embeddings.retry_attempts,
            rrf_k: search.rrf_k,
            hybrid_pool: search.hybrid_pool,
        }
    }
}

impl SearchSemanticConfig {
    /// Cross-field validation for the embeddings provider, mirroring
    /// [`EmbeddingConfig::validate_for_provider`]: the configured dimensions
    /// must match [`super::SCHEMA_EMBEDDING_DIMENSIONS`], the model must be
    /// non-empty, and an `openai_compatible` provider that is enabled must
    /// carry an API key. Used by [`crate::embeddings::OpenAiCompatibleEmbeddingProvider::new`]
    /// so a hand-built config (as `tests/embeddings_semantic_search.rs`
    /// constructs) is validated the same way a loaded one is.
    pub fn validate_for_provider(&self) -> Result<(), String> {
        if self.dimensions != super::SCHEMA_EMBEDDING_DIMENSIONS {
            return Err(format!(
                "ATLAS_EMBEDDINGS_DIMENSIONS is {} but the search_embeddings.embedding column is \
                 vector({dims}); use a model with {dims} dimensions",
                self.dimensions,
                dims = super::SCHEMA_EMBEDDING_DIMENSIONS
            ));
        }
        if self.model.trim().is_empty() {
            return Err("ATLAS_EMBEDDINGS_MODEL must not be empty".to_owned());
        }
        if matches!(self.provider, EmbeddingProviderKind::OpenAiCompatible)
            && self.enabled
            && self
                .api_key
                .as_ref()
                .map(|key| key.expose().trim().is_empty())
                .unwrap_or(true)
        {
            return Err(
                "ATLAS_EMBEDDINGS_API_KEY is required for openai_compatible embeddings".to_owned(),
            );
        }
        Ok(())
    }
}

impl ComponentConfig for SearchSemanticConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let embeddings = EmbeddingConfig::from_env(source).map_err(ConfigError::composition)?;
        let search = load_search_config(source).map_err(ConfigError::composition)?;

        Ok(Self {
            enabled: embeddings.enabled,
            provider: embeddings.provider,
            model: embeddings.model,
            dimensions: embeddings.dimensions,
            api_key: embeddings.api_key.map(Secret::new),
            base_url: embeddings.base_url,
            batch_size: embeddings.batch_size,
            timeout_ms: embeddings.timeout_ms,
            retry_attempts: embeddings.retry_attempts,
            rrf_k: search.rrf_k,
            hybrid_pool: search.hybrid_pool,
        })
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl EnvSource {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn search_lexical_config_loads_with_no_fields() {
        SearchLexicalConfig::from_env(&env(&[])).expect("expected Ok");
    }

    #[test]
    fn search_semantic_config_binds_fusion_tuning_and_embeddings() {
        let cfg = SearchSemanticConfig::from_env(&env(&[
            ("ATLAS_SEARCH_RRF_K", "12.5"),
            ("ATLAS_SEARCH_HYBRID_POOL", "17"),
            ("ATLAS_EMBEDDINGS_MODEL", "custom-model"),
        ]))
        .expect("expected Ok");

        assert!((cfg.rrf_k - 12.5).abs() < f32::EPSILON);
        assert_eq!(cfg.hybrid_pool, 17);
        assert_eq!(cfg.model, "custom-model");
    }

    #[test]
    fn search_semantic_config_defaults_match_v1() {
        let cfg = SearchSemanticConfig::from_env(&env(&[])).expect("expected Ok");

        assert!((cfg.rrf_k - 60.0).abs() < f32::EPSILON);
        assert_eq!(cfg.hybrid_pool, 50);
        assert!(!cfg.enabled);
    }

    #[test]
    fn search_semantic_config_wraps_the_api_key_in_secret() {
        let cfg = SearchSemanticConfig::from_env(&env(&[("ATLAS_EMBEDDINGS_API_KEY", "key-123")]))
            .expect("expected Ok");

        assert_eq!(
            cfg.api_key.map(|s| s.expose().clone()),
            Some("key-123".to_string())
        );
    }
}
