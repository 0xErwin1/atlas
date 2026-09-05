//! Container-backed drain test for E11-S3b (spec "Shutdown drains workers
//! in reverse startup order under the global `shutdown_timeout`").
//!
//! Boots the real six-worker supervisor chain (`build_workers` →
//! `BoundWorkers::bind` → `start_workers`) against a real `AppState`, then
//! drains it: confirms all six real workers drain within a short deadline
//! under normal conditions (spec's "Drain proceeds through all workers
//! within the deadline" scenario), and that the six real loops observe
//! cancellation promptly.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use std::time::Duration;

use atlas_core::config::EnvSource;
use atlas_core::registry::BoundWorkers;
use atlas_server::config::AtlasConfig;
use atlas_server::ops::supervisor::start_workers;
use atlas_server::ops::workers::build_workers;
use atlas_server::state::AppState;

/// A fixed, in-test `EnvSource`: only the two variables `AtlasConfig::from_registry`
/// cannot default (`DATABASE_URL`, `ATLAS_WEBHOOK_ENC_KEY`) are supplied.
/// `build_workers` never reads its `AtlasConfig` argument today (T1.12's
/// signature is accepted for boot-sequence symmetry), so these values are
/// never used to connect anywhere; the real database connection this test
/// drives its workers against comes from `TestDb`, independently of this
/// config object.
struct FixedEnv;

impl EnvSource for FixedEnv {
    fn get(&self, key: &str) -> Option<String> {
        match key {
            "DATABASE_URL" => Some("postgres://user:pass@localhost/atlas_test".to_string()),
            "ATLAS_WEBHOOK_ENC_KEY" => {
                use base64::Engine;
                Some(base64::engine::general_purpose::STANDARD.encode([0xABu8; 32]))
            }
            _ => None,
        }
    }
}

#[tokio::test]
async fn all_six_real_workers_drain_within_the_deadline_under_normal_conditions() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");

    let entries =
        atlas_server::reg5::reg5_component_entries(atlas_server::reg5::StorageBackend::Filesystem);
    let cfg = AtlasConfig::from_registry(&entries, &FixedEnv).expect("fixed env must compose");

    let workers = build_workers(&state, &cfg, state.workers.clone());
    assert_eq!(workers.len(), 6, "must build exactly the six REG-5 workers");

    let bound = BoundWorkers::bind(&state.registry, workers).expect("all six workers must bind");
    let running = start_workers(&state.registry, bound, state.workers.clone());

    // Let every spawned task reach its first await point before draining.
    tokio::task::yield_now().await;

    let outcome = running.drain(Duration::from_secs(10)).await;

    assert!(
        outcome.timed_out.is_empty(),
        "no worker should be cut off under normal conditions: {:?}",
        outcome.timed_out
    );
    assert_eq!(
        outcome.drained.len(),
        6,
        "all six real workers must drain within the deadline"
    );

    db.teardown().await;
}
