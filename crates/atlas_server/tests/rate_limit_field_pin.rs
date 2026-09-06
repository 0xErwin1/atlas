//! Assertion-only pin for E11-S7 U1.1 (spec "Rate-limit configuration
//! location is pinned by assertion"; G5). Confirms that
//! `AtlasConfig.platform.rate_limit.{enabled, per_second, burst}` is the
//! exact field path the three V1 environment variable names resolve to.
//! Both the path and the three names (`config/mod.rs:359-361`) already
//! exist since E11-S1 — this test pins them by name, not by sampled
//! default value, so a future rename fails loudly instead of silently.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_core::config::EnvSource;
use atlas_server::config::AtlasConfig;
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

/// A fixed, in-test `EnvSource` (`shutdown_drain.rs:31-44`'s pattern): the
/// two variables `AtlasConfig::from_registry` cannot default
/// (`DATABASE_URL`, `ATLAS_WEBHOOK_ENC_KEY`), plus the three rate-limit
/// variables this test pins.
struct FixedEnv;

impl EnvSource for FixedEnv {
    fn get(&self, key: &str) -> Option<String> {
        match key {
            "DATABASE_URL" => Some("postgres://user:pass@localhost/atlas_test".to_string()),
            "ATLAS_WEBHOOK_ENC_KEY" => {
                use base64::Engine;
                Some(base64::engine::general_purpose::STANDARD.encode([0xABu8; 32]))
            }
            "ATLAS_RATE_LIMIT_ENABLED" => Some("false".to_string()),
            "ATLAS_RATE_LIMIT_PER_SECOND" => Some("7".to_string()),
            "ATLAS_RATE_LIMIT_BURST" => Some("42".to_string()),
            _ => None,
        }
    }
}

#[test]
fn rate_limit_env_names_resolve_at_platform_rate_limit_field_path() {
    let entries = reg5_component_entries(StorageBackend::Filesystem);

    let config = AtlasConfig::from_registry(&entries, &FixedEnv).expect("fixed env must compose");

    assert!(
        !config.platform.rate_limit.enabled,
        "ATLAS_RATE_LIMIT_ENABLED must resolve at config.platform.rate_limit.enabled"
    );
    assert_eq!(
        config.platform.rate_limit.per_second, 7,
        "ATLAS_RATE_LIMIT_PER_SECOND must resolve at config.platform.rate_limit.per_second"
    );
    assert_eq!(
        config.platform.rate_limit.burst, 42,
        "ATLAS_RATE_LIMIT_BURST must resolve at config.platform.rate_limit.burst"
    );
}
