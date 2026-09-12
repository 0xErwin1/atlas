#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! Container-backed proof for `GET /api/v2/custos/discover` (E11-S8 design
//! D4/D5, spec `custos-discovery`).
//!
//! Runs the real handler against a live Postgres so `PgDiscoveryRepo`'s
//! reverse-grant query and `PgWorkspaceRepo::list_for_user`'s
//! membership-derived union both execute for real, unlike the port-level
//! unit tests in `atlas_custos`/`atlas_custos_postgres`, which fake or
//! isolate one side of the union.
//!
//! **SH5 in two fixture shapes (D7, design D-S8-8)**: an api-key principal
//! (the literal wording) and a user principal with the identical grant and
//! no membership. Both are proven here, including the user-shape divergence
//! against Acta's own workspace route (E11-S8 PR4): `discover` lists the
//! grant-only workspace and project, but `GET
//! /api/v2/acta/workspaces/{ws}` still answers 404 for that same user,
//! because Acta's own `Authorized` extractor gates on membership
//! (`authorized.rs:1087-1089`), not on a Custos grant. This is a named,
//! shipped divergence handed to the epic closeout (SHELL-NAV-3), not a bug
//! this slice fixes.

mod support;

use atlas_acta::ids::ProjectId;
use atlas_custos::WorkspaceScope;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo;
use atlas_server::authz::policy::NewPermissionGrant;
use atlas_server::authz::{PermissionGrantRepo, ResourceRole};
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey};
use uuid::Uuid;

/// Unauthenticated `GET /api/v2/custos/discover` must answer 401 (spec
/// scenario "Unauthenticated request is rejected").
#[tokio::test]
async fn anonymous_request_is_rejected_with_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let response = http
        .get(format!("{}/api/v2/custos/discover", server.base_url()))
        .send()
        .await
        .expect("send request");

    assert_eq!(response.status(), 401);

    db.teardown().await;
}

/// A user with workspace membership only (no grant) sees that workspace
/// through the membership-derived source (D-S8-9, spec "Grant-derived and
/// membership-derived sources are both present" — the membership half).
#[tokio::test]
async fn membership_only_user_discovers_the_workspace_via_membership() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "discover-member-only").await;

    let response = client.custos().discover().await.expect("discover");

    assert!(!response.admin, "membership-only user must not be admin");
    assert!(!response.truncated);

    let acta = response
        .components
        .iter()
        .find(|c| c.component == "acta")
        .expect("acta component must be present for a workspace member");

    let expected_ref = atlas_acta::permissions::resource_ref_codec::to_core(
        &atlas_acta::permissions::ResourceRef::Workspace,
        ws.id,
    )
    .to_string();
    assert_eq!(
        acta.scopes,
        vec![expected_ref],
        "membership-derived discover must list exactly the member's workspace"
    );

    assert!(
        response.components.iter().all(|c| c.component != "custos"),
        "a non-admin principal with no custos grant must carry no custos entry \
         (INV-ABSENT-NOT-EMPTY)"
    );

    db.teardown().await;
}

/// Spec "Repeated calls return the same answer": discover reads state and
/// writes nothing, so two calls by the same principal serialize identically.
#[tokio::test]
async fn repeated_calls_return_the_same_answer() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client, _ws, _user) =
        support::login_user_with_workspace(&server, &db, "discover-repeat").await;

    let first = client.custos().discover().await.expect("first discover");
    let second = client.custos().discover().await.expect("second discover");

    assert_eq!(
        serde_json::to_value(&first).expect("first serializes"),
        serde_json::to_value(&second).expect("second serializes"),
        "a second discover call must answer exactly like the first"
    );

    db.teardown().await;
}

/// SH5, api-key shape (D-S8-8, literal wording): an api-key principal with a
/// grant only on a project sees exactly that project through the
/// grant-derived source, no membership, `admin: false`.
#[tokio::test]
async fn api_key_with_a_grant_only_discovers_exactly_that_grant() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "discover-apikey-owner").await;

    let plain = "atlas_discover_apikey_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let key = db
        .api_key_repo()
        .create_for_user(
            owner_user.id,
            NewApiKey {
                name: "discover-agent-key".to_string(),
                token_hash: hash,
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: Vec::new(),
            },
        )
        .await
        .expect("create api key");

    let project_id = ProjectId(Uuid::now_v7());
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: WorkspaceScope(ws.id.0),
            user_id: None,
            api_key_id: Some(key.id),
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Project(project_id),
                ws.id,
            ),
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed api-key grant");

    let agent_client =
        atlas_client::AtlasClient::new(server.base_url()).with_token(plain.to_string());

    let response = agent_client.custos().discover().await.expect("discover");

    assert!(!response.admin, "an api-key principal is never admin");
    assert!(!response.truncated);

    let acta = response
        .components
        .iter()
        .find(|c| c.component == "acta")
        .expect("acta component must be present for the granted api key");

    let expected_ref = atlas_acta::permissions::resource_ref_codec::to_core(
        &atlas_acta::permissions::ResourceRef::Project(project_id),
        ws.id,
    )
    .to_string();
    assert_eq!(
        acta.scopes,
        vec![expected_ref],
        "SH5: an api-key grant-only principal must see exactly its granted project, nothing else \
         (no workspace, no membership — api keys have no membership ceiling, design D4 R5)"
    );

    db.teardown().await;
}

/// SH5, user shape (D-S8-8): a user with a grant on a workspace and a project
/// but no membership row sees both refs through the grant-derived source
/// (D-S8-1), identically to the api-key shape above — but unlike the api-key
/// shape, this principal is a `User`, so the divergence against Acta's own
/// membership-gated workspace route is observable and asserted here.
#[tokio::test]
async fn user_with_a_grant_only_diverges_from_actas_membership_gated_workspace_route() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (_owner_client, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "discover-user-grant-owner").await;
    let (grantee_client, grantee_user) =
        support::login_user(&server, &db, "discover-user-grant-only").await;

    let project_id = ProjectId(Uuid::now_v7());
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    let workspace_ref = atlas_acta::permissions::resource_ref_codec::to_core(
        &atlas_acta::permissions::ResourceRef::Workspace,
        ws.id,
    );
    let project_ref = atlas_acta::permissions::resource_ref_codec::to_core(
        &atlas_acta::permissions::ResourceRef::Project(project_id),
        ws.id,
    );
    for resource_ref in [workspace_ref.clone(), project_ref.clone()] {
        grant_repo
            .upsert(NewPermissionGrant {
                workspace_id: WorkspaceScope(ws.id.0),
                user_id: Some(grantee_user.id),
                api_key_id: None,
                group_id: None,
                resource_ref,
                role: ResourceRole::Editor,
                created_by_user_id: Some(owner_user.id),
                created_by_api_key_id: None,
            })
            .await
            .expect("seed user grant");
    }

    let response = grantee_client.custos().discover().await.expect("discover");

    assert!(!response.admin, "a grant-only user is not admin");

    let acta = response
        .components
        .iter()
        .find(|c| c.component == "acta")
        .expect("acta component must be present for the granted user");
    let mut scopes = acta.scopes.clone();
    scopes.sort();
    let mut expected = vec![workspace_ref.to_string(), project_ref.to_string()];
    expected.sort();
    assert_eq!(
        scopes, expected,
        "SH5 (user shape): discover must list exactly the granted workspace and project, \
         through the grant-derived source alone (no membership row exists for this user)"
    );

    let workspace_lookup_error = grantee_client
        .acta()
        .get_workspace(&ws.slug)
        .await
        .expect_err("a grant-only user with no membership must not reach the Acta workspace route");
    assert!(
        matches!(workspace_lookup_error, atlas_client::ClientError::Api(ref p) if p.status == 404),
        "named divergence (D-S8-8): discover lists the workspace, but Acta's own membership-gated \
         route still 404s for the same grant-only user — {workspace_lookup_error:?}"
    );

    db.teardown().await;
}

/// Root/system-admin principals short-circuit to `admin: true` and every
/// present registry component, reading zero grant rows (INV-ADMIN-FROM-FLAGS,
/// spec "Admin flag short-circuits grant evaluation").
#[tokio::test]
async fn root_principal_short_circuits_to_admin_with_every_present_component() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;

    let response = root.custos().discover().await.expect("discover");

    assert!(response.admin, "root must short-circuit to admin: true");
    assert!(
        !response.truncated,
        "the admin short-circuit reads zero grant rows, so truncated must stay false"
    );
    assert!(
        response.components.iter().any(|c| c.component == "custos"),
        "the admin short-circuit must list every present registry component, including custos, \
         which never appears for a non-admin principal with no custos grant"
    );
    assert!(
        response.components.iter().all(|c| c.scopes.is_empty()),
        "the admin short-circuit lists components with empty scopes — admin reach is carried by \
         the flag, not enumerated scopes"
    );

    db.teardown().await;
}

/// `GrantedScopes`'s 500-scope cap (`MAX_DISCOVERY_SCOPES`, PR1) surfaces
/// through the wire response's `truncated` flag when a principal's grants
/// exceed it.
#[tokio::test]
async fn truncated_flag_surfaces_when_grants_exceed_the_cap() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "discover-truncated").await;

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    // One grant row per fabricated project id — the discovery port never
    // joins back to a real Acta project row, so 501 distinct resource refs
    // is enough to cross MAX_DISCOVERY_SCOPES (500) without seeding 501 real
    // projects.
    for _ in 0..501 {
        let project_id = ProjectId(Uuid::now_v7());
        grant_repo
            .upsert(NewPermissionGrant {
                workspace_id: WorkspaceScope(ws.id.0),
                user_id: Some(user.id),
                api_key_id: None,
                group_id: None,
                resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                    &atlas_acta::permissions::ResourceRef::Project(project_id),
                    ws.id,
                ),
                role: ResourceRole::Editor,
                created_by_user_id: Some(user.id),
                created_by_api_key_id: None,
            })
            .await
            .expect("seed over-cap grant");
    }

    let response = client.custos().discover().await.expect("discover");

    assert!(
        response.truncated,
        "over-cap grants must surface truncated: true"
    );

    let acta = response
        .components
        .iter()
        .find(|c| c.component == "acta")
        .expect("acta component must be present");
    assert_eq!(
        acta.scopes.len(),
        500,
        "the wire response must cap at MAX_DISCOVERY_SCOPES total, matching PR1's \
         insert_enforces_the_cap unit proof"
    );

    db.teardown().await;
}
