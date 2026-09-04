//! By-name environment-binding characterization test (design D3.2/R1, spec
//! Requirement "Every V1 environment variable keeps its name and binds to
//! the same field").
//!
//! Enumerates every one of the 44 V1 environment variables read by the flat
//! `ServerConfig`/`AppState`/`main.rs` loaders across `config.rs`, `state.rs`,
//! `main.rs`, `routes/health.rs`, and `routes/users.rs`. For each, this test
//! sets the variable alone (plus whatever prerequisite context its own
//! validation requires — e.g. `ATLAS_ATTACHMENT_BACKEND=s3` for the four
//! `ATLAS_S3_*` variables) and asserts the resulting value at its documented
//! field equals the set value; a second load with the variable absent
//! asserts the documented default (or the required-missing error, for the
//! variables V1 already requires).
//!
//! `VARS.len() == 44` is asserted first as a non-vacuity gate (INV-BYVAR-ENUM):
//! a variable dropped from this list fails the count before any individual
//! row could fail silently. PR2 (design D3.2, U2) must keep this file's
//! variable names, set-values, and default-values byte-unchanged; only the
//! internal loader-call plumbing behind each row may change.

use atlas_core::config::{ComponentConfig, EnvSource};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::collections::HashMap;

/// A fixed-map `EnvSource`, built from an explicit variable list so each
/// test case controls exactly which variables are "set" and leaves
/// everything else absent. Values are owned so a case can build a variable
/// value at runtime (e.g. a base64-encoded fixture) without fighting
/// lifetimes.
struct MapEnv(HashMap<String, String>);

impl EnvSource for MapEnv {
    fn get(&self, key: &str) -> Option<String> {
        self.0.get(key).cloned()
    }
}

fn env(pairs: &[(&'static str, &'static str)]) -> MapEnv {
    MapEnv(
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    )
}

fn empty() -> MapEnv {
    MapEnv(HashMap::new())
}

/// One case per V1 environment variable, keyed by its exact name.
struct VarCase {
    name: &'static str,
    check: fn() -> Result<(), String>,
}

/// The complete by-name enumeration: 29 variables read in `config.rs`'s flat
/// `ServerConfig` (including the 3 read by `atlas_postgres::PostgresConfig`/
/// `PoolConfig`), plus 15 read directly by `state.rs`/`main.rs`/
/// `routes/health.rs`/`routes/users.rs`.
const VARS: &[VarCase] = &[
    VarCase {
        name: "DATABASE_URL",
        check: check_database_url,
    },
    VarCase {
        name: "ATLAS_DB_MAX_CONNECTIONS",
        check: check_db_max_connections,
    },
    VarCase {
        name: "ATLAS_DB_MIN_CONNECTIONS",
        check: check_db_min_connections,
    },
    VarCase {
        name: "ATLAS_DB_ACQUIRE_TIMEOUT_SECS",
        check: check_db_acquire_timeout_secs,
    },
    VarCase {
        name: "ATLAS_ROOT_PASSWORD",
        check: check_root_password,
    },
    VarCase {
        name: "ATLAS_ANCHOR_INTERVAL",
        check: check_anchor_interval,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_ENC_KEY",
        check: check_webhook_enc_key,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_POLL_INTERVAL_MS",
        check: check_webhook_poll_interval_ms,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_MAX_ATTEMPTS",
        check: check_webhook_max_attempts,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS",
        check: check_webhook_delivery_timeout_ms,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_MAX_CONCURRENT",
        check: check_webhook_max_concurrent,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_BATCH_SIZE",
        check: check_webhook_batch_size,
    },
    VarCase {
        name: "ATLAS_WEBHOOK_LEASE_SECS",
        check: check_webhook_lease_secs,
    },
    VarCase {
        name: "ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS",
        check: check_allow_private_webhook_targets,
    },
    VarCase {
        name: "ATLAS_RATE_LIMIT_ENABLED",
        check: check_rate_limit_enabled,
    },
    VarCase {
        name: "ATLAS_RATE_LIMIT_PER_SECOND",
        check: check_rate_limit_per_second,
    },
    VarCase {
        name: "ATLAS_RATE_LIMIT_BURST",
        check: check_rate_limit_burst,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_ENABLED",
        check: check_embeddings_enabled,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_PROVIDER",
        check: check_embeddings_provider,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_MODEL",
        check: check_embeddings_model,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_DIMENSIONS",
        check: check_embeddings_dimensions,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_API_KEY",
        check: check_embeddings_api_key,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_BASE_URL",
        check: check_embeddings_base_url,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_BATCH_SIZE",
        check: check_embeddings_batch_size,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_TIMEOUT_MS",
        check: check_embeddings_timeout_ms,
    },
    VarCase {
        name: "ATLAS_EMBEDDINGS_RETRY_ATTEMPTS",
        check: check_embeddings_retry_attempts,
    },
    VarCase {
        name: "ATLAS_SEARCH_RRF_K",
        check: check_search_rrf_k,
    },
    VarCase {
        name: "ATLAS_SEARCH_HYBRID_POOL",
        check: check_search_hybrid_pool,
    },
    VarCase {
        name: "ATLAS_SHUTDOWN_TIMEOUT_SECS",
        check: check_shutdown_timeout_secs,
    },
    VarCase {
        name: "ATLAS_PORT",
        check: check_port,
    },
    VarCase {
        name: "ATLAS_SERVER_URL",
        check: check_server_url,
    },
    VarCase {
        name: "ATLAS_BUILD",
        check: check_build,
    },
    VarCase {
        name: "ATLAS_COOKIE_SECURE",
        check: check_cookie_secure,
    },
    VarCase {
        name: "ATLAS_SESSION_TTL_HOURS",
        check: check_session_ttl_hours,
    },
    VarCase {
        name: "ATLAS_SESSION_MAX_TTL_HOURS",
        check: check_session_max_ttl_hours,
    },
    VarCase {
        name: "ATLAS_IDEMPOTENCY_RETENTION_HOURS",
        check: check_idempotency_retention_hours,
    },
    VarCase {
        name: "ATLAS_UPLOAD_ALLOWED_EXTENSIONS",
        check: check_upload_allowed_extensions,
    },
    VarCase {
        name: "ATLAS_ATTACHMENT_BACKEND",
        check: check_attachment_backend,
    },
    VarCase {
        name: "ATLAS_ATTACHMENT_ROOT",
        check: check_attachment_root,
    },
    VarCase {
        name: "ATLAS_S3_BUCKET",
        check: check_s3_bucket,
    },
    VarCase {
        name: "ATLAS_S3_ENDPOINT",
        check: check_s3_endpoint,
    },
    VarCase {
        name: "ATLAS_S3_ACCESS_KEY_ID",
        check: check_s3_access_key_id,
    },
    VarCase {
        name: "ATLAS_S3_SECRET_ACCESS_KEY",
        check: check_s3_secret_access_key,
    },
    VarCase {
        name: "ATLAS_S3_REGION",
        check: check_s3_region,
    },
];

#[test]
fn enumerates_all_44_v1_variables_by_name() {
    assert_eq!(
        VARS.len(),
        44,
        "the by-name enumeration must cover exactly 44 V1 variables; got {}",
        VARS.len()
    );
}

#[test]
fn every_enumerated_variable_binds_to_its_documented_field() {
    let mut failures = Vec::new();

    for case in VARS {
        if let Err(message) = (case.check)() {
            failures.push(format!("{}: {message}", case.name));
        }
    }

    assert!(
        failures.is_empty(),
        "variables that failed to bind to their documented field:\n{}",
        failures.join("\n")
    );
}

fn check_database_url() -> Result<(), String> {
    let set = atlas_postgres::PostgresConfig::from_env(&env(&[(
        "DATABASE_URL",
        "postgres://set-value/db",
    )]))
    .map_err(|e| format!("unexpected error with the variable set: {e}"))?;

    if set.database_url.expose() != "postgres://set-value/db" {
        return Err("database_url did not bind to the set value".to_string());
    }

    if atlas_postgres::PostgresConfig::from_env(&empty()).is_ok() {
        return Err("DATABASE_URL is required; a missing value must error".to_string());
    }

    Ok(())
}

fn check_db_max_connections() -> Result<(), String> {
    let set = atlas_postgres::PoolConfig::from_env(&env(&[("ATLAS_DB_MAX_CONNECTIONS", "7")]))
        .map_err(|e| e.to_string())?;
    if set.max_connections != 7 {
        return Err(format!("expected 7, got {}", set.max_connections));
    }

    let unset = atlas_postgres::PoolConfig::from_env(&empty()).map_err(|e| e.to_string())?;
    if unset.max_connections != 20 {
        return Err(format!(
            "expected default 20, got {}",
            unset.max_connections
        ));
    }

    Ok(())
}

fn check_db_min_connections() -> Result<(), String> {
    let set = atlas_postgres::PoolConfig::from_env(&env(&[("ATLAS_DB_MIN_CONNECTIONS", "5")]))
        .map_err(|e| e.to_string())?;
    if set.min_connections != 5 {
        return Err(format!("expected 5, got {}", set.min_connections));
    }

    let unset = atlas_postgres::PoolConfig::from_env(&empty()).map_err(|e| e.to_string())?;
    if unset.min_connections != 1 {
        return Err(format!("expected default 1, got {}", unset.min_connections));
    }

    Ok(())
}

fn check_db_acquire_timeout_secs() -> Result<(), String> {
    let set =
        atlas_postgres::PoolConfig::from_env(&env(&[("ATLAS_DB_ACQUIRE_TIMEOUT_SECS", "99")]))
            .map_err(|e| e.to_string())?;
    if set.acquire_timeout_secs != 99 {
        return Err(format!("expected 99, got {}", set.acquire_timeout_secs));
    }

    let unset = atlas_postgres::PoolConfig::from_env(&empty()).map_err(|e| e.to_string())?;
    if unset.acquire_timeout_secs != 10 {
        return Err(format!(
            "expected default 10, got {}",
            unset.acquire_timeout_secs
        ));
    }

    Ok(())
}

fn check_root_password() -> Result<(), String> {
    let set = atlas_server::config::env_var_nonempty(
        &env(&[("ATLAS_ROOT_PASSWORD", "s3cr3t")]),
        "ATLAS_ROOT_PASSWORD",
    );
    if set.as_deref() != Some("s3cr3t") {
        return Err("root_password did not bind to the set value".to_string());
    }

    let unset = atlas_server::config::env_var_nonempty(&empty(), "ATLAS_ROOT_PASSWORD");
    if unset.is_some() {
        return Err("expected None when unset".to_string());
    }

    Ok(())
}

fn check_anchor_interval() -> Result<(), String> {
    let set = atlas_server::config::read_anchor_interval(&env(&[("ATLAS_ANCHOR_INTERVAL", "10")]))?;
    if set != 10 {
        return Err(format!("expected 10, got {set}"));
    }

    let unset = atlas_server::config::read_anchor_interval(&empty())?;
    if unset != 50 {
        return Err(format!("expected default 50, got {unset}"));
    }

    Ok(())
}

fn webhook_key_b64(byte: u8) -> String {
    STANDARD.encode([byte; 32])
}

fn check_webhook_enc_key() -> Result<(), String> {
    let key_b64 = webhook_key_b64(0xAB);
    let mut vars = HashMap::new();
    vars.insert("ATLAS_WEBHOOK_ENC_KEY".to_string(), key_b64);
    let set = atlas_server::config::load_webhook_enc_key(&MapEnv(vars))?;
    if set != [0xABu8; 32] {
        return Err("webhook_enc_key did not bind to the set bytes".to_string());
    }

    if atlas_server::config::load_webhook_enc_key(&empty()).is_ok() {
        return Err("ATLAS_WEBHOOK_ENC_KEY is required; a missing value must error".to_string());
    }

    Ok(())
}

fn check_webhook_poll_interval_ms() -> Result<(), String> {
    let set = atlas_server::config::load_dispatcher_config(&env(&[(
        "ATLAS_WEBHOOK_POLL_INTERVAL_MS",
        "42",
    )]));
    if set.poll_interval_ms != 42 {
        return Err(format!("expected 42, got {}", set.poll_interval_ms));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.poll_interval_ms != 1_000 {
        return Err(format!(
            "expected default 1000, got {}",
            unset.poll_interval_ms
        ));
    }

    Ok(())
}

fn check_webhook_max_attempts() -> Result<(), String> {
    let set =
        atlas_server::config::load_dispatcher_config(&env(&[("ATLAS_WEBHOOK_MAX_ATTEMPTS", "9")]));
    if set.max_attempts != 9 {
        return Err(format!("expected 9, got {}", set.max_attempts));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.max_attempts != 5 {
        return Err(format!("expected default 5, got {}", unset.max_attempts));
    }

    Ok(())
}

fn check_webhook_delivery_timeout_ms() -> Result<(), String> {
    let set = atlas_server::config::load_dispatcher_config(&env(&[(
        "ATLAS_WEBHOOK_DELIVERY_TIMEOUT_MS",
        "12345",
    )]));
    if set.delivery_timeout_ms != 12_345 {
        return Err(format!("expected 12345, got {}", set.delivery_timeout_ms));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.delivery_timeout_ms != 10_000 {
        return Err(format!(
            "expected default 10000, got {}",
            unset.delivery_timeout_ms
        ));
    }

    Ok(())
}

fn check_webhook_max_concurrent() -> Result<(), String> {
    let set = atlas_server::config::load_dispatcher_config(&env(&[(
        "ATLAS_WEBHOOK_MAX_CONCURRENT",
        "3",
    )]));
    if set.max_concurrent != 3 {
        return Err(format!("expected 3, got {}", set.max_concurrent));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.max_concurrent != 16 {
        return Err(format!("expected default 16, got {}", unset.max_concurrent));
    }

    Ok(())
}

fn check_webhook_batch_size() -> Result<(), String> {
    let set =
        atlas_server::config::load_dispatcher_config(&env(&[("ATLAS_WEBHOOK_BATCH_SIZE", "11")]));
    if set.batch_size != 11 {
        return Err(format!("expected 11, got {}", set.batch_size));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.batch_size != 32 {
        return Err(format!("expected default 32, got {}", unset.batch_size));
    }

    Ok(())
}

fn check_webhook_lease_secs() -> Result<(), String> {
    let set =
        atlas_server::config::load_dispatcher_config(&env(&[("ATLAS_WEBHOOK_LEASE_SECS", "77")]));
    if set.lease_secs != 77 {
        return Err(format!("expected 77, got {}", set.lease_secs));
    }

    let unset = atlas_server::config::load_dispatcher_config(&empty());
    if unset.lease_secs != 30 {
        return Err(format!("expected default 30, got {}", unset.lease_secs));
    }

    Ok(())
}

fn check_allow_private_webhook_targets() -> Result<(), String> {
    let set = atlas_server::config::read_env_bool(
        &env(&[("ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS", "true")]),
        "ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS",
        false,
    );
    if !set {
        return Err("expected true, got false".to_string());
    }

    let unset =
        atlas_server::config::read_env_bool(&empty(), "ATLAS_ALLOW_PRIVATE_WEBHOOK_TARGETS", false);
    if unset {
        return Err("expected default false, got true".to_string());
    }

    Ok(())
}

fn check_rate_limit_enabled() -> Result<(), String> {
    let set = atlas_server::config::load_rate_limit_config(&env(&[(
        "ATLAS_RATE_LIMIT_ENABLED",
        "false",
    )]));
    if set.enabled {
        return Err("expected false, got true".to_string());
    }

    let unset = atlas_server::config::load_rate_limit_config(&empty());
    if !unset.enabled {
        return Err("expected default true, got false".to_string());
    }

    Ok(())
}

fn check_rate_limit_per_second() -> Result<(), String> {
    let set =
        atlas_server::config::load_rate_limit_config(&env(&[("ATLAS_RATE_LIMIT_PER_SECOND", "5")]));
    if set.per_second != 5 {
        return Err(format!("expected 5, got {}", set.per_second));
    }

    let unset = atlas_server::config::load_rate_limit_config(&empty());
    if unset.per_second != 20 {
        return Err(format!("expected default 20, got {}", unset.per_second));
    }

    Ok(())
}

fn check_rate_limit_burst() -> Result<(), String> {
    let set =
        atlas_server::config::load_rate_limit_config(&env(&[("ATLAS_RATE_LIMIT_BURST", "3")]));
    if set.burst != 3 {
        return Err(format!("expected 3, got {}", set.burst));
    }

    let unset = atlas_server::config::load_rate_limit_config(&empty());
    if unset.burst != 40 {
        return Err(format!("expected default 40, got {}", unset.burst));
    }

    Ok(())
}

fn check_embeddings_enabled() -> Result<(), String> {
    // `ATLAS_EMBEDDINGS_PROVIDER` is required once enabled=true, so it rides
    // along as prerequisite context; only `enabled` is under test here.
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[
        ("ATLAS_EMBEDDINGS_ENABLED", "true"),
        ("ATLAS_EMBEDDINGS_PROVIDER", "deterministic"),
    ]))?;
    if !set.enabled {
        return Err("expected enabled=true".to_string());
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.enabled {
        return Err("expected default enabled=false".to_string());
    }

    Ok(())
}

fn check_embeddings_provider() -> Result<(), String> {
    use atlas_server::config::EmbeddingProviderKind;

    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_PROVIDER",
        "openai_compatible",
    )]))?;
    if set.provider != EmbeddingProviderKind::OpenAiCompatible {
        return Err(format!("expected OpenAiCompatible, got {:?}", set.provider));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.provider != EmbeddingProviderKind::Deterministic {
        return Err(format!(
            "expected default Deterministic, got {:?}",
            unset.provider
        ));
    }

    Ok(())
}

fn check_embeddings_model() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_MODEL",
        "custom-model",
    )]))?;
    if set.model != "custom-model" {
        return Err(format!("expected 'custom-model', got {}", set.model));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.model != "atlas-test-embedding" {
        return Err(format!(
            "expected default 'atlas-test-embedding', got {}",
            unset.model
        ));
    }

    Ok(())
}

fn check_embeddings_dimensions() -> Result<(), String> {
    // `validate_for_provider` rejects any dimension count other than
    // `SCHEMA_EMBEDDING_DIMENSIONS` (1536) unconditionally, so a set value
    // that differs from the default cannot succeed. Binding is instead
    // proven through the error path: the rejection message echoes the parsed
    // value (a config parameter, not a secret), showing the variable's value
    // reached the field before validation rejected it.
    let result = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_DIMENSIONS",
        "4096",
    )]));
    match result {
        Ok(_) => return Err("expected a validation error for a non-schema dimension".to_string()),
        Err(message) if !message.contains("4096") => {
            return Err(format!(
                "expected the rejection to name the parsed value 4096: {message}"
            ));
        }
        Err(_) => {}
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.dimensions != atlas_server::config::SCHEMA_EMBEDDING_DIMENSIONS {
        return Err(format!(
            "expected default {}, got {}",
            atlas_server::config::SCHEMA_EMBEDDING_DIMENSIONS,
            unset.dimensions
        ));
    }

    Ok(())
}

fn check_embeddings_api_key() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_API_KEY",
        "key-123",
    )]))?;
    if set.api_key.as_deref() != Some("key-123") {
        return Err(format!("expected Some('key-123'), got {:?}", set.api_key));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.api_key.is_some() {
        return Err(format!("expected default None, got {:?}", unset.api_key));
    }

    Ok(())
}

fn check_embeddings_base_url() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_BASE_URL",
        "https://example.test/v1",
    )]))?;
    if set.base_url != "https://example.test/v1" {
        return Err(format!("expected the set URL, got {}", set.base_url));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.base_url != "https://api.openai.com/v1" {
        return Err(format!(
            "expected default 'https://api.openai.com/v1', got {}",
            unset.base_url
        ));
    }

    Ok(())
}

fn check_embeddings_batch_size() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_BATCH_SIZE",
        "7",
    )]))?;
    if set.batch_size != 7 {
        return Err(format!("expected 7, got {}", set.batch_size));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.batch_size != 64 {
        return Err(format!("expected default 64, got {}", unset.batch_size));
    }

    Ok(())
}

fn check_embeddings_timeout_ms() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_TIMEOUT_MS",
        "5000",
    )]))?;
    if set.timeout_ms != 5_000 {
        return Err(format!("expected 5000, got {}", set.timeout_ms));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.timeout_ms != 30_000 {
        return Err(format!("expected default 30000, got {}", unset.timeout_ms));
    }

    Ok(())
}

fn check_embeddings_retry_attempts() -> Result<(), String> {
    let set = atlas_server::config::EmbeddingConfig::from_env(&env(&[(
        "ATLAS_EMBEDDINGS_RETRY_ATTEMPTS",
        "6",
    )]))?;
    if set.retry_attempts != 6 {
        return Err(format!("expected 6, got {}", set.retry_attempts));
    }

    let unset = atlas_server::config::EmbeddingConfig::from_env(&empty())?;
    if unset.retry_attempts != 2 {
        return Err(format!("expected default 2, got {}", unset.retry_attempts));
    }

    Ok(())
}

fn check_search_rrf_k() -> Result<(), String> {
    let set = atlas_server::config::load_search_config(&env(&[("ATLAS_SEARCH_RRF_K", "12.5")]))?;
    if (set.rrf_k - 12.5).abs() > f32::EPSILON {
        return Err(format!("expected 12.5, got {}", set.rrf_k));
    }

    let unset = atlas_server::config::load_search_config(&empty())?;
    if (unset.rrf_k - 60.0).abs() > f32::EPSILON {
        return Err(format!("expected default 60.0, got {}", unset.rrf_k));
    }

    Ok(())
}

fn check_search_hybrid_pool() -> Result<(), String> {
    let set =
        atlas_server::config::load_search_config(&env(&[("ATLAS_SEARCH_HYBRID_POOL", "17")]))?;
    if set.hybrid_pool != 17 {
        return Err(format!("expected 17, got {}", set.hybrid_pool));
    }

    let unset = atlas_server::config::load_search_config(&empty())?;
    if unset.hybrid_pool != 50 {
        return Err(format!("expected default 50, got {}", unset.hybrid_pool));
    }

    Ok(())
}

fn check_shutdown_timeout_secs() -> Result<(), String> {
    let set: u64 = atlas_server::config::read_env(
        &env(&[("ATLAS_SHUTDOWN_TIMEOUT_SECS", "45")]),
        "ATLAS_SHUTDOWN_TIMEOUT_SECS",
        20,
    );
    if set != 45 {
        return Err(format!("expected 45, got {set}"));
    }

    let unset: u64 = atlas_server::config::read_env(&empty(), "ATLAS_SHUTDOWN_TIMEOUT_SECS", 20);
    if unset != 20 {
        return Err(format!("expected default 20, got {unset}"));
    }

    Ok(())
}

fn check_port() -> Result<(), String> {
    let set = atlas_server::startup::read_port(&env(&[("ATLAS_PORT", "9090")]), 8080);
    if set != 9090 {
        return Err(format!("expected 9090, got {set}"));
    }

    let unset = atlas_server::startup::read_port(&empty(), 8080);
    if unset != 8080 {
        return Err(format!("expected default 8080, got {unset}"));
    }

    Ok(())
}

// `ATLAS_SERVER_URL`/`ATLAS_BUILD` move into `PlatformConfig` in PR2 (design
// D3.4, orchestrator override): they are read at startup now, so these two
// rows assert through `PlatformConfig::from_env` rather than
// `EnvSource::get` directly (the pre-PR2 shape this file previously pinned,
// when the two variables were still read per-request in `routes/health.rs`/
// `routes/users.rs`). Every other row in this file is unchanged.
fn check_server_url() -> Result<(), String> {
    let set = atlas_server::config::PlatformConfig::from_env(&env(&[
        ("DATABASE_URL", "postgres://set-value/db"),
        ("ATLAS_SERVER_URL", "https://atlas.example.test"),
    ]))
    .map_err(|e| e.to_string())?;
    if set.server_url.as_deref() != Some("https://atlas.example.test") {
        return Err("server_url did not bind to the set value".to_string());
    }

    let unset = atlas_server::config::PlatformConfig::from_env(&env(&[(
        "DATABASE_URL",
        "postgres://set-value/db",
    )]))
    .map_err(|e| e.to_string())?;
    if unset.server_url.is_some() {
        return Err("expected None when unset".to_string());
    }

    Ok(())
}

fn check_build() -> Result<(), String> {
    let set = atlas_server::config::PlatformConfig::from_env(&env(&[
        ("DATABASE_URL", "postgres://set-value/db"),
        ("ATLAS_BUILD", "2026.09.01+abc123"),
    ]))
    .map_err(|e| e.to_string())?;
    if set.build.as_deref() != Some("2026.09.01+abc123") {
        return Err("build did not bind to the set value".to_string());
    }

    let unset = atlas_server::config::PlatformConfig::from_env(&env(&[(
        "DATABASE_URL",
        "postgres://set-value/db",
    )]))
    .map_err(|e| e.to_string())?;
    if unset.build.is_some() {
        return Err("expected None when unset".to_string());
    }

    Ok(())
}

fn check_cookie_secure() -> Result<(), String> {
    let set = atlas_server::state::resolve_cookie_secure(&env(&[("ATLAS_COOKIE_SECURE", "false")]));
    if set {
        return Err("expected false, got true".to_string());
    }

    let unset = atlas_server::state::resolve_cookie_secure(&empty());
    if !unset {
        return Err("expected default true, got false".to_string());
    }

    Ok(())
}

fn check_session_ttl_hours() -> Result<(), String> {
    let set: i64 = atlas_server::config::read_env(
        &env(&[("ATLAS_SESSION_TTL_HOURS", "10")]),
        "ATLAS_SESSION_TTL_HOURS",
        168,
    );
    if set != 10 {
        return Err(format!("expected 10, got {set}"));
    }

    let unset: i64 = atlas_server::config::read_env(&empty(), "ATLAS_SESSION_TTL_HOURS", 168);
    if unset != 168 {
        return Err(format!("expected default 168, got {unset}"));
    }

    Ok(())
}

fn check_session_max_ttl_hours() -> Result<(), String> {
    let set: i64 = atlas_server::config::read_env(
        &env(&[("ATLAS_SESSION_MAX_TTL_HOURS", "100")]),
        "ATLAS_SESSION_MAX_TTL_HOURS",
        720,
    );
    if set != 100 {
        return Err(format!("expected 100, got {set}"));
    }

    let unset: i64 = atlas_server::config::read_env(&empty(), "ATLAS_SESSION_MAX_TTL_HOURS", 720);
    if unset != 720 {
        return Err(format!("expected default 720, got {unset}"));
    }

    Ok(())
}

fn check_idempotency_retention_hours() -> Result<(), String> {
    let set: i64 = atlas_server::config::read_env(
        &env(&[("ATLAS_IDEMPOTENCY_RETENTION_HOURS", "5")]),
        "ATLAS_IDEMPOTENCY_RETENTION_HOURS",
        24,
    );
    if set != 5 {
        return Err(format!("expected 5, got {set}"));
    }

    let unset: i64 =
        atlas_server::config::read_env(&empty(), "ATLAS_IDEMPOTENCY_RETENTION_HOURS", 24);
    if unset != 24 {
        return Err(format!("expected default 24, got {unset}"));
    }

    Ok(())
}

fn check_upload_allowed_extensions() -> Result<(), String> {
    let source = env(&[("ATLAS_UPLOAD_ALLOWED_EXTENSIONS", "png,jpg")]);
    let set = atlas_server::state::parse_upload_allowed_extensions(
        source.get("ATLAS_UPLOAD_ALLOWED_EXTENSIONS"),
    );
    match set {
        Some(exts) if exts.contains("png") && exts.contains("jpg") => {}
        other => return Err(format!("expected {{png, jpg}}, got {other:?}")),
    }

    let unset = atlas_server::state::parse_upload_allowed_extensions(
        empty().get("ATLAS_UPLOAD_ALLOWED_EXTENSIONS"),
    );
    if unset.is_some() {
        return Err("expected default None when unset".to_string());
    }

    Ok(())
}

fn check_attachment_backend() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    // Prerequisite S3 context is required to reach the S3 arm; only the
    // discriminator itself is under test here.
    let set = atlas_server::state::resolve_attachment_backend(&env(&[
        ("ATLAS_ATTACHMENT_BACKEND", "s3"),
        ("ATLAS_S3_BUCKET", "b"),
        ("ATLAS_S3_ENDPOINT", "e"),
        ("ATLAS_S3_ACCESS_KEY_ID", "a"),
        ("ATLAS_S3_SECRET_ACCESS_KEY", "s"),
    ]))
    .map_err(|e| e.to_string())?;
    if !matches!(set, AttachmentBackendChoice::S3 { .. }) {
        return Err(format!("expected the S3 variant, got {set:?}"));
    }

    let unset =
        atlas_server::state::resolve_attachment_backend(&empty()).map_err(|e| e.to_string())?;
    if !matches!(unset, AttachmentBackendChoice::Disk { .. }) {
        return Err(format!("expected default Disk variant, got {unset:?}"));
    }

    Ok(())
}

fn check_attachment_root() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let set = atlas_server::state::resolve_attachment_backend(&env(&[(
        "ATLAS_ATTACHMENT_ROOT",
        "/custom/root",
    )]))
    .map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::Disk { root } if root == "/custom/root" => {}
        other => {
            return Err(format!(
                "expected Disk{{root: /custom/root}}, got {other:?}"
            ));
        }
    }

    let unset =
        atlas_server::state::resolve_attachment_backend(&empty()).map_err(|e| e.to_string())?;
    match unset {
        AttachmentBackendChoice::Disk { root } if root == "./data/attachments" => {}
        other => {
            return Err(format!(
                "expected default Disk{{root: ./data/attachments}}, got {other:?}"
            ));
        }
    }

    Ok(())
}

fn s3_prereqs_except(missing: &'static str) -> Vec<(&'static str, &'static str)> {
    let all = [
        ("ATLAS_S3_BUCKET", "bucket-value"),
        ("ATLAS_S3_ENDPOINT", "endpoint-value"),
        ("ATLAS_S3_ACCESS_KEY_ID", "access-key-value"),
        ("ATLAS_S3_SECRET_ACCESS_KEY", "secret-key-value"),
    ];

    let mut pairs: Vec<(&'static str, &'static str)> = vec![("ATLAS_ATTACHMENT_BACKEND", "s3")];
    pairs.extend(all.into_iter().filter(|(name, _)| *name != missing));
    pairs
}

fn check_s3_bucket() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let mut pairs = s3_prereqs_except("__none__");
    pairs.retain(|(name, _)| *name != "ATLAS_S3_BUCKET");
    pairs.push(("ATLAS_S3_BUCKET", "distinct-bucket"));

    let set =
        atlas_server::state::resolve_attachment_backend(&env(&pairs)).map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::S3 { bucket, .. } if bucket == "distinct-bucket" => {}
        other => return Err(format!("expected bucket 'distinct-bucket', got {other:?}")),
    }

    let missing = atlas_server::state::resolve_attachment_backend(&env(&s3_prereqs_except(
        "ATLAS_S3_BUCKET",
    )));
    if missing.is_ok() {
        return Err("ATLAS_S3_BUCKET is required under backend=s3; missing must error".to_string());
    }

    Ok(())
}

fn check_s3_endpoint() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let mut pairs = s3_prereqs_except("__none__");
    pairs.retain(|(name, _)| *name != "ATLAS_S3_ENDPOINT");
    pairs.push(("ATLAS_S3_ENDPOINT", "distinct-endpoint"));

    let set =
        atlas_server::state::resolve_attachment_backend(&env(&pairs)).map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::S3 { endpoint, .. } if endpoint == "distinct-endpoint" => {}
        other => {
            return Err(format!(
                "expected endpoint 'distinct-endpoint', got {other:?}"
            ));
        }
    }

    let missing = atlas_server::state::resolve_attachment_backend(&env(&s3_prereqs_except(
        "ATLAS_S3_ENDPOINT",
    )));
    if missing.is_ok() {
        return Err(
            "ATLAS_S3_ENDPOINT is required under backend=s3; missing must error".to_string(),
        );
    }

    Ok(())
}

fn check_s3_access_key_id() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let mut pairs = s3_prereqs_except("__none__");
    pairs.retain(|(name, _)| *name != "ATLAS_S3_ACCESS_KEY_ID");
    pairs.push(("ATLAS_S3_ACCESS_KEY_ID", "distinct-access-key"));

    let set =
        atlas_server::state::resolve_attachment_backend(&env(&pairs)).map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::S3 { access_key_id, .. }
            if access_key_id == "distinct-access-key" => {}
        other => {
            return Err(format!(
                "expected access_key_id 'distinct-access-key', got {other:?}"
            ));
        }
    }

    let missing = atlas_server::state::resolve_attachment_backend(&env(&s3_prereqs_except(
        "ATLAS_S3_ACCESS_KEY_ID",
    )));
    if missing.is_ok() {
        return Err(
            "ATLAS_S3_ACCESS_KEY_ID is required under backend=s3; missing must error".to_string(),
        );
    }

    Ok(())
}

fn check_s3_secret_access_key() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let mut pairs = s3_prereqs_except("__none__");
    pairs.retain(|(name, _)| *name != "ATLAS_S3_SECRET_ACCESS_KEY");
    pairs.push(("ATLAS_S3_SECRET_ACCESS_KEY", "distinct-secret-key"));

    let set =
        atlas_server::state::resolve_attachment_backend(&env(&pairs)).map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::S3 {
            secret_access_key, ..
        } if secret_access_key == "distinct-secret-key" => {}
        other => {
            return Err(format!(
                "expected secret_access_key 'distinct-secret-key', got {other:?}"
            ));
        }
    }

    let missing = atlas_server::state::resolve_attachment_backend(&env(&s3_prereqs_except(
        "ATLAS_S3_SECRET_ACCESS_KEY",
    )));
    if missing.is_ok() {
        return Err(
            "ATLAS_S3_SECRET_ACCESS_KEY is required under backend=s3; missing must error"
                .to_string(),
        );
    }

    Ok(())
}

fn check_s3_region() -> Result<(), String> {
    use atlas_server::state::AttachmentBackendChoice;

    let mut pairs = s3_prereqs_except("__none__");
    pairs.push(("ATLAS_S3_REGION", "eu-west-1"));

    let set =
        atlas_server::state::resolve_attachment_backend(&env(&pairs)).map_err(|e| e.to_string())?;
    match set {
        AttachmentBackendChoice::S3 { region, .. } if region == "eu-west-1" => {}
        other => return Err(format!("expected region 'eu-west-1', got {other:?}")),
    }

    let unset =
        atlas_server::state::resolve_attachment_backend(&env(&s3_prereqs_except("__none__")))
            .map_err(|e| e.to_string())?;
    match unset {
        AttachmentBackendChoice::S3 { region, .. } if region == "auto" => {}
        other => return Err(format!("expected default region 'auto', got {other:?}")),
    }

    Ok(())
}
