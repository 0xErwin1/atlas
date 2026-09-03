#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

//! `v2-e3-s3` PR4 (T4.18-T4.20): the idempotency live sweep, container-backed
//! (Postgres via `TestDb`), driven against the REAL server for every
//! declared-`idempotent: true` route.
//!
//! Data-driven off `reg5.rs` via the registry (never a hand list): the set
//! of routes exercised is `declared_routes()` filtered to `idempotent ==
//! true` (34 routes today), sorted by `(method, path)` for a reproducible
//! failure name. `body_for`'s match FAILS, naming the route, for any
//! declared-true route it has no provisioning arm for (INV-DATA-DRIVEN,
//! mirrors `api_page_conformance.rs`'s own `unregistered` check) — a new
//! declared-true route cannot silently skip this sweep.
//!
//! Per route: request 1 (fresh `Idempotency-Key`) gets the handler's normal
//! status; request 2 (identical key AND body) gets a status+body IDENTICAL
//! to request 1 (never a literal — some handlers' success status depends on
//! the body) plus `Idempotent-Replayed: true`; request 3 (same key,
//! different body) gets 409 `urn:atlas:error:idempotency-key-conflict`.
//! Request 3's body never needs to be semantically valid for the target
//! handler — the mismatch is caught by the idempotency middleware BEFORE
//! the handler's own extractors ever run.
//!
//! The six attachment-upload routes (`upload_attachment`,
//! `upload_document_comment_attachment`,
//! `upload_document_comment_draft_attachment`, `upload_task_attachment`,
//! `upload_task_comment_attachment`, `upload_task_comment_draft_attachment`)
//! are declared `idempotent: false` (F4, `R4-upload-bodies-buffered-in-
//! memory`: dedup would require buffering a streamed upload body in
//! memory) and are excluded from this sweep's true-route set entirely —
//! every route this sweep drives takes a plain JSON body.
//!
//! Coverage is filled only inside the arm that actually exercised a route
//! (never pre-seeded), and compared as a SET, both directions, against the
//! declared-true set (INV-SET) — a route with no arm fails by name before
//! any coverage check runs; an arm that runs but forgets to record coverage
//! is caught the same way a genuinely-missing route is.

mod support;

use std::collections::HashSet;

use atlas_api::dtos::boards_tasks::{
    CreateBoardRequest, CreateChecklistItemRequest, CreateColumnRequest, CreateReferenceRequest,
    CreateSubtaskRequest, CreateTaskRequest,
};
use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, CreateUserRequest, GrantPrincipal,
};
use atlas_client::AtlasClient;
use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use support::{
    TestDb, TestServer, activate_user_in_db, login_root_user, login_user_with_workspace,
};

// ---------------------------------------------------------------------------
// Request/response plumbing
// ---------------------------------------------------------------------------

/// The request-body shape every declared-true route needs. The six
/// attachment-upload routes are declared `idempotent: false` (F4,
/// `R4-upload-bodies-buffered-in-memory`: dedup would require buffering a
/// streamed upload body in memory), so this sweep no longer needs a
/// raw/multipart body shape at all — every declared-true route today takes
/// a plain JSON body.
#[derive(Clone)]
enum ReqBody {
    Json(serde_json::Value),
}

impl ReqBody {
    /// Produces a body that is byte-DIFFERENT from `self` while remaining
    /// syntactically acceptable to send (never required to be semantically
    /// valid for the target handler — the idempotency middleware's
    /// fingerprint mismatch short-circuits before any handler extractor
    /// runs).
    fn varied(&self) -> ReqBody {
        match self {
            ReqBody::Json(value) => {
                let mut object = match value {
                    serde_json::Value::Object(map) => map.clone(),
                    other => {
                        let mut map = serde_json::Map::new();
                        map.insert("__base__".to_string(), other.clone());
                        map
                    }
                };
                object.insert(
                    "__sweep_variant__".to_string(),
                    serde_json::Value::Bool(true),
                );
                ReqBody::Json(serde_json::Value::Object(object))
            }
        }
    }
}

/// Sends one raw, authenticated POST carrying `Idempotency-Key: {key}`
/// (bypassing `AtlasClient`'s typed methods entirely, since none expose
/// custom header injection) plus any route-specific extra headers (e.g.
/// `x-create-token`).
async fn send(
    client: &AtlasClient,
    path: &str,
    key: &str,
    extra_headers: &[(&str, String)],
    body: &ReqBody,
) -> reqwest::Response {
    let mut builder = client
        .http_client()
        .post(format!("{}{}", client.base_url(), path))
        .header("x-atlas-csrf", "1")
        .header("idempotency-key", key);

    if let Some(token) = client.token() {
        builder = builder.bearer_auth(token);
    }
    for (name, value) in extra_headers {
        builder = builder.header(*name, value.clone());
    }

    builder = match body {
        ReqBody::Json(value) => builder.json(value),
    };

    builder
        .send()
        .await
        .unwrap_or_else(|e| panic!("POST {path}: request failed: {e}"))
}

/// The full three-request proof (T4.18) for one declared-`idempotent: true`
/// route: replay on retry, fingerprint-mismatch 409 on a different body.
async fn assert_idempotent_true(
    client: &AtlasClient,
    path: &str,
    extra_headers: &[(&str, String)],
    body: ReqBody,
) {
    let key = format!("sweep-{}", uuid::Uuid::now_v7());

    let first = send(client, path, &key, extra_headers, &body).await;
    let status1 = first.status();
    assert!(
        status1.is_success(),
        "{path}: first request (fresh key) must succeed, got {status1}: {:?}",
        first.text().await
    );
    let body1 = first
        .bytes()
        .await
        .unwrap_or_else(|e| panic!("{path}: reading first response body failed: {e}"));

    let second = send(client, path, &key, extra_headers, &body).await;
    let status2 = second.status();
    let replayed = second
        .headers()
        .get("idempotent-replayed")
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let body2 = second
        .bytes()
        .await
        .unwrap_or_else(|e| panic!("{path}: reading second response body failed: {e}"));
    assert_eq!(
        status2, status1,
        "{path}: replay status must equal the first response's status (never a literal)"
    );
    assert_eq!(
        body2, body1,
        "{path}: replay body must be byte-identical to the first response's body"
    );
    assert_eq!(
        replayed.as_deref(),
        Some("true"),
        "{path}: replay must carry Idempotent-Replayed: true"
    );

    let third = send(client, path, &key, extra_headers, &body.varied()).await;
    assert_eq!(
        third.status().as_u16(),
        409,
        "{path}: a different body under the same key must 409, got {}: {:?}",
        third.status(),
        third.text().await
    );
    let problem: serde_json::Value = third
        .json()
        .await
        .unwrap_or_else(|e| panic!("{path}: mismatch response was not JSON: {e}"));
    assert_eq!(
        problem.get("type").and_then(|v| v.as_str()),
        Some("urn:atlas:error:idempotency-key-conflict"),
        "{path}: mismatch must use the D1 conflict problem type: {problem}"
    );
}

/// The declared-`idempotent: false` header-ignored proof (T4.19), for the
/// self-checking sample only (see `DECLARED_FALSE_SAMPLE`). Also queries
/// `platform.idempotency_keys` directly to confirm the middleware never
/// wrote a row for this route's `Idempotency-Key` — the only live proof
/// that a hand-written `.layer()` omission and its `AuditedRoute.idempotent:
/// false` declaration agree, for the routes where no macro-level coupling
/// exists (see `docs/reg5-idempotent-judgment.md`).
async fn assert_idempotent_false(
    db: &TestDb,
    client: &AtlasClient,
    principal_id: uuid::Uuid,
    path: &str,
    body: ReqBody,
) {
    let key = format!("sweep-false-{}", uuid::Uuid::now_v7());

    let first = send(client, path, &key, &[], &body).await;
    assert!(
        first.status().is_success(),
        "{path}: first request must succeed, got {}: {:?}",
        first.status(),
        first.text().await
    );
    assert!(
        first.headers().get("idempotent-replayed").is_none(),
        "{path}: the ORIGINAL response must never carry Idempotent-Replayed"
    );

    let second = send(client, path, &key, &[], &body).await;
    assert!(
        second.status().is_success(),
        "{path}: a false route must not reject a retried Idempotency-Key, got {}: {:?}",
        second.status(),
        second.text().await
    );
    assert!(
        second.headers().get("idempotent-replayed").is_none(),
        "{path}: a declared-false route must silently ignore the header — the second \
         response must not be a replay of the first"
    );

    let count: i64 = sea_orm::ConnectionTrait::query_one_raw(
        db.conn(),
        sea_orm::Statement::from_sql_and_values(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT count(*) AS c FROM platform.idempotency_keys WHERE principal_id = $1 AND key = $2",
            [principal_id.into(), key.clone().into()],
        ),
    )
    .await
    .expect("count query must not error")
    .and_then(|row| row.try_get::<i64>("", "c").ok())
    .unwrap_or(0);
    assert_eq!(
        count, 0,
        "{path}: a declared-false route must never write a row to \
         platform.idempotency_keys for principal {principal_id} key {key}"
    );
}

// ---------------------------------------------------------------------------
// Route sets: exhaustive, data-driven off the registry (never a hand list)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Route {
    method: HttpMethod,
    path: String,
}

fn declared_true_routes() -> Vec<Route> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    let mut routes: Vec<Route> = registry
        .entries()
        .iter()
        .flat_map(|component| component.api.routes.iter())
        .filter(|route| route.idempotent)
        .map(|route| Route {
            method: route.method,
            path: route.path.as_str().to_string(),
        })
        .collect();

    routes.sort_by_key(|route| (format!("{:?}", route.method), route.path.clone()));
    routes
}

fn declared_false_set() -> HashSet<Route> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    registry
        .entries()
        .iter()
        .flat_map(|component| component.api.routes.iter())
        .filter(|route| !route.idempotent)
        .map(|route| Route {
            method: route.method,
            path: route.path.as_str().to_string(),
        })
        .collect()
}

/// T4.19's self-checking sample: a named list with a reason, asserted (in
/// the test below) to be a subset of the real declared-`idempotent: false`
/// set — never a curated list that could silently drift from the registry.
/// Chosen to span six distinct shapes: a no-body action
/// (`apply_status_templates`), two state-transition toggles
/// (`archive_board`/`unarchive_board`), a read/search-shaped POST
/// (`search_content`), a periodic ping (`heartbeat`), and one hand-declared
/// route from `acta.rs`'s `layered` module (`move_documents_batch`) — the
/// five judgment categories D8's rule names for `false`, per
/// `docs/reg5-idempotent-judgment.md`, plus the one shape the macro-driven
/// routes above cannot exercise: a `.route()` call where the `idempotency`
/// layer is hand-applied only to a sibling `MethodRouter` before `.merge()`,
/// not by the macro's `[idempotent]` token.
///
/// `logout` is deliberately NOT sampled here: it consumes the session, so a
/// retried request under the SAME session is unauthenticated rather than
/// "successfully ignoring the header" — it cannot prove T4.19's property
/// (a false route accepts a retried key and does not reject it) at all.
const DECLARED_FALSE_SAMPLE: &[(HttpMethod, &str, &str)] = &[
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/apply-status-templates",
        "no-body action, re-applying status templates skips columns that \
         already exist by name and is a no-op on retry",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/archive",
        "state-transition toggle, re-archiving is a no-op",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/unarchive",
        "state-transition toggle, opposite direction",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/{slug}/content/search",
        "read/search-shaped POST, the rule's explicit exception",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/boards/{board_id}/presence",
        "periodic heartbeat, intentionally re-sent on an interval",
    ),
    (
        HttpMethod::Post,
        "/workspaces/{ws}/documents/moves/batch",
        "hand-declared in acta.rs's layered module, mutates existing \
         documents' positions/parents, converges to the same end state on \
         retry — the only sampled route whose `.layer()` omission is a \
         hand-written fact independent of AuditedRoute.idempotent",
    ),
];

// ---------------------------------------------------------------------------
// T4.18/T4.20: the live sweep over every declared-idempotent:true route
// ---------------------------------------------------------------------------

/// Container-backed (Postgres via `TestDb`); confirmed to compile via
/// `cargo check --workspace --all-targets`, NOT run locally
/// (INV-CONTAINER-UNVERIFIABLE, rootless podman blocks
/// `ATLAS_TEST_DATABASE_URL` here) — CI shards are the actual gate.
#[tokio::test]
async fn every_declared_idempotent_true_route_replays_and_rejects_mismatch() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let true_routes = declared_true_routes();
    assert_eq!(
        true_routes.len(),
        34,
        "expected exactly 34 declared idempotent:true routes"
    );

    let mut covered: HashSet<Route> = HashSet::new();

    for route in &true_routes {
        assert_eq!(
            route.method,
            HttpMethod::Post,
            "every idempotent:true route is a POST"
        );

        match route.path.as_str() {
            // ---------------------------------------------------------------
            // custos: admin/root-scoped user management
            // ---------------------------------------------------------------
            "/users/{user_id}/reset-password" => {
                let root = login_root_user(&server, &db).await;
                let ws = root
                    .create_workspace("sweep-reset-ws")
                    .await
                    .expect("create workspace");
                let created = root
                    .create_user(CreateUserRequest {
                        username: format!("sweep-reset-{}", uuid::Uuid::now_v7().as_simple()),
                        display_name: "Reset Target".to_string(),
                        email: None,
                        workspace: ws.slug.clone(),
                        role: "member".to_string(),
                    })
                    .await
                    .expect("create user to reset");
                let path = support::path::api_path(
                    "custos",
                    &format!("/users/{}/reset-password", created.user.id),
                );
                let body = ReqBody::Json(serde_json::json!({ "new_password": "SweepPassword1!" }));
                assert_idempotent_true(&root, &path, &[], body).await;
            }
            "/users/{user_id}/activation-link" => {
                let root = login_root_user(&server, &db).await;
                let ws = root
                    .create_workspace("sweep-actlink-ws")
                    .await
                    .expect("create workspace");
                let created = root
                    .create_user(CreateUserRequest {
                        username: format!("sweep-actlink-{}", uuid::Uuid::now_v7().as_simple()),
                        display_name: "Activation Target".to_string(),
                        email: None,
                        workspace: ws.slug.clone(),
                        role: "member".to_string(),
                    })
                    .await
                    .expect("create user for activation-link regen");
                let path = support::path::api_path(
                    "custos",
                    &format!("/users/{}/activation-link", created.user.id),
                );
                assert_idempotent_true(&root, &path, &[], ReqBody::Json(serde_json::json!({})))
                    .await;
            }
            "/api-keys" => {
                let (client, _ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-apikeys").await;
                let body = ReqBody::Json(serde_json::json!({ "name": "Sweep Key" }));
                assert_idempotent_true(
                    &client,
                    &support::path::api_path("custos", route.path.as_str()),
                    &[],
                    body,
                )
                .await;
            }

            // ---------------------------------------------------------------
            // custos: grants and groups
            // ---------------------------------------------------------------
            "/workspaces/{ws}/projects/{project_slug}/grants" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-projgrant").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let second_user_id = second_platform_user(&db, "sweep-projgrant-2").await;
                client
                    .add_member(&ws.slug, second_user_id, "member")
                    .await
                    .expect("add second user to workspace so it is a valid grant target");
                let path = support::path::api_path(
                    "custos",
                    &format!("/workspaces/{}/projects/{}/grants", ws.slug, project.slug),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateGrantRequest {
                        principal: GrantPrincipal {
                            r#type: "user".to_string(),
                            id: second_user_id,
                        },
                        role: "viewer".to_string(),
                    })
                    .expect("serialize CreateGrantRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/grants" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-wsgrant").await;
                let second_user_id = second_platform_user(&db, "sweep-wsgrant-2").await;
                client
                    .add_member(&ws.slug, second_user_id, "member")
                    .await
                    .expect("add second user to workspace so it is a valid grant target");
                let path =
                    support::path::api_path("custos", &format!("/workspaces/{}/grants", ws.slug));
                let body = ReqBody::Json(
                    serde_json::to_value(CreateGrantRequest {
                        principal: GrantPrincipal {
                            r#type: "user".to_string(),
                            id: second_user_id,
                        },
                        role: "viewer".to_string(),
                    })
                    .expect("serialize CreateGrantRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/groups" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-group").await;
                let path =
                    support::path::api_path("custos", &format!("/workspaces/{}/groups", ws.slug));
                let body = ReqBody::Json(serde_json::json!({ "name": "Sweep Group" }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/groups/{group_id}/members" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-groupmember").await;
                let group = client
                    .create_group(
                        &ws.slug,
                        atlas_api::dtos::groups::CreateGroupRequest {
                            name: "Sweep Group".to_string(),
                        },
                    )
                    .await
                    .expect("create group");
                let second_user_id = second_platform_user(&db, "sweep-groupmember-2").await;
                client
                    .add_member(&ws.slug, second_user_id, "member")
                    .await
                    .expect("add second user to workspace so it is a valid group-member target");
                let path = support::path::api_path(
                    "custos",
                    &format!("/workspaces/{}/groups/{}/members", ws.slug, group.id),
                );
                let body = ReqBody::Json(serde_json::json!({ "user_id": second_user_id }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }

            // ---------------------------------------------------------------
            // acta: workspace/project/member/tag/status-template/etc.
            // ---------------------------------------------------------------
            "/workspaces" => {
                let (client, _ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-newws").await;
                let body = ReqBody::Json(
                    serde_json::json!({ "name": format!("Sweep WS {}", uuid::Uuid::now_v7()) }),
                );
                assert_idempotent_true(
                    &client,
                    &support::path::api_path("acta", route.path.as_str()),
                    &[],
                    body,
                )
                .await;
            }
            "/admin/status-templates" => {
                let root = login_root_user(&server, &db).await;
                let body = ReqBody::Json(serde_json::json!({
                    "name": format!("Sweep Status {}", uuid::Uuid::now_v7()),
                    "color": null,
                    "before": null,
                    "after": null
                }));
                assert_idempotent_true(
                    &root,
                    &support::path::api_path("acta", route.path.as_str()),
                    &[],
                    body,
                )
                .await;
            }
            "/workspaces/{ws}/projects" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-newproj").await;
                let path =
                    support::path::api_path("acta", &format!("/workspaces/{}/projects", ws.slug));
                let body = ReqBody::Json(serde_json::json!({
                    "name": "Sweep Project",
                    "slug": "sweep-project",
                    "task_prefix": "SWP",
                    "visibility": null,
                    "visibility_role": null
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/members" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-addmember").await;
                let second_user_id = second_platform_user(&db, "sweep-addmember-2").await;
                let path =
                    support::path::api_path("acta", &format!("/workspaces/{}/members", ws.slug));
                let body = ReqBody::Json(
                    serde_json::json!({ "user_id": second_user_id, "role": "member" }),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tags" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-tag").await;
                let path =
                    support::path::api_path("acta", &format!("/workspaces/{}/tags", ws.slug));
                let body = ReqBody::Json(serde_json::json!({ "name": "sweep-tag" }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/status-templates" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-statustpl").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/status-templates", ws.slug),
                );
                let body = ReqBody::Json(serde_json::json!({
                    "name": "Sweep Status",
                    "color": null,
                    "before": null,
                    "after": null
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/property-definitions" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-propdef").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/property-definitions", ws.slug),
                );
                let body = ReqBody::Json(serde_json::json!({
                    "name": "Sweep Prop",
                    "kind": "text",
                    "options": null,
                    "applies_to": null
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/saved-searches" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-savedsearch").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/saved-searches", ws.slug),
                );
                let body = ReqBody::Json(
                    serde_json::json!({ "name": "Sweep Search", "query": "is:open" }),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/task-views" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-taskview").await;
                let path =
                    support::path::api_path("acta", &format!("/workspaces/{}/task-views", ws.slug));
                let body = ReqBody::Json(serde_json::json!({
                    "name": "Sweep View",
                    "filters": {}
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/admin/trash/purge" => {
                // Isolated database: a global, admin-scoped action; no other
                // arm needs to run first for this to be meaningful, and an
                // isolated db keeps this arm from depending on any other
                // arm's ordering (mirrors `api_page_conformance.rs`'s own
                // `/api/admin/trash`/`/api/admin/audit` isolation pattern).
                let admin_db = TestDb::create().await.expect("TestDb::create");
                let admin_server = TestServer::spawn(&admin_db).await;
                let root = login_root_user(&admin_server, &admin_db).await;
                let ws = root
                    .create_workspace("sweep-purge-ws")
                    .await
                    .expect("create workspace");
                let project = root
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                root.delete_project(&ws.slug, &project.slug)
                    .await
                    .expect("soft-delete project so trash/purge has a target");
                let body = ReqBody::Json(serde_json::json!({
                    "kind": "project",
                    "target_id": project.id,
                    "confirm": true
                }));
                assert_idempotent_true(
                    &root,
                    &support::path::api_path("acta", route.path.as_str()),
                    &[],
                    body,
                )
                .await;
                admin_db.teardown().await;
            }

            // ---------------------------------------------------------------
            // acta: boards/columns/tasks
            // ---------------------------------------------------------------
            "/workspaces/{ws}/projects/{project_slug}/boards" => {
                let (client, ws, user) =
                    login_user_with_workspace(&server, &db, "sweep-board").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/projects/{}/boards", ws.slug, project.slug),
                );
                let _ = &user;
                let body = ReqBody::Json(
                    serde_json::to_value(CreateBoardRequest {
                        name: "Sweep Board".to_string(),
                        folder_id: None,
                    })
                    .expect("serialize CreateBoardRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/boards/{board_id}/columns" => {
                let (client, ws, board_id, _column_id) =
                    provision_board(&server, &db, "sweep-column").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/boards/{}/columns", ws.slug, board_id),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateColumnRequest {
                        name: "Sweep Column".to_string(),
                        color: None,
                        before: None,
                        after: None,
                    })
                    .expect("serialize CreateColumnRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/boards/{board_id}/tasks" => {
                let (client, ws, board_id, column_id) =
                    provision_board(&server, &db, "sweep-task").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/boards/{}/tasks", ws.slug, board_id),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateTaskRequest {
                        column_id,
                        title: "Sweep Task".to_string(),
                        description: None,
                        properties: None,
                        before: None,
                        after: None,
                        references: Vec::new(),
                    })
                    .expect("serialize CreateTaskRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/assignees" => {
                let (client, ws, task, _second_id) =
                    provision_task(&server, &db, "sweep-assignee").await;
                let self_id = client
                    .me()
                    .await
                    .expect("fetch self")
                    .id
                    .expect("authenticated human user has an id");
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/assignees",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(
                    serde_json::json!({ "assignee_type": "user", "assignee_id": self_id }),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/references" => {
                let (client, ws, task, second_task) =
                    provision_task_pair(&server, &db, "sweep-reference").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/references",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateReferenceRequest {
                        kind: "relates".to_string(),
                        target_task_readable_id: Some(second_task.readable_id.clone()),
                        target_document_id: None,
                    })
                    .expect("serialize CreateReferenceRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/references/batch" => {
                let (client, ws, task, second_task) =
                    provision_task_pair(&server, &db, "sweep-refbatch").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/references/batch",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(serde_json::json!({
                    "references": [{
                        "kind": "relates",
                        "target_task_readable_id": second_task.readable_id,
                    }]
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/comment-drafts" => {
                let (client, ws, task, _second_id) =
                    provision_task(&server, &db, "sweep-taskdraft").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/comment-drafts",
                        ws.slug, task.readable_id
                    ),
                );
                let headers = vec![("x-create-token", uuid::Uuid::now_v7().to_string())];
                assert_idempotent_true(
                    &client,
                    &path,
                    &headers,
                    ReqBody::Json(serde_json::json!({})),
                )
                .await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/checklist" => {
                let (client, ws, task, _second_id) =
                    provision_task(&server, &db, "sweep-checklist").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/checklist",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateChecklistItemRequest {
                        title: "Sweep Item".to_string(),
                        before: None,
                        after: None,
                    })
                    .expect("serialize CreateChecklistItemRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote" => {
                let (client, ws, board_id, column_id) =
                    provision_board(&server, &db, "sweep-promoteitem").await;
                let path_create = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/boards/{}/tasks", ws.slug, board_id),
                );
                let task: atlas_api::dtos::boards_tasks::TaskDto = client
                    .create_task(
                        &ws.slug,
                        board_id,
                        CreateTaskRequest {
                            column_id,
                            title: "Sweep Parent Task".to_string(),
                            description: None,
                            properties: None,
                            before: None,
                            after: None,
                            references: Vec::new(),
                        },
                    )
                    .await
                    .expect("create parent task");
                let _ = &path_create;
                let item = client
                    .create_checklist_item(
                        &ws.slug,
                        &task.readable_id,
                        CreateChecklistItemRequest {
                            title: "Sweep Checklist Item".to_string(),
                            before: None,
                            after: None,
                        },
                    )
                    .await
                    .expect("create checklist item");
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/checklist/{}/promote",
                        ws.slug, task.readable_id, item.id
                    ),
                );
                let body = ReqBody::Json(
                    serde_json::json!({ "board_id": board_id, "column_id": column_id }),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/subtasks" => {
                let (client, ws, task, _second_id) =
                    provision_task(&server, &db, "sweep-subtask").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/subtasks",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(
                    serde_json::to_value(CreateSubtaskRequest {
                        title: "Sweep Subtask".to_string(),
                        column_id: None,
                        description: None,
                        properties: None,
                        before: None,
                        after: None,
                        references: Vec::new(),
                    })
                    .expect("serialize CreateSubtaskRequest"),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/promote" => {
                let (client, ws, board_id, column_id) =
                    provision_board(&server, &db, "sweep-promotesub").await;
                let parent = client
                    .create_task(
                        &ws.slug,
                        board_id,
                        CreateTaskRequest {
                            column_id,
                            title: "Sweep Parent".to_string(),
                            description: None,
                            properties: None,
                            before: None,
                            after: None,
                            references: Vec::new(),
                        },
                    )
                    .await
                    .expect("create parent task");
                let subtask = client
                    .create_subtask(
                        &ws.slug,
                        &parent.readable_id,
                        CreateSubtaskRequest {
                            title: "Sweep Subtask To Promote".to_string(),
                            column_id: None,
                            description: None,
                            properties: None,
                            before: None,
                            after: None,
                            references: Vec::new(),
                        },
                    )
                    .await
                    .expect("create subtask");
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/promote",
                        ws.slug, subtask.task.readable_id
                    ),
                );
                assert_idempotent_true(&client, &path, &[], ReqBody::Json(serde_json::json!({})))
                    .await;
            }
            "/workspaces/{ws}/tasks/{readable_id}/comments" => {
                let (client, ws, task, _second_id) =
                    provision_task(&server, &db, "sweep-taskcomment").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/tasks/{}/comments",
                        ws.slug, task.readable_id
                    ),
                );
                let body = ReqBody::Json(serde_json::json!({ "body": "Sweep comment" }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }

            // ---------------------------------------------------------------
            // acta: documents/folders
            // ---------------------------------------------------------------
            "/workspaces/{ws}/projects/{project_slug}/documents" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-doc").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/projects/{}/documents",
                        ws.slug, project.slug
                    ),
                );
                let body = ReqBody::Json(serde_json::json!({
                    "title": "Sweep Document",
                    "slug": "sweep-document",
                    "folder_id": null,
                    "content": "hello"
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/documents/{slug}/comment-drafts" => {
                let (client, ws, doc_slug) =
                    provision_document(&server, &db, "sweep-docdraft").await;
                let path = support::path::api_path(
                    "acta",
                    &format!(
                        "/workspaces/{}/documents/{}/comment-drafts",
                        ws.slug, doc_slug
                    ),
                );
                let headers = vec![("x-create-token", uuid::Uuid::now_v7().to_string())];
                assert_idempotent_true(
                    &client,
                    &path,
                    &headers,
                    ReqBody::Json(serde_json::json!({})),
                )
                .await;
            }
            "/workspaces/{ws}/documents/{slug}/copy" => {
                let (client, ws, doc_slug) =
                    provision_document(&server, &db, "sweep-doccopy").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/documents/{}/copy", ws.slug, doc_slug),
                );
                assert_idempotent_true(&client, &path, &[], ReqBody::Json(serde_json::json!({})))
                    .await;
            }
            "/workspaces/{ws}/documents/{slug}/comments" => {
                let (client, ws, doc_slug) =
                    provision_document(&server, &db, "sweep-doccomment").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/documents/{}/comments", ws.slug, doc_slug),
                );
                let body = ReqBody::Json(serde_json::json!({ "body": "Sweep comment" }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/projects/{project_slug}/folders" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-folder").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/projects/{}/folders", ws.slug, project.slug),
                );
                let body = ReqBody::Json(
                    serde_json::json!({ "name": "Sweep Folder", "parent_folder_id": null }),
                );
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/folders/{folder_id}/copy" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-foldercopy").await;
                let project = client
                    .create_project(
                        &ws.slug,
                        CreateProjectRequest {
                            name: "Sweep Project".to_string(),
                            slug: "sweep-proj".to_string(),
                            task_prefix: "SWP".to_string(),
                            visibility: None,
                            visibility_role: None,
                        },
                    )
                    .await
                    .expect("create project");
                let folder = client
                    .create_folder(
                        &ws.slug,
                        &project.slug,
                        atlas_api::dtos::folders::CreateFolderRequest {
                            name: "Sweep Folder To Copy".to_string(),
                            parent_folder_id: None,
                        },
                    )
                    .await
                    .expect("create folder");
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/folders/{}/copy", ws.slug, folder.id),
                );
                let body = ReqBody::Json(serde_json::json!({ "parent_folder_id": null }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }

            // ---------------------------------------------------------------
            // acta: automation-rules / search
            // ---------------------------------------------------------------
            "/workspaces/{ws}/automation-rules" => {
                let (client, ws, board_id, column_id) =
                    provision_board(&server, &db, "sweep-automation").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/automation-rules", ws.slug),
                );
                let body = ReqBody::Json(serde_json::json!({
                    "name": "Sweep Rule",
                    "trigger_event_type": "external.github.workflow_run",
                    "trigger_filter": null,
                    "project_id": null,
                    "action_type": "create_task",
                    "action_params": {
                        "board_id": board_id,
                        "column_id": column_id,
                        "title_template": "Sweep {event}"
                    }
                }));
                assert_idempotent_true(&client, &path, &[], body).await;
            }
            "/workspaces/{ws}/semantic-search/reindex" => {
                let (client, ws, _user) =
                    login_user_with_workspace(&server, &db, "sweep-reindex").await;
                let path = support::path::api_path(
                    "acta",
                    &format!("/workspaces/{}/semantic-search/reindex", ws.slug),
                );
                assert_idempotent_true(&client, &path, &[], ReqBody::Json(serde_json::json!({})))
                    .await;
            }

            unknown => panic!(
                "declared idempotent:true route {unknown} has no provisioning arm in this \
                 sweep — add one, do not skip it (INV-DATA-DRIVEN)"
            ),
        }

        covered.insert(route.clone());
    }

    let declared_set: HashSet<Route> = true_routes.iter().cloned().collect();
    let missing: Vec<&Route> = declared_set.difference(&covered).collect();
    let extra: Vec<&Route> = covered.difference(&declared_set).collect();
    assert!(
        missing.is_empty(),
        "declared idempotent:true routes never covered by an arm: {missing:?}"
    );
    assert!(
        extra.is_empty(),
        "covered routes not in the declared idempotent:true set (should be impossible): {extra:?}"
    );
}

/// T4.19: the self-checking declared-false sample. Isolated in its own test
/// (not sharing state with the true-route sweep above) so a failure here
/// never masks or is masked by the exhaustive true-route sweep.
#[tokio::test]
async fn declared_idempotent_false_sample_ignores_the_header() {
    let false_set = declared_false_set();
    for &(method, path, reason) in DECLARED_FALSE_SAMPLE {
        assert!(
            false_set.contains(&Route {
                method,
                path: path.to_string(),
            }),
            "{method:?} {path} is not in the declared idempotent:false set — \
             DECLARED_FALSE_SAMPLE has drifted from reg5.rs (reason claimed: {reason})"
        );
    }

    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    // apply_status_templates / archive_board / unarchive_board / heartbeat
    // (share one board)
    {
        let (client, ws, board_id, _column_id) =
            provision_board(&server, &db, "sweep-false-board").await;
        let self_id = client
            .me()
            .await
            .expect("fetch self")
            .id
            .expect("authenticated human user has an id");

        let apply_templates_path = support::path::api_path(
            "acta",
            &format!(
                "/workspaces/{}/boards/{}/apply-status-templates",
                ws.slug, board_id
            ),
        );
        assert_idempotent_false(
            &db,
            &client,
            self_id,
            &apply_templates_path,
            ReqBody::Json(serde_json::json!({})),
        )
        .await;

        let archive_path = support::path::api_path(
            "acta",
            &format!("/workspaces/{}/boards/{}/archive", ws.slug, board_id),
        );
        assert_idempotent_false(
            &db,
            &client,
            self_id,
            &archive_path,
            ReqBody::Json(serde_json::json!({})),
        )
        .await;

        let unarchive_path = support::path::api_path(
            "acta",
            &format!("/workspaces/{}/boards/{}/unarchive", ws.slug, board_id),
        );
        assert_idempotent_false(
            &db,
            &client,
            self_id,
            &unarchive_path,
            ReqBody::Json(serde_json::json!({})),
        )
        .await;

        let heartbeat_path = support::path::api_path(
            "acta",
            &format!("/workspaces/{}/boards/{}/presence", ws.slug, board_id),
        );
        assert_idempotent_false(
            &db,
            &client,
            self_id,
            &heartbeat_path,
            ReqBody::Json(serde_json::json!({})),
        )
        .await;
    }

    // search_content
    {
        let (client, ws, doc_slug) = provision_document(&server, &db, "sweep-false-search").await;
        let self_id = client
            .me()
            .await
            .expect("fetch self")
            .id
            .expect("authenticated human user has an id");
        let path = support::path::api_path(
            "acta",
            &format!(
                "/workspaces/{}/documents/{}/content/search",
                ws.slug, doc_slug
            ),
        );
        let body = ReqBody::Json(serde_json::json!({ "query": "hello" }));
        assert_idempotent_false(&db, &client, self_id, &path, body).await;
    }

    // move_documents_batch: hand-declared in acta.rs's `layered` module,
    // outside the macro's `[idempotent]` grammar. Moves a document to its
    // own current folder (root, `folder_id: null`) so both requests succeed
    // and converge to the same end state — the app-level idempotence D8's
    // rule relies on for a `false` classification.
    {
        let (client, ws, doc_slug) =
            provision_document(&server, &db, "sweep-false-movebatch").await;
        let self_id = client
            .me()
            .await
            .expect("fetch self")
            .id
            .expect("authenticated human user has an id");
        let path = support::path::api_path(
            "acta",
            &format!("/workspaces/{}/documents/moves/batch", ws.slug),
        );
        let body = ReqBody::Json(serde_json::json!({
            "moves": [{ "source_document": doc_slug, "folder_id": null }]
        }));
        assert_idempotent_false(&db, &client, self_id, &path, body).await;
    }
}

// ---------------------------------------------------------------------------
// Shared provisioning helpers
// ---------------------------------------------------------------------------

/// Creates a fresh workspace/owner session, a project, a board, and one
/// column — the minimal parent chain every board/column/task-family route
/// needs.
async fn provision_board(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    AtlasClient,
    atlas_acta::entities::identity::Workspace,
    uuid::Uuid,
    uuid::Uuid,
) {
    let (client, ws, user) = login_user_with_workspace(server, db, username).await;
    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Sweep Project".to_string(),
                slug: "sweep-proj".to_string(),
                task_prefix: "SWP".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let _ = &user;
    let board = client
        .create_board(
            &ws.slug,
            &project.slug,
            CreateBoardRequest {
                name: "Sweep Board".to_string(),
                folder_id: None,
            },
        )
        .await
        .expect("create board");
    let column = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                color: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create column");
    (client, ws, board.id, column.id)
}

/// `provision_board` plus one task — the minimal parent chain every
/// task-body-family route (comments, checklist, subtasks, attachments)
/// needs. The `uuid::Uuid` in the return tuple is the session user's own id,
/// reused by `add_assignee`'s arm.
async fn provision_task(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    AtlasClient,
    atlas_acta::entities::identity::Workspace,
    atlas_api::dtos::boards_tasks::TaskDto,
    uuid::Uuid,
) {
    let (client, ws, board_id, column_id) = provision_board(server, db, username).await;
    let task = client
        .create_task(
            &ws.slug,
            board_id,
            CreateTaskRequest {
                column_id,
                title: "Sweep Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
                references: Vec::new(),
            },
        )
        .await
        .expect("create task");
    let self_id = client
        .me()
        .await
        .expect("fetch self")
        .id
        .expect("authenticated human user has an id");
    (client, ws, task, self_id)
}

/// `provision_task` plus a SECOND task on the same board — the minimal
/// parent chain `create_reference`/`create_references_batch` need (a task
/// to reference).
async fn provision_task_pair(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    AtlasClient,
    atlas_acta::entities::identity::Workspace,
    atlas_api::dtos::boards_tasks::TaskDto,
    atlas_api::dtos::boards_tasks::TaskDto,
) {
    let (client, ws, board_id, column_id) = provision_board(server, db, username).await;
    let task = client
        .create_task(
            &ws.slug,
            board_id,
            CreateTaskRequest {
                column_id,
                title: "Sweep Source Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
                references: Vec::new(),
            },
        )
        .await
        .expect("create source task");
    let second_task = client
        .create_task(
            &ws.slug,
            board_id,
            CreateTaskRequest {
                column_id,
                title: "Sweep Target Task".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
                references: Vec::new(),
            },
        )
        .await
        .expect("create target task");
    (client, ws, task, second_task)
}

/// Creates a fresh workspace/owner session, a project, and one document —
/// the minimal parent chain every document-body-family route needs.
async fn provision_document(
    server: &TestServer,
    db: &TestDb,
    username: &str,
) -> (
    AtlasClient,
    atlas_acta::entities::identity::Workspace,
    String,
) {
    let (client, ws, _user) = login_user_with_workspace(server, db, username).await;
    let project = client
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Sweep Project".to_string(),
                slug: "sweep-proj".to_string(),
                task_prefix: "SWP".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");
    let document = client
        .create_document(
            &ws.slug,
            &project.slug,
            atlas_api::dtos::documents::CreateDocumentRequest {
                title: "Sweep Document".to_string(),
                folder_id: None,
                content: Some("hello".to_string()),
            },
        )
        .await
        .expect("create document");
    (
        client,
        ws,
        document.slug.expect("created document must have a slug"),
    )
}

/// Creates and activates a second, independent platform user directly
/// against the database (mirrors `support::seed_workspace`'s own
/// construction), WITHOUT adding them to any workspace — the minimal
/// "another existing principal" every grant/group-member/`add_member` arm
/// needs. `add_member`'s own arm is exactly what performs the FIRST
/// workspace-membership add for this user (via the raw idempotent call
/// under test), so this helper must never call `add_member` itself, or that
/// arm's own "first request succeeds" step would instead observe an
/// already-a-member domain conflict.
async fn second_platform_user(db: &TestDb, username: &str) -> uuid::Uuid {
    use atlas_server::persistence::repos::{NewUser, UserRepo};

    let user_repo = db.user_repo();
    let created = user_repo
        .create(NewUser {
            username: format!("{username}-{}", uuid::Uuid::now_v7().as_simple()),
            display_name: username.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create second platform user");
    activate_user_in_db(db, created.id.0).await;

    created.id.0
}
