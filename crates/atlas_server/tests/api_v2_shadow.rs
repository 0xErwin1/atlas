//! Shadow authorization (`v2-e7-s2`): with the shadow on, every response
//! is byte-identical to the shadow-off response for the same request, on a
//! fixed matrix of Acta routes and outcomes (allow, 404 for a non-member,
//! 404 for a missing resource, 403 for an unscoped key); with the shadow
//! off, the probe never activates, so no V2 question is built.
//!
//! The shadow's own outcome is observed through its log line and counter,
//! never through the response: the mismatch test drives the router on the
//! test thread with a local metrics recorder and a capturing subscriber
//! installed, so the counter and the `authz.v2_shadow` line are read
//! directly while the client still sees the V1 answer.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use metrics_exporter_prometheus::PrometheusBuilder;
use support::path::api_url;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use atlas_api::dtos::CreateProjectRequest;
use atlas_api::dtos::documents::CreateDocumentRequest;
use atlas_core::capabilities::{
    CapabilityError, ProviderCatalog, ResourceExistence, ResourceFacts, ResourceProvider,
};
use atlas_core::ids::{PrincipalId, PrincipalSetId, ResourceRef};
use atlas_custos::authorize::{AuthorizationService, AuthorizationSettings, ProviderSet};
use atlas_custos::capability::Capability;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey};
use atlas_custos::eval::DenyMode;
use atlas_custos_postgres::repos::authorize::{PgAuthorizationFactsStore, PgGroupMembershipSource};
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};
use atlas_server::auth::tokens::{generate_api_key, hash_token};
use atlas_server::authz::v2_service::{TokioSleeper, validation_catalog};
use atlas_server::authz::v2_shadow::{SHADOW_TOTAL, ShadowProbe};
use atlas_server::config::ShadowMode;
use atlas_server::state::AppState;

/// One request of the matrix: the bearer token to present and the relative
/// Acta path to fetch.
struct Probe {
    label: &'static str,
    token: String,
    path: String,
}

/// `(status, body)` of `GET /api/v2/acta/<path>` with `token`.
async fn fetch(server: &support::TestServer, token: &str, path: &str) -> (u16, String) {
    let response = reqwest::Client::new()
        .get(api_url(server.base_url(), "acta", path))
        .bearer_auth(token)
        .send()
        .await
        .expect("request");
    let status = response.status().as_u16();
    let body = response.text().await.expect("body");

    (status, body)
}

async fn spawn(db: &support::TestDb, mode: ShadowMode) -> support::TestServer {
    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_shadow_mode(mode);

    support::TestServer::spawn_with_state(state).await
}

/// Builds the request matrix against a fresh server so both servers share
/// the same database rows: a workspace owner, a non-member, an agent key
/// without any read scope, an existing project and a missing document.
async fn matrix(server: &support::TestServer, db: &support::TestDb) -> Vec<Probe> {
    let (owner_client, ws, owner) =
        support::login_user_with_workspace(server, db, "shadow-owner").await;
    let (outsider_client, _outsider) = support::login_user(server, db, "shadow-outsider").await;
    let project = owner_client
        .acta()
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Shadow Project".to_string(),
                slug: "shadow-project".to_string(),
                task_prefix: "SHD".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let raw_key = generate_api_key();
    let ctx = support::ctx(&ws, &owner);
    PgApiKeyRepo {
        conn: db.conn().clone(),
    }
    .create(
        atlas_custos::WorkspaceScope(ctx.workspace_id.0),
        &ctx.actor,
        NewApiKey {
            name: "shadow-key".to_string(),
            token_hash: hash_token(&raw_key),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes: vec![Capability {
                family: atlas_custos::capability::CapabilityFamily::Tasks,
                action: atlas_custos::capability::CapabilityAction::Read,
            }],
        },
    )
    .await
    .expect("create api key");

    let owner_token = owner_client.token().expect("owner token").to_string();
    let outsider_token = outsider_client.token().expect("outsider token").to_string();

    vec![
        Probe {
            label: "owner reads the workspace",
            token: owner_token.clone(),
            path: format!("/workspaces/{}", ws.slug),
        },
        Probe {
            label: "owner reads a project",
            token: owner_token.clone(),
            path: format!("/workspaces/{}/projects/{}", ws.slug, project.slug),
        },
        Probe {
            label: "owner lists projects",
            token: owner_token.clone(),
            path: format!("/workspaces/{}/projects", ws.slug),
        },
        Probe {
            label: "owner reads a missing document",
            token: owner_token.clone(),
            path: format!("/workspaces/{}/documents/no-such-document", ws.slug),
        },
        Probe {
            label: "a non-member is told nothing exists",
            token: outsider_token,
            path: format!("/workspaces/{}", ws.slug),
        },
        Probe {
            label: "a key without the read scope is refused",
            token: raw_key,
            path: format!("/workspaces/{}/projects", ws.slug),
        },
    ]
}

#[tokio::test]
async fn responses_are_byte_identical_with_the_shadow_on_and_off() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let off = spawn(&db, ShadowMode::Off).await;
    let log = spawn(&db, ShadowMode::Log).await;
    let metrics = spawn(&db, ShadowMode::Metrics).await;
    let probes = matrix(&off, &db).await;

    let mut statuses = Vec::new();
    for probe in &probes {
        let baseline = fetch(&off, &probe.token, &probe.path).await;
        let shadowed_log = fetch(&log, &probe.token, &probe.path).await;
        let shadowed_metrics = fetch(&metrics, &probe.token, &probe.path).await;

        assert_eq!(
            baseline, shadowed_log,
            "{}: shadow log changed the response",
            probe.label
        );
        assert_eq!(
            baseline, shadowed_metrics,
            "{}: shadow metrics changed the response",
            probe.label
        );
        statuses.push(baseline.0);
    }

    assert_eq!(
        statuses,
        [200, 200, 200, 404, 404, 403],
        "the matrix exercises an allow, a missing resource, a non-member and an unscoped key"
    );

    db.teardown().await;
}

/// With the shadow off the probe never activates, so no principal, target
/// or V2 question is ever built; with it on, only routes that declare a V2
/// target activate it.
#[tokio::test]
async fn the_probe_activates_only_when_the_shadow_is_on_and_the_route_declares_a_target() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let off = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let declared = "/api/v2/acta/workspaces/{ws}/documents/{slug}";
    assert!(!ShadowProbe::for_route(&off, &Method::GET, declared).active());

    let on = off.with_shadow_mode(ShadowMode::Log);
    assert!(ShadowProbe::for_route(&on, &Method::GET, declared).active());
    assert!(
        !ShadowProbe::for_route(&on, &Method::GET, "/api/v2/acta/admin/trash").active(),
        "platform-scoped routes declare no target"
    );
    assert!(
        !ShadowProbe::for_route(&on, &Method::GET, "/api/v2/custos/roles").active(),
        "custos routes declare no target"
    );

    db.teardown().await;
}

/// An Acta provider for which every canonical reference exists at the root:
/// the shadow's question reaches the evaluator with real V2 rows (none for
/// the actor) instead of failing on the missing product provider.
struct EveryActaResourceExists;

#[async_trait]
impl ResourceProvider for EveryActaResourceExists {
    async fn validate_ref(&self, _resource: &ResourceRef) -> Result<bool, CapabilityError> {
        Ok(true)
    }

    async fn path_of(&self, _resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError> {
        Ok(Vec::new())
    }

    async fn ancestors(
        &self,
        _resource: &ResourceRef,
    ) -> Result<Vec<ResourceRef>, CapabilityError> {
        Ok(Vec::new())
    }

    async fn members_of(&self, _set: &PrincipalSetId) -> Result<Vec<PrincipalId>, CapabilityError> {
        Ok(Vec::new())
    }

    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
        Ok(ProviderCatalog {
            resource_kinds: Vec::new(),
            actions: Vec::new(),
            role_definitions: Vec::new(),
            principal_sets: Vec::new(),
            role_definitions_v2: Vec::new(),
        })
    }

    async fn resource_facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        Ok(resources
            .iter()
            .map(|resource| ResourceFacts {
                resource: resource.clone(),
                existence: ResourceExistence::Exists,
                path: None,
            })
            .collect())
    }
}

/// `state` in `Metrics` mode with an authorization service whose Acta
/// provider is [`EveryActaResourceExists`], over the real V2 stores.
fn shadowed_state(state: AppState, db: &support::TestDb) -> AppState {
    let settings = AuthorizationSettings {
        catalog: validation_catalog(&state.registry).expect("catalog"),
        deny_mode: DenyMode::Disabled,
        provider_timeout: Duration::from_millis(500),
    };
    let service = AuthorizationService::new(
        ProviderSet::new().with("acta", Arc::new(EveryActaResourceExists)),
        PgAuthorizationFactsStore {
            conn: db.conn().clone(),
        },
        PgGroupMembershipSource {
            conn: db.conn().clone(),
        },
        Arc::new(TokioSleeper),
        settings,
    );

    state
        .with_authorization_service(Arc::new(service))
        .with_shadow_mode(ShadowMode::Metrics)
}

/// A `tracing_subscriber::Layer` that records the fields of every
/// `authz.v2_shadow` event, as `span_fields.rs` does for the `http` span.
#[derive(Clone, Default)]
struct ShadowLineCapture {
    lines: Arc<Mutex<Vec<HashMap<String, String>>>>,
}

impl ShadowLineCapture {
    fn lines(&self) -> Vec<HashMap<String, String>> {
        self.lines.lock().unwrap().clone()
    }
}

struct FieldRecorder<'a>(&'a mut HashMap<String, String>);

impl Visit for FieldRecorder<'_> {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

impl<S> Layer<S> for ShadowLineCapture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        if event.metadata().target() != "authz.v2_shadow" {
            return;
        }
        let mut fields = HashMap::new();
        fields.insert("level".to_string(), event.metadata().level().to_string());
        event.record(&mut FieldRecorder(&mut fields));
        self.lines.lock().unwrap().push(fields);
    }
}

/// Drives `app` with `GET /api/v2/acta/<path>` as `token` on this thread,
/// returning the status; the caller's recorder and subscriber see the
/// shadow's counter and line because nothing leaves the thread.
async fn get_on_thread(app: axum::Router, token: &str, path: &str) -> StatusCode {
    let request = Request::builder()
        .method(Method::GET)
        .uri(format!("/api/v2/acta{path}"))
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();

    app.oneshot(request).await.unwrap().status()
}

fn series(rendered: &str, operation: &str, outcome: &str) -> bool {
    rendered.contains(&format!(
        "{SHADOW_TOTAL}{{component=\"acta\",operation=\"{operation}\",outcome=\"{outcome}\"}} 1"
    ))
}

/// The owner reads a document V1 allows while V2 holds no grant for them:
/// the response is still 200, the counter and the info line record the
/// `v1_allow_v2_not_found` mismatch (an existing target with no effective
/// action is `not_found` to the evaluator). A read of a missing document
/// refuses before any target is resolved and is recorded as `skipped`.
#[tokio::test]
async fn a_v1_allow_without_a_v2_grant_is_counted_and_logged_as_a_mismatch() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let setup = spawn(&db, ShadowMode::Off).await;
    let (owner_client, ws, _owner) =
        support::login_user_with_workspace(&setup, &db, "shadow-mismatch").await;
    let project = owner_client
        .acta()
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Mismatch Project".to_string(),
                slug: "mismatch-project".to_string(),
                task_prefix: "MSM".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let document = owner_client
        .acta()
        .create_document(
            &ws.slug,
            &project.slug,
            CreateDocumentRequest {
                title: "Shadowed".to_string(),
                folder_id: None,
                content: None,
            },
        )
        .await
        .expect("create document");
    let slug = document.slug.expect("a created document has a slug");
    let token = owner_client.token().expect("owner token").to_string();

    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test");
    let app = atlas_server::app(shadowed_state(state, &db));

    let recorder = PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    let _recorder = metrics::set_default_local_recorder(&recorder);
    let capture = ShadowLineCapture::default();
    let _subscriber =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(capture.clone()));

    let allowed = get_on_thread(
        app.clone(),
        &token,
        &format!("/workspaces/{}/documents/{slug}", ws.slug),
    )
    .await;
    assert_eq!(allowed, StatusCode::OK, "V1 still decides the response");

    let missing = get_on_thread(
        app,
        &token,
        &format!("/workspaces/{}/documents/no-such-document", ws.slug),
    )
    .await;
    assert_eq!(missing, StatusCode::NOT_FOUND);

    let rendered = handle.render();
    assert!(
        series(&rendered, "get_document", "v1_allow_v2_not_found"),
        "expected one mismatch series for the allowed read, got:\n{rendered}"
    );
    assert!(
        series(&rendered, "get_document", "skipped"),
        "expected one skipped series for the missing document, got:\n{rendered}"
    );
    assert!(
        !rendered.contains("outcome=\"agree\"") && !rendered.contains("outcome=\"v2_unavailable\""),
        "no other outcome was expected, got:\n{rendered}"
    );

    let lines = capture.lines();
    let outcomes: Vec<(String, String)> = lines
        .iter()
        .map(|line| {
            (
                line.get("outcome").cloned().unwrap_or_default(),
                line.get("level").cloned().unwrap_or_default(),
            )
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            ("\"v1_allow_v2_not_found\"".to_string(), "INFO".to_string()),
            ("\"skipped\"".to_string(), "INFO".to_string()),
        ],
        "one info line per comparison, got: {lines:?}"
    );
    let mismatch = &lines[0];
    assert_eq!(
        mismatch.get("operation").map(String::as_str),
        Some("get_document")
    );
    assert_eq!(
        mismatch.get("action").map(String::as_str),
        Some("acta::document::read")
    );
    assert_eq!(mismatch.get("v1").map(String::as_str), Some("Allow"));
    assert_eq!(
        mismatch.get("v2").map(String::as_str),
        Some("\"not_found\"")
    );

    db.teardown().await;
}
