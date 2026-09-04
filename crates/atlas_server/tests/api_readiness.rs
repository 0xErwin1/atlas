#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use atlas_core::capabilities::{DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus};
use atlas_core::ops::test_support::FakeDiagnostics;
use atlas_core::registry::{ComponentId, build};
use serde_json::Value;

use atlas_server::ops::{ComponentDiagnostics, DiagnosticsRegistry};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};

fn component(value: &str) -> ComponentId {
    ComponentId::new(value).expect("valid component id")
}

fn fake(health: HealthStatus, readiness: ReadinessStatus) -> ComponentDiagnostics {
    let fake: Arc<FakeDiagnostics> = Arc::new(FakeDiagnostics::new(
        health,
        readiness,
        Vec::<DoctorFinding>::new(),
    ));
    ComponentDiagnostics {
        health: fake.clone(),
        readiness: fake,
    }
}

/// A `Readiness` implementer that never resolves within any reasonable
/// bound, used to exercise `TokioDeadline`'s elapsed path (design T1.28)
/// without relying on `FakeDiagnostics`'s always-immediate outcome.
struct NeverResolves;

impl Health for NeverResolves {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for NeverResolves {
    async fn readiness(&self) -> ReadinessStatus {
        std::future::pending::<()>().await;
        unreachable!("never resolves")
    }
}

fn never_resolves() -> ComponentDiagnostics {
    let stalling = Arc::new(NeverResolves);
    ComponentDiagnostics {
        health: stalling.clone(),
        readiness: stalling,
    }
}

/// SH2 (spec Scenario, permanent regression test): Custos not ready yields
/// a 503 naming exactly `custos`, with Platform and Acta both Ready —
/// forced through `AppState::with_diagnostics` + `FakeDiagnostics`, never a
/// poisoned pool (design R8): a poisoned pool would fail every component at
/// once and prove nothing about aggregation.
#[tokio::test]
async fn sh2_custos_not_ready_yields_503_naming_only_custos() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(
                HealthStatus::Ok,
                ReadinessStatus::NotReady {
                    reason: "warming up".to_string(),
                },
            ),
        ),
        (
            component("acta"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let response = reqwest::get(format!("{}/ready", server.base_url()))
        .await
        .expect("GET /ready");

    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("json body");

    assert_eq!(body["ready"], Value::Bool(false));
    let not_ready = body["not_ready"].as_array().expect("not_ready is an array");
    assert_eq!(
        not_ready.len(),
        1,
        "exactly one component must be named, not platform/acta too: {body}"
    );
    assert_eq!(
        not_ready[0]["component"],
        Value::String("custos".to_string())
    );
    assert!(
        !not_ready[0]["reason"]
            .as_str()
            .expect("reason is a string")
            .is_empty(),
        "the reason must be non-empty"
    );

    db.teardown().await;
}

/// All mandatory components ready yields 200 `{"ready":true,"not_ready":[]}`.
#[tokio::test]
async fn all_mandatory_ready_yields_200() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("acta"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let response = reqwest::get(format!("{}/ready", server.base_url()))
        .await
        .expect("GET /ready");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json body");
    assert_eq!(body["ready"], Value::Bool(true));
    assert_eq!(
        body["not_ready"].as_array().expect("array").len(),
        0,
        "not_ready must be empty when every mandatory component is ready"
    );

    db.teardown().await;
}

/// Multiple not-ready components are all named, not just the first
/// (spec Scenario).
#[tokio::test]
async fn multiple_not_ready_components_are_all_named() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(
                HealthStatus::Ok,
                ReadinessStatus::NotReady {
                    reason: "warming up".to_string(),
                },
            ),
        ),
        (
            component("acta"),
            fake(
                HealthStatus::Ok,
                ReadinessStatus::NotReady {
                    reason: "waiting on custos".to_string(),
                },
            ),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let response = reqwest::get(format!("{}/ready", server.base_url()))
        .await
        .expect("GET /ready");

    assert_eq!(response.status(), 503);
    let body: Value = response.json().await.expect("json body");
    let not_ready = body["not_ready"].as_array().expect("array");
    assert_eq!(not_ready.len(), 2, "both must be named: {body}");

    let names: Vec<&str> = not_ready
        .iter()
        .map(|entry| entry["component"].as_str().expect("component is a string"))
        .collect();
    assert!(names.contains(&"custos"));
    assert!(names.contains(&"acta"));

    db.teardown().await;
}

/// Budget test (design R1, T1.28-T1.29): every mandatory component stalling
/// past the bound is turned into `NotReady` within `3 * per_component`, and
/// every one is listed — `TokioDeadline` is actually wired, not merely
/// implemented. `with_readiness_timeout` keeps the wall-clock cost of this
/// container test bounded.
#[tokio::test]
async fn readiness_budget_bounds_the_worst_case_and_lists_every_stalling_component() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (component("platform"), never_resolves()),
        (component("custos"), never_resolves()),
        (component("acta"), never_resolves()),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let per_component = Duration::from_millis(200);
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics)
        .with_readiness_timeout(per_component);
    let server = support::TestServer::spawn_with_state(state).await;

    let started = std::time::Instant::now();
    let response = reqwest::get(format!("{}/ready", server.base_url()))
        .await
        .expect("GET /ready");
    let elapsed = started.elapsed();

    assert_eq!(response.status(), 503);
    assert!(
        elapsed < per_component * 3 + Duration::from_secs(1),
        "the whole request must return within roughly 3 * per_component, took {elapsed:?}"
    );

    let body: Value = response.json().await.expect("json body");
    let not_ready = body["not_ready"].as_array().expect("array");
    assert_eq!(
        not_ready.len(),
        3,
        "every one of the 3 mandatory components must be listed: {body}"
    );

    db.teardown().await;
}

/// No secret ever appears in a probe payload (SHELL-CFG-2): a readiness
/// failure reason names no credential value, no connection string.
#[tokio::test]
async fn readiness_failure_reason_names_no_secret_or_connection_string() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(
                HealthStatus::Ok,
                ReadinessStatus::NotReady {
                    reason: "database is unreachable".to_string(),
                },
            ),
        ),
        (
            component("acta"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let response = reqwest::get(format!("{}/ready", server.base_url()))
        .await
        .expect("GET /ready");
    let body = response.text().await.expect("body text");

    assert!(
        !body.contains("://"),
        "no connection string may appear: {body}"
    );
    assert!(
        !body.to_uppercase().contains("DATABASE_URL"),
        "no DATABASE_URL sentinel may appear: {body}"
    );
    assert!(
        !body.to_lowercase().contains("password"),
        "no password sentinel may appear: {body}"
    );

    db.teardown().await;
}

/// A component's own health probe reflects its own signal only (spec
/// Scenario).
#[tokio::test]
async fn a_components_own_health_probe_reflects_its_own_signal_only() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("acta"),
            fake(
                HealthStatus::Degraded {
                    reason: "cache miss rate high".to_string(),
                },
                ReadinessStatus::Ready,
            ),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let response = reqwest::get(format!("{}/api/v2/acta/health", server.base_url()))
        .await
        .expect("GET /api/v2/acta/health");

    assert_eq!(response.status(), 200);
    let body: Value = response.json().await.expect("json body");
    assert_eq!(body["status"], Value::String("degraded".to_string()));
    assert_eq!(
        body["reason"],
        Value::String("cache miss rate high".to_string())
    );

    db.teardown().await;
}

/// A component's readiness probe is not the aggregate (spec Scenario):
/// A component's own `/ready` is bounded by the same per-component budget
/// as root `/ready`: a stalling probe answers 503 with the deadline reason.
#[tokio::test]
async fn a_components_readiness_probe_is_bounded_by_the_shared_deadline() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (component("acta"), never_resolves()),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let per_component = Duration::from_millis(200);
    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics)
        .with_readiness_timeout(per_component);
    let server = support::TestServer::spawn_with_state(state).await;

    let started = std::time::Instant::now();
    let response = reqwest::get(format!("{}/api/v2/acta/ready", server.base_url()))
        .await
        .expect("GET /api/v2/acta/ready");
    let elapsed = started.elapsed();

    assert_eq!(response.status(), 503);
    assert!(
        elapsed < per_component + Duration::from_secs(1),
        "a stalling component probe must return within roughly per_component, took {elapsed:?}"
    );
    let body: Value = response.json().await.expect("json body");
    assert_eq!(
        body["reason"],
        Value::String("readiness check exceeded its deadline".to_string())
    );

    db.teardown().await;
}

/// Custos not ready must not affect Acta's own `/ready`, and vice versa.
#[tokio::test]
async fn a_components_readiness_probe_is_not_reinterpreted_by_the_shell() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let registry =
        build(reg5_component_entries(StorageBackend::Filesystem)).expect("valid registry");

    let table = vec![
        (
            component("platform"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
        (
            component("custos"),
            fake(
                HealthStatus::Ok,
                ReadinessStatus::NotReady {
                    reason: "warming up".to_string(),
                },
            ),
        ),
        (
            component("acta"),
            fake(HealthStatus::Ok, ReadinessStatus::Ready),
        ),
    ];
    let diagnostics = DiagnosticsRegistry::bind(&registry, table).expect("valid binding");

    let state = atlas_server::state::AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_diagnostics(diagnostics);
    let server = support::TestServer::spawn_with_state(state).await;

    let acta_ready = reqwest::get(format!("{}/api/v2/acta/ready", server.base_url()))
        .await
        .expect("GET /api/v2/acta/ready");
    assert_eq!(
        acta_ready.status(),
        200,
        "acta's own readiness must be independent of custos's state"
    );

    let custos_ready = reqwest::get(format!("{}/api/v2/custos/ready", server.base_url()))
        .await
        .expect("GET /api/v2/custos/ready");
    assert_eq!(custos_ready.status(), 503);
    let custos_body: Value = custos_ready.json().await.expect("json body");
    assert_eq!(
        custos_body["component"],
        Value::String("custos".to_string())
    );

    db.teardown().await;
}
