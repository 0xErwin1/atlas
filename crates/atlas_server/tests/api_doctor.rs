//! Container-backed tests for `POST /api/v2/platform/doctor` (E11-S3b design
//! D6.1): SH7 (platform-admin authorization matrix, with at least one seeded
//! finding) and SH3 (a downed non-critical worker is 200 on `/ready` but
//! named by doctor). Compiles clean via `cargo test -p atlas_server
//! --no-run`; execution is CI's job (no podman in this sandbox), same
//! posture every other container-backed suite in this crate documents.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::sync::Arc;

use atlas_api::dtos::{CreateUserApiKeyRequest, LoginRequest};
use atlas_client::{AtlasClient, ClientError};
use atlas_core::registry::{WorkerId, build};
use atlas_server::ops::workers::WorkerStates;
use atlas_server::persistence::repos::{NewUser, UserRepo};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::state::AppState;

/// Creates a fully activated `is_system_admin` user with a real password,
/// logs in, and returns the authenticated client (mirrors
/// `api_self_protection.rs`'s own local helper — not shared through
/// `support::` since that module carries only `is_root` login today).
async fn login_system_admin(
    server: &support::TestServer,
    db: &support::TestDb,
    username: &str,
) -> AtlasClient {
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash password");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: true,
        })
        .await
        .expect("create system admin");

    support::activate_user_in_db(db, user.id.0).await;

    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login system admin");

    client
}

/// Builds an `AppState` whose worker table has exactly one worker
/// `Failed`, every other declared worker `Running` — the SH3 seam
/// (`AppState::with_workers`, design R11), never a real process kill.
async fn state_with_one_worker_failed(db: &support::TestDb, failed: &str) -> AppState {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must build");
    let failed_id = WorkerId::new(failed).expect("valid worker id");

    let workers = WorkerStates::from_registry(&registry);
    for status in workers.all() {
        let handle = workers
            .handle(&status.id)
            .expect("from_registry binds a handle for every declared worker");
        if status.id == failed_id {
            handle.failed("forced failed for SH3/SH7 container test");
        } else {
            handle.running();
        }
    }

    AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_workers(Arc::new(workers))
}

/// SH7 (spec Scenario, permanent regression test): platform admin and
/// `is_system_admin` both get 200 with the seeded finding present, shaped
/// `{component, severity, finding, action}`; a plain workspace member and
/// an API key both get 403; an anonymous caller gets 401.
#[tokio::test]
async fn sh7_platform_admin_authorization_matrix_with_a_seeded_finding() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = state_with_one_worker_failed(&db, "acta.webhook_dispatcher").await;
    let server = support::TestServer::spawn_with_state(state).await;

    let root = support::login_root_user(&server, &db).await;
    let report = root
        .platform()
        .doctor()
        .await
        .expect("root must be able to run doctor");
    assert!(
        !report.findings.is_empty(),
        "the forced-failed worker must produce at least one finding"
    );
    let dispatcher_finding = report
        .findings
        .iter()
        .find(|f| f.finding.contains("acta.webhook_dispatcher"));
    assert!(
        dispatcher_finding.is_some(),
        "the report must name the forced-failed worker: {:?}",
        report.findings
    );
    for f in &report.findings {
        assert!(!f.component.is_empty(), "component must be present");
        assert!(!f.severity.is_empty(), "severity must be present");
        assert!(!f.finding.is_empty(), "finding must be present");
        assert!(!f.action.is_empty(), "action must be present");
    }

    let system_admin = login_system_admin(&server, &db, "doctor-system-admin").await;
    let system_admin_report = system_admin
        .platform()
        .doctor()
        .await
        .expect("is_system_admin must be able to run doctor");
    assert!(!system_admin_report.findings.is_empty());

    let (plain, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "doctor-plain-user").await;
    let plain_err = plain
        .platform()
        .doctor()
        .await
        .expect_err("a plain workspace owner is not an Atlas admin");
    assert!(
        matches!(plain_err, ClientError::Api(ref p) if p.status == 403),
        "expected 403 for a plain user, got {plain_err:?}"
    );

    let api_key = plain
        .custos()
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "doctor-agent".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: None,
        })
        .await
        .expect("create API key");
    let agent = AtlasClient::new(server.base_url()).with_token(api_key.secret);
    let agent_err = agent
        .platform()
        .doctor()
        .await
        .expect_err("API keys must not run doctor");
    assert!(
        matches!(agent_err, ClientError::Api(ref p) if p.status == 403),
        "expected 403 for an API key, got {agent_err:?}"
    );

    let anonymous = AtlasClient::new(server.base_url());
    let anon_err = anonymous
        .platform()
        .doctor()
        .await
        .expect_err("anonymous callers must not run doctor");
    assert!(
        matches!(anon_err, ClientError::Api(ref p) if p.status == 401),
        "expected 401 for an anonymous caller, got {anon_err:?}"
    );

    db.teardown().await;
}

/// SH3 (spec Scenario, permanent regression test, this slice's headline
/// gate): the dispatcher worker forced `Failed` still answers `GET /ready`
/// 200 (all six REG-5 workers are `critical: false`), and doctor's report,
/// requested immediately after, names the dispatcher worker.
#[tokio::test]
async fn sh3_a_downed_worker_is_ready_200_and_named_by_doctor() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let state = state_with_one_worker_failed(&db, "acta.webhook_dispatcher").await;
    let server = support::TestServer::spawn_with_state(state).await;
    let root = support::login_root_user(&server, &db).await;

    let http = reqwest::Client::new();
    let ready_response = http
        .get(format!("{}/ready", server.base_url()))
        .send()
        .await
        .expect("GET /ready must not error");
    assert_eq!(
        ready_response.status().as_u16(),
        200,
        "a non-critical worker being Failed must not affect /ready"
    );

    let report = root
        .platform()
        .doctor()
        .await
        .expect("root must be able to run doctor");
    let dispatcher_finding = report
        .findings
        .iter()
        .find(|f| f.finding.contains("acta.webhook_dispatcher"));
    assert!(
        dispatcher_finding.is_some(),
        "doctor must name the forced-failed dispatcher worker even though /ready is 200: {:?}",
        report.findings
    );

    db.teardown().await;
}
