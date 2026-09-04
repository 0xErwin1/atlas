//! Typed, per-component server configuration (SHELL-CFG-1..3).
//!
//! [`AtlasConfig`] composes one [`atlas_core::config::ComponentConfig`]
//! implementer per component/Module this release builds:
//! [`platform::PlatformConfig`], [`custos::CustosConfig`],
//! [`acta::ActaConfig`], and — under [`ModuleConfigs`] —
//! [`storage::StorageConfig`], [`search::SearchLexicalConfig`], and
//! [`search::SearchSemanticConfig`]. The single flat V1 loader this module
//! used to expose is retired by this split; no consumer reads it after this
//! slice.
//!
//! [`EmbeddingConfig`], [`SearchConfig`], and the free `read_*`/`load_*`
//! helpers below predate the split. They stay `pub` and unmodified so the
//! by-name environment-binding characterization test
//! (`tests/env_binding.rs`, frozen unchanged across this PR per design
//! D3.2/R1) and `tests/anchor_interval_divergence.rs` keep exercising the
//! exact production surface they were written against. No route,
//! `AppState`, or `main.rs` call site reads `EmbeddingConfig`/`SearchConfig`
//! directly after this slice — [`search::SearchSemanticConfig::from_env`]
//! is the only remaining caller, and it carries their validated values
//! forward into `Secret`-wrapped fields.

mod acta;
mod custos;
mod platform;
mod search;
mod storage;

pub use acta::ActaConfig;
pub use custos::CustosConfig;
pub use platform::PlatformConfig;
pub use search::{SearchLexicalConfig, SearchSemanticConfig};
pub use storage::StorageConfig;

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource};
use atlas_core::registry::ComponentEntry;
use base64::{Engine, engine::general_purpose::STANDARD};
use std::fmt;

pub use atlas_postgres::{PoolConfig, PostgresConfig};

/// Vector width the `search_embeddings.embedding` column is declared with.
///
/// The DDL (`migration::m20260708_000039_search_embeddings`) hardcodes
/// `vector(1536)`, and Postgres rejects an insert of any other width. Nothing in
/// the embedding pipeline reconciles the two, so a configured dimension that
/// disagrees with the column only surfaces on the first insert — long after
/// startup, once a document write has already been accepted.
pub const SCHEMA_EMBEDDING_DIMENSIONS: usize = 1536;

/// Today's hardcoded attachment size cap, preserved as
/// [`acta::ActaConfig::max_attachment_bytes`]'s default so an unconfigured
/// deployment keeps behaving identically after `ATLAS_ACTA_MAX_ATTACHMENT_BYTES`
/// is introduced.
pub const DEFAULT_MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024; // 20 MiB

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EmbeddingProviderKind {
    Deterministic,
    OpenAiCompatible,
}

#[derive(Clone, PartialEq, Eq)]
pub struct EmbeddingConfig {
    pub enabled: bool,
    pub provider: EmbeddingProviderKind,
    pub model: String,
    pub dimensions: usize,
    pub api_key: Option<String>,
    pub base_url: String,
    pub batch_size: usize,
    pub timeout_ms: u64,
    pub retry_attempts: u32,
}

impl EmbeddingConfig {
    /// Kept for existing test call sites that build a closure-based source;
    /// the closure already satisfies `EnvSource` via `atlas_core`'s blanket
    /// impl, so this is a thin forwarder to [`Self::from_env`].
    pub fn from_env_vars<F>(get: F) -> Result<Self, String>
    where
        F: Fn(&str) -> Option<String>,
    {
        Self::from_env(&get)
    }

    pub fn from_env(source: &dyn EnvSource) -> Result<Self, String> {
        let read = |name: &str| source.get(name);
        let enabled = read_bool(read("ATLAS_EMBEDDINGS_ENABLED"), false);
        let provider = match nonempty(read("ATLAS_EMBEDDINGS_PROVIDER")) {
            Some(raw) => match raw.as_str() {
                "deterministic" | "test" => EmbeddingProviderKind::Deterministic,
                "openai_compatible" => EmbeddingProviderKind::OpenAiCompatible,
                other => return Err(format!("unsupported ATLAS_EMBEDDINGS_PROVIDER: {other}")),
            },
            // The deterministic provider hashes text into a valid-looking vector
            // whose nearest neighbours are arbitrary, so inheriting it silently
            // would answer every semantic query with noise and no error.
            None if enabled => {
                return Err(
                    "ATLAS_EMBEDDINGS_PROVIDER is required when ATLAS_EMBEDDINGS_ENABLED=true; \
                     use 'openai_compatible' for real embeddings, or ask for 'deterministic' \
                     explicitly to get the test provider that encodes no meaning"
                        .to_owned(),
                );
            }
            None => EmbeddingProviderKind::Deterministic,
        };
        let model =
            read("ATLAS_EMBEDDINGS_MODEL").unwrap_or_else(|| "atlas-test-embedding".to_owned());
        if model.trim().is_empty() {
            return Err("ATLAS_EMBEDDINGS_MODEL must not be empty".to_owned());
        }
        let dimensions = read("ATLAS_EMBEDDINGS_DIMENSIONS")
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(SCHEMA_EMBEDDING_DIMENSIONS);
        let config = Self {
            enabled,
            provider,
            model,
            dimensions,
            api_key: read("ATLAS_EMBEDDINGS_API_KEY"),
            base_url: read("ATLAS_EMBEDDINGS_BASE_URL")
                .unwrap_or_else(|| "https://api.openai.com/v1".to_owned()),
            batch_size: read("ATLAS_EMBEDDINGS_BATCH_SIZE")
                .and_then(|value| value.parse::<usize>().ok())
                .unwrap_or(64),
            timeout_ms: read("ATLAS_EMBEDDINGS_TIMEOUT_MS")
                .and_then(|value| value.parse::<u64>().ok())
                .unwrap_or(30_000),
            retry_attempts: read("ATLAS_EMBEDDINGS_RETRY_ATTEMPTS")
                .and_then(|value| value.parse::<u32>().ok())
                .unwrap_or(2),
        };
        config.validate_for_provider()?;
        Ok(config)
    }

    pub fn validate_for_provider(&self) -> Result<(), String> {
        if self.dimensions != SCHEMA_EMBEDDING_DIMENSIONS {
            return Err(format!(
                "ATLAS_EMBEDDINGS_DIMENSIONS is {} but the search_embeddings.embedding column is \
                 vector({SCHEMA_EMBEDDING_DIMENSIONS}); use a model with \
                 {SCHEMA_EMBEDDING_DIMENSIONS} dimensions",
                self.dimensions
            ));
        }
        if self.model.trim().is_empty() {
            return Err("ATLAS_EMBEDDINGS_MODEL must not be empty".to_owned());
        }
        if matches!(self.provider, EmbeddingProviderKind::OpenAiCompatible)
            && self.enabled
            && self
                .api_key
                .as_deref()
                .unwrap_or_default()
                .trim()
                .is_empty()
        {
            return Err(
                "ATLAS_EMBEDDINGS_API_KEY is required for openai_compatible embeddings".to_owned(),
            );
        }
        Ok(())
    }
}

impl fmt::Debug for EmbeddingConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EmbeddingConfig")
            .field("enabled", &self.enabled)
            .field("provider", &self.provider)
            .field("model", &self.model)
            .field("dimensions", &self.dimensions)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("base_url", &self.base_url)
            .field("batch_size", &self.batch_size)
            .field("timeout_ms", &self.timeout_ms)
            .field("retry_attempts", &self.retry_attempts)
            .finish()
    }
}

impl Default for EmbeddingConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            provider: EmbeddingProviderKind::Deterministic,
            model: "atlas-test-embedding".to_owned(),
            dimensions: SCHEMA_EMBEDDING_DIMENSIONS,
            api_key: None,
            base_url: "https://api.openai.com/v1".to_owned(),
            batch_size: 64,
            timeout_ms: 30_000,
            retry_attempts: 2,
        }
    }
}

/// Tuning for the hybrid (lexical + vector) search mode.
///
/// Both values are exposed because the right ones depend on the corpus: the
/// published RRF default of 60 and a 50-candidate pool are starting points to
/// measure against real content, not constants worth hardcoding.
#[derive(Clone, Debug, PartialEq)]
pub struct SearchConfig {
    /// RRF damping constant. Smaller lets one arm's top hit dominate; larger
    /// weighs the two arms more evenly.
    pub rrf_k: f32,
    /// Candidates each arm contributes before fusion. Fusion happens over these
    /// two pools only, so this is also how deep hybrid results can be paged.
    pub hybrid_pool: usize,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            rrf_k: 60.0,
            hybrid_pool: 50,
        }
    }
}

/// Runtime parameters for the webhook dispatcher.
#[derive(Clone, Debug)]
pub struct DispatcherConfig {
    /// Milliseconds between successive poll cycles when there is no work.
    pub poll_interval_ms: u64,
    /// Maximum delivery attempts before an outbox row transitions to `dead`.
    pub max_attempts: i32,
    /// Per-delivery HTTP request timeout in milliseconds.
    pub delivery_timeout_ms: u64,
    /// Maximum number of concurrent deliveries per poll cycle.
    pub max_concurrent: usize,
    /// Maximum number of outbox rows to claim per poll cycle.
    pub batch_size: i64,
    /// Seconds a claimed row is locked before the recovery sweep reclaims it.
    pub lease_secs: i64,
}

impl Default for DispatcherConfig {
    fn default() -> Self {
        Self {
            poll_interval_ms: 1_000,
            max_attempts: 5,
            delivery_timeout_ms: 10_000,
            max_concurrent: 16,
            batch_size: 32,
            lease_secs: 30,
        }
    }
}

/// Per-principal rate-limit parameters for the authenticated API surface.
///
/// The limiter keys by the authenticated caller (user or API key), not by IP:
/// the abuse vector the limit guards against is programmatic clients (the MCP
/// server and CLI) driving high request volume, and those are always
/// authenticated. `per_second` is the steady-state refill rate and `burst` is
/// the maximum number of requests allowed in an instantaneous spike.
#[derive(Clone, Debug)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub per_second: u32,
    pub burst: u32,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            per_second: 20,
            burst: 40,
        }
    }
}

/// Reads `ATLAS_ANCHOR_INTERVAL` through `source`, refusing a value below 2.
///
/// Exposed (rather than inlined) so the by-name environment-binding
/// characterization test (`tests/env_binding.rs`) and the
/// `ATLAS_ANCHOR_INTERVAL` divergence characterization test
/// (`tests/anchor_interval_divergence.rs`) can exercise this exact rule.
/// [`acta::ActaConfig::from_env`] wraps this in a [`ConfigError`] that never
/// echoes the parsed value, matching every other `ComponentConfig`
/// implementer in this module.
pub fn read_anchor_interval(source: &dyn EnvSource) -> Result<u32, String> {
    let value = read_env(source, "ATLAS_ANCHOR_INTERVAL", 50);

    if value < 2 {
        return Err(format!("ATLAS_ANCHOR_INTERVAL must be >= 2, got {value}"));
    }

    Ok(value)
}

/// Reads and validates `ATLAS_WEBHOOK_ENC_KEY`.
///
/// The variable must contain a standard-base64-encoded value that decodes to
/// exactly 32 bytes. The error message never echoes the value so a misconfigured
/// key cannot leak through startup logs.
pub fn load_webhook_enc_key(source: &dyn EnvSource) -> Result<[u8; 32], String> {
    let raw = source
        .get("ATLAS_WEBHOOK_ENC_KEY")
        .ok_or_else(|| "ATLAS_WEBHOOK_ENC_KEY is required but not set".to_string())?;

    let bytes = STANDARD
        .decode(raw.trim())
        .map_err(|e| format!("ATLAS_WEBHOOK_ENC_KEY is not valid base64: {e}"))?;

    bytes.as_slice().try_into().map_err(|_| {
        format!(
            "ATLAS_WEBHOOK_ENC_KEY must decode to exactly 32 bytes, got {}",
            bytes.len()
        )
    })
}

pub fn load_dispatcher_config(source: &dyn EnvSource) -> DispatcherConfig {
    DispatcherConfig {
        poll_interval_ms: read_env(source, "ATLAS_WEBHOOK_POLL_INTERVAL_MS", 1_000),
        max_attempts: read_env(source, "ATLAS_WEBHOOK_MAX_ATTEMPTS", 5),
        delivery_timeout_ms: read_env(source, "ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS", 10_000),
        max_concurrent: read_env(source, "ATLAS_WEBHOOK_MAX_CONCURRENT", 16),
        batch_size: read_env(source, "ATLAS_WEBHOOK_BATCH_SIZE", 32),
        lease_secs: read_env(source, "ATLAS_WEBHOOK_LEASE_SECS", 30),
    }
}

pub fn load_search_config(source: &dyn EnvSource) -> Result<SearchConfig, String> {
    let defaults = SearchConfig::default();

    let rrf_k = match env_var_nonempty(source, "ATLAS_SEARCH_RRF_K") {
        Some(raw) => raw
            .parse::<f32>()
            .ok()
            .filter(|value| value.is_finite() && *value >= 1.0)
            .ok_or_else(|| format!("ATLAS_SEARCH_RRF_K must be a number >= 1, got {raw}"))?,
        None => defaults.rrf_k,
    };

    let hybrid_pool = match env_var_nonempty(source, "ATLAS_SEARCH_HYBRID_POOL") {
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|value| *value > 0)
            .ok_or_else(|| {
                format!("ATLAS_SEARCH_HYBRID_POOL must be a positive integer, got {raw}")
            })?,
        None => defaults.hybrid_pool,
    };

    Ok(SearchConfig { rrf_k, hybrid_pool })
}

pub fn load_rate_limit_config(source: &dyn EnvSource) -> RateLimitConfig {
    let defaults = RateLimitConfig::default();
    RateLimitConfig {
        enabled: read_env_bool(source, "ATLAS_RATE_LIMIT_ENABLED", defaults.enabled),
        per_second: read_env(source, "ATLAS_RATE_LIMIT_PER_SECOND", defaults.per_second),
        burst: read_env(source, "ATLAS_RATE_LIMIT_BURST", defaults.burst),
    }
}

/// Reads and parses an environment variable through `source`, falling back to
/// `default` when the variable is absent or fails to parse as `T`.
///
/// Generic over every numeric field this module and `state.rs` read (`u32`,
/// `u64`, `i64`, `i32`, `usize`), matching V1's lenient-parsing behavior:
/// an unparseable value falls back to the default rather than failing
/// startup.
pub fn read_env<T>(source: &dyn EnvSource, var: &str, default: T) -> T
where
    T: std::str::FromStr,
{
    source
        .get(var)
        .and_then(|s| s.parse::<T>().ok())
        .unwrap_or(default)
}

pub fn read_env_bool(source: &dyn EnvSource, var: &str, default: bool) -> bool {
    read_bool(source.get(var), default)
}

fn read_bool(value: Option<String>, default: bool) -> bool {
    match nonempty(value) {
        Some(s) => matches!(s.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        None => default,
    }
}

/// Collapses a present-but-empty value to `None`.
///
/// A variable that is defined but empty carries no configuration intent, so it
/// must behave exactly like an absent one; `std::env::var` returns `Ok("")` for
/// such values, which would otherwise bypass the caller's default.
fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

/// Reads an environment variable through `source`, treating a present-but-empty
/// value as absent so the caller's default applies instead of a blank string.
///
/// Public (rather than `pub(crate)`) so the by-name environment-binding
/// characterization test (`tests/env_binding.rs`) can exercise it directly,
/// alongside its other consumers, `reg5::storage_backend_from_env` and
/// [`storage::StorageConfig::from_env`].
pub fn env_var_nonempty(source: &dyn EnvSource, key: &str) -> Option<String> {
    nonempty(source.get(key))
}

/// The typed configuration surface this release composes: one struct per
/// component (`platform`, `custos`, `acta`) plus one per configurable
/// Module, grouped under [`ModuleConfigs`].
///
/// Built by [`Self::from_registry`], never by hand: composition sequences
/// each component's own `ComponentConfig::from_env` and surfaces the first
/// `ConfigError` unmodified (SHELL-CFG-1's "the Shell validates only
/// composition" — no field-level rule is re-implemented here).
#[derive(Debug)] // safe: every field's own `Debug` already redacts its secrets.
pub struct AtlasConfig {
    pub platform: PlatformConfig,
    pub custos: CustosConfig,
    pub acta: ActaConfig,
    pub modules: ModuleConfigs,
}

/// One config struct per configurable Module active in the built registry.
#[derive(Debug)]
pub struct ModuleConfigs {
    pub storage: StorageConfig,
    pub search_lexical: SearchLexicalConfig,
    pub search_semantic: SearchSemanticConfig,
}

/// The `ConfigDeclaration.struct_name` of every field [`AtlasConfig`]
/// composes. `tests/config_registry_composition.rs` checks this list
/// bidirectionally against the registry's own declarations (D2.2): a
/// declared struct with no loader here fails, and a loaded struct declared
/// by no entry fails, for both storage backends.
pub const COMPOSED_STRUCT_NAMES: &[&str] = &[
    "PlatformConfig",
    "CustosConfig",
    "ActaConfig",
    "StorageConfig",
    "SearchLexicalConfig",
    "SearchSemanticConfig",
];

/// The exact `ATLAS_EMBEDDINGS_*` variables `search.pgvector_embeddings`
/// declares (`ATLAS_EMBEDDINGS_` prefix), checked one-by-one so a build that
/// excludes the Module can still warn when one is set without needing to
/// enumerate the whole process environment — `EnvSource` is deliberately not
/// enumerable (SHELL-CFG-1).
const SEARCH_SEMANTIC_VARS: &[&str] = &[
    "ATLAS_EMBEDDINGS_ENABLED",
    "ATLAS_EMBEDDINGS_PROVIDER",
    "ATLAS_EMBEDDINGS_MODEL",
    "ATLAS_EMBEDDINGS_DIMENSIONS",
    "ATLAS_EMBEDDINGS_API_KEY",
    "ATLAS_EMBEDDINGS_BASE_URL",
    "ATLAS_EMBEDDINGS_BATCH_SIZE",
    "ATLAS_EMBEDDINGS_TIMEOUT_MS",
    "ATLAS_EMBEDDINGS_RETRY_ATTEMPTS",
];

impl AtlasConfig {
    /// Loads every component config declared by `entries`, returning the
    /// first `ConfigError` a component's own `from_env` produces.
    ///
    /// `search.pgvector_embeddings` is `mandatory: false` (SHELL-REG-5): when
    /// it is absent from `entries`, its config is not loaded — a set
    /// `ATLAS_EMBEDDINGS_*` variable only produces a `warn`-level log naming
    /// the variable, never a startup failure (SHELL-CFG-1).
    pub fn from_registry(
        entries: &[ComponentEntry],
        source: &dyn EnvSource,
    ) -> Result<Self, ConfigError> {
        let search_semantic = if component_present(entries, "search.pgvector_embeddings") {
            SearchSemanticConfig::from_env(source)?
        } else {
            warn_configured_for_absent_component(
                source,
                "search.pgvector_embeddings",
                SEARCH_SEMANTIC_VARS,
            );
            SearchSemanticConfig::default()
        };

        Ok(Self {
            platform: PlatformConfig::from_env(source)?,
            custos: CustosConfig::from_env(source)?,
            acta: ActaConfig::from_env(source)?,
            modules: ModuleConfigs {
                storage: StorageConfig::from_env(source)?,
                search_lexical: SearchLexicalConfig::from_env(source)?,
                search_semantic,
            },
        })
    }
}

fn component_present(entries: &[ComponentEntry], stable_id: &str) -> bool {
    entries
        .iter()
        .any(|entry| entry.identity.stable_id.as_str() == stable_id)
}

/// Logs one `warn`-level line per `var` in `vars` that is set despite
/// `component` being absent from the built registry (SHELL-CFG-1: "config
/// for an absent component is ignored with a warning, not an error").
fn warn_configured_for_absent_component(source: &dyn EnvSource, component: &str, vars: &[&str]) {
    for var in vars {
        if source.get(var).is_some() {
            tracing::warn!(
                variable = *var,
                component,
                "configuration variable is set for a component absent from this build; ignored"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use atlas_core::config::Secret;

    #[test]
    fn platform_config_debug_never_contains_the_database_url_password() {
        let config = PlatformConfig {
            postgres: PostgresConfig {
                database_url: Secret::new(
                    "postgres://user:supersecretpassword@localhost/db".to_string(),
                ),
                pool: PoolConfig::default(),
            },
            rate_limit: RateLimitConfig::default(),
            shutdown_timeout_secs: 20,
            port: 8080,
            build: None,
            server_url: None,
            cookie_secure: true,
            session_ttl_hours: 168,
            session_max_ttl_hours: 720,
            idempotency_retention_hours: 24,
        };

        let output = format!("{config:?}");

        assert!(!output.contains("supersecretpassword"));
        assert!(output.contains("Secret(<redacted>)"));
    }

    #[test]
    fn custos_config_debug_never_contains_the_root_password() {
        let config = CustosConfig {
            root_password: Some(Secret::new("rootsecret".to_string())),
        };

        let output = format!("{config:?}");

        assert!(!output.contains("rootsecret"));
        assert!(output.contains("Secret(<redacted>)"));
    }

    #[test]
    fn acta_config_debug_never_contains_the_webhook_key_bytes() {
        let config = ActaConfig {
            anchor_interval: 50,
            dispatcher: DispatcherConfig::default(),
            webhook_enc_key: Secret::new([0xABu8; 32]),
            allow_private_webhook_targets: false,
            max_attachment_bytes: DEFAULT_MAX_ATTACHMENT_BYTES,
            upload_allowed_extensions: None,
        };

        let output = format!("{config:?}");

        assert!(!output.contains("0xAB") && !output.contains("171"));
        assert!(output.contains("Secret(<redacted>)"));
    }

    #[test]
    fn storage_config_debug_never_contains_the_s3_secret_key() {
        let config = StorageConfig::S3 {
            bucket: "bucket".to_string(),
            endpoint: "https://example.test".to_string(),
            access_key_id: "access-key".to_string(),
            secret_access_key: Secret::new("supersecretaccesskey".to_string()),
            region: "auto".to_string(),
        };

        let output = format!("{config:?}");

        assert!(!output.contains("supersecretaccesskey"));
        assert!(output.contains("Secret(<redacted>)"));
    }

    #[test]
    fn search_semantic_config_debug_never_contains_the_api_key() {
        let config = SearchSemanticConfig {
            api_key: Some(Secret::new("sk-supersecretapikey".to_string())),
            ..SearchSemanticConfig::default()
        };

        let output = format!("{config:?}");

        assert!(!output.contains("sk-supersecretapikey"));
        assert!(output.contains("Secret(<redacted>)"));
    }

    #[test]
    fn rate_limit_config_has_sane_defaults() {
        let cfg = RateLimitConfig::default();
        assert!(cfg.enabled, "rate limiting is enabled by default");
        assert_eq!(cfg.per_second, 20);
        assert_eq!(cfg.burst, 40);
    }

    #[test]
    fn dispatcher_config_has_sane_defaults() {
        let cfg = DispatcherConfig::default();
        assert_eq!(cfg.poll_interval_ms, 1_000);
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.delivery_timeout_ms, 10_000);
        assert_eq!(cfg.max_concurrent, 16);
        assert_eq!(cfg.batch_size, 32);
        assert_eq!(cfg.lease_secs, 30);
    }

    #[test]
    fn read_bool_treats_empty_as_absent() {
        assert!(read_bool(Some(String::new()), true));
        assert!(!read_bool(Some(String::new()), false));
    }

    #[test]
    fn read_bool_honors_truthy_and_falsy_tokens() {
        assert!(read_bool(Some("true".to_string()), false));
        assert!(!read_bool(Some("false".to_string()), true));
        assert!(read_bool(None, true));
        assert!(!read_bool(None, false));
    }

    #[test]
    fn nonempty_treats_empty_as_absent() {
        assert_eq!(nonempty(Some(String::new())), None);
        assert_eq!(nonempty(Some("x".to_string())), Some("x".to_string()));
        assert_eq!(nonempty(None), None);
    }

    #[test]
    fn from_env_vars_forwards_to_from_env_via_the_env_source_blanket_impl() {
        let cfg = EmbeddingConfig::from_env_vars(|name| match name {
            "ATLAS_EMBEDDINGS_MODEL" => Some("closure-model".to_owned()),
            _ => None,
        })
        .expect("valid deterministic config");

        assert_eq!(cfg.model, "closure-model");
    }

    #[test]
    fn absent_component_config_warns_without_composition_error() {
        // `entries` deliberately omits `search.pgvector_embeddings`, unlike
        // the real `reg5_component_entries` builds today (§0.5). This proves
        // the absent-component branch by construction rather than by
        // rebuilding the registry, since no current build variant excludes
        // it (SHELL-REG-5 reserves that for a future release).
        let entries: Vec<ComponentEntry> = vec![];
        let source = |key: &str| -> Option<String> {
            match key {
                "DATABASE_URL" => Some("postgres://set-value/db".to_string()),
                "ATLAS_WEBHOOK_ENC_KEY" => Some(STANDARD.encode([0xAB_u8; 32]).to_string()),
                "ATLAS_EMBEDDINGS_API_KEY" => Some("configured-but-absent".to_string()),
                _ => None,
            }
        };

        let cfg = AtlasConfig::from_registry(&entries, &source).expect("expected Ok");

        assert!(!cfg.modules.search_semantic.enabled);
        assert!(cfg.modules.search_semantic.api_key.is_none());
    }

    #[test]
    fn from_registry_surfaces_the_owning_components_own_validation_error() {
        let entries: Vec<ComponentEntry> = vec![];
        let source = |key: &str| -> Option<String> {
            match key {
                "DATABASE_URL" => Some("postgres://set-value/db".to_string()),
                "ATLAS_WEBHOOK_ENC_KEY" => Some(STANDARD.encode([0xAB_u8; 32]).to_string()),
                "ATLAS_ANCHOR_INTERVAL" => Some("1".to_string()),
                _ => None,
            }
        };

        let error = AtlasConfig::from_registry(&entries, &source).expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid("ATLAS_ANCHOR_INTERVAL", "must be >= 2")
        );
    }
}
