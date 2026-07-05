#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{ApiKeyScope, CreateUserApiKeyRequest, InitialGrantRequest};
use support::{TestDb, TestServer, login_user, login_user_with_workspace};

fn key_req(name: &str) -> CreateUserApiKeyRequest {
    CreateUserApiKeyRequest {
        name: name.to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: None,
        scopes: None,
    }
}

fn user_key_req(name: &str) -> CreateUserApiKeyRequest {
    CreateUserApiKeyRequest {
        name: name.to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: None,
        scopes: None,
    }
}

// ---------------------------------------------------------------------------
// POST /v1/api-keys
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_user_api_key_returns_secret_and_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user1").await;

    let created = user
        .create_user_api_key(user_key_req("my-agent-key"))
        .await
        .expect("create user api key");

    assert!(
        created.secret.starts_with("atlas_"),
        "secret must have atlas_ prefix"
    );
    assert_eq!(created.name, "my-agent-key");
    assert_eq!(created.r#type, "agent", "default type must be agent");
    assert_ne!(created.id, uuid::Uuid::nil());

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_respects_explicit_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user2").await;

    let req = CreateUserApiKeyRequest {
        name: "cli-key".to_string(),
        r#type: Some("cli".to_string()),
        expires_at: None,
        initial_grant: None,
        scopes: None,
    };

    let created = user
        .create_user_api_key(req)
        .await
        .expect("create cli api key");

    assert_eq!(created.r#type, "cli");

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_rejects_invalid_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "cuk-user3").await;

    let req = CreateUserApiKeyRequest {
        name: "bad-type-key".to_string(),
        r#type: Some("superuser".to_string()),
        expires_at: None,
        initial_grant: None,
        scopes: None,
    };

    let err = user.create_user_api_key(req).await;

    assert!(err.is_err(), "invalid type must be rejected");

    db.teardown().await;
}

#[tokio::test]
async fn api_key_principal_cannot_create_user_api_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _ws, _) = login_user_with_workspace(&server, &db, "cuk-user4").await;

    let ws_key = owner
        .create_user_api_key(key_req("agent-key"))
        .await
        .expect("create agent key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(ws_key.secret);

    let err = agent.create_user_api_key(user_key_req("forbidden")).await;

    assert!(err.is_err(), "api key principal must get 403");

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_with_initial_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-user5").await;

    let req = CreateUserApiKeyRequest {
        name: "granted-key".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "editor".to_string(),
        }),
        scopes: None,
    };

    let created = owner
        .create_user_api_key(req)
        .await
        .expect("create key with initial grant");

    assert_eq!(created.name, "granted-key");

    let members = owner
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let key_member = members
        .iter()
        .find(|m| m.principal_type == "api_key" && m.id == created.id);

    assert!(
        key_member.is_some(),
        "key should appear as a workspace member after grant"
    );

    db.teardown().await;
}

#[tokio::test]
async fn initial_grant_rejects_admin_role() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-user6").await;

    let req = CreateUserApiKeyRequest {
        name: "admin-grant-key".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "admin".to_string(),
        }),
        scopes: None,
    };

    let err = owner.create_user_api_key(req).await;

    assert!(
        err.is_err(),
        "admin role in initial_grant must be rejected for api keys"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// GET /v1/api-keys
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_user_api_keys_returns_own_keys_only() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (alice, _) = login_user(&server, &db, "list-alice").await;
    let (bob, _) = login_user(&server, &db, "list-bob").await;

    alice
        .create_user_api_key(user_key_req("alice-key"))
        .await
        .expect("alice creates key");

    bob.create_user_api_key(user_key_req("bob-key"))
        .await
        .expect("bob creates key");

    let alice_page = alice
        .list_user_api_keys(None, None)
        .await
        .expect("alice lists her keys");

    assert_eq!(alice_page.items.len(), 1, "alice sees exactly 1 key");
    assert_eq!(alice_page.items[0].name, "alice-key");

    let names: Vec<&str> = alice_page.items.iter().map(|k| k.name.as_str()).collect();
    assert!(!names.contains(&"bob-key"), "alice must not see bob's key");

    db.teardown().await;
}

#[tokio::test]
async fn list_user_api_keys_includes_type_field() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "list-type").await;

    user.create_user_api_key(CreateUserApiKeyRequest {
        name: "typed-key".to_string(),
        r#type: Some("bot".to_string()),
        expires_at: None,
        initial_grant: None,
        scopes: None,
    })
    .await
    .expect("create typed key");

    let page = user
        .list_user_api_keys(None, None)
        .await
        .expect("list keys");

    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].r#type, "bot");

    db.teardown().await;
}

#[tokio::test]
async fn api_key_principal_cannot_list_user_api_keys() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _ws, _) = login_user_with_workspace(&server, &db, "list-agent").await;

    let ws_key = owner
        .create_user_api_key(key_req("list-agent-key"))
        .await
        .expect("create agent key");

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(ws_key.secret);

    let err = agent.list_user_api_keys(None, None).await;

    assert!(err.is_err(), "api key principal must get 403");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// DELETE /v1/api-keys/{id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_user_api_key_removes_from_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "revoke-user1").await;

    let created = user
        .create_user_api_key(user_key_req("to-revoke"))
        .await
        .expect("create key");

    let page_before = user
        .list_user_api_keys(None, None)
        .await
        .expect("list before revoke");

    assert!(
        page_before.items.iter().any(|k| k.id == created.id),
        "key must appear before revoke"
    );

    user.revoke_user_api_key(created.id)
        .await
        .expect("revoke key");

    let page_after = user
        .list_user_api_keys(None, None)
        .await
        .expect("list after revoke");

    assert!(
        !page_after.items.iter().any(|k| k.id == created.id),
        "revoked key must not appear after revoke"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_user_api_key_rejects_other_users_key() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (alice, _) = login_user(&server, &db, "revoke-alice").await;
    let (bob, _) = login_user(&server, &db, "revoke-bob").await;

    let alice_key = alice
        .create_user_api_key(user_key_req("alice-secret"))
        .await
        .expect("alice creates key");

    let err = bob.revoke_user_api_key(alice_key.id).await;

    assert!(err.is_err(), "bob must not be able to revoke alice's key");

    let page = alice
        .list_user_api_keys(None, None)
        .await
        .expect("list alice's keys");

    assert!(
        page.items.iter().any(|k| k.id == alice_key.id),
        "alice's key must still exist after bob's failed revoke"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_user_api_key_nonexistent_returns_error() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (user, _) = login_user(&server, &db, "revoke-notfound").await;

    let err = user.revoke_user_api_key(uuid::Uuid::now_v7()).await;

    assert!(err.is_err(), "revoking a nonexistent key must return error");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// GET /v1/api-keys/{id}/grants
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_api_key_grants_returns_workspace_and_project_grants() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grants-list-owner").await;

    let created = owner
        .create_user_api_key(user_key_req("grants-agent"))
        .await
        .expect("create key");

    owner
        .create_workspace_grant(
            &ws.slug,
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant key to workspace");

    owner
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Key Grants Proj".to_string(),
                slug: "key-grants-proj".to_string(),
                task_prefix: "KGP".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    owner
        .create_project_grant(
            &ws.slug,
            "key-grants-proj",
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "viewer".to_string(),
            },
        )
        .await
        .expect("grant key to project");

    let grants = owner
        .list_api_key_grants(created.id)
        .await
        .expect("list api key grants");

    assert_eq!(grants.len(), 2, "key must have 2 grants");

    let ws_grant = grants
        .iter()
        .find(|g| g.resource_kind == "workspace")
        .expect("workspace grant must be present");
    assert_eq!(ws_grant.role, "editor");
    assert_eq!(ws_grant.workspace_slug, ws.slug);
    assert!(ws_grant.project_slug.is_none());

    let proj_grant = grants
        .iter()
        .find(|g| g.resource_kind == "project")
        .expect("project grant must be present");
    assert_eq!(proj_grant.role, "viewer");
    assert_eq!(proj_grant.workspace_slug, ws.slug);
    assert_eq!(proj_grant.project_slug.as_deref(), Some("key-grants-proj"));

    db.teardown().await;
}

#[tokio::test]
async fn list_api_key_grants_includes_granted_by_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "grants-grantedby-owner").await;

    let created = owner
        .create_user_api_key(user_key_req("grants-grantedby-agent"))
        .await
        .expect("create key");

    owner
        .create_workspace_grant(
            &ws.slug,
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant key to workspace");

    let grants = owner
        .list_api_key_grants(created.id)
        .await
        .expect("list api key grants");

    assert_eq!(grants.len(), 1, "key must have 1 grant");

    let granted_by = grants[0]
        .granted_by
        .as_ref()
        .expect("granted_by must be present for a user-created grant");

    assert_eq!(
        granted_by.id, owner_user.id.0,
        "granter id must be the owner"
    );
    assert_eq!(
        granted_by.display, owner_user.display_name,
        "granter display must be the owner's display name"
    );
    assert_eq!(
        granted_by.principal_type, "user",
        "granter principal_type must be 'user'"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_api_key_grants_non_owner_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grants-nonowner-owner").await;
    let (other, _) = login_user(&server, &db, "grants-nonowner-other").await;

    let created = owner
        .create_user_api_key(user_key_req("grants-agent-nonowner"))
        .await
        .expect("create key");

    owner
        .create_workspace_grant(
            &ws.slug,
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant key");

    let err = other
        .list_api_key_grants(created.id)
        .await
        .expect_err("non-owner must be rejected");

    match err {
        atlas_client::ClientError::Api(p) => assert!(
            p.status == 403 || p.status == 404,
            "expected 403 or 404 for non-owner, got {}",
            p.status
        ),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// DELETE /v1/api-keys/{id}/grants/{grant_id}
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_api_key_grant_removes_grant() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grant-del-owner").await;

    let created = owner
        .create_user_api_key(user_key_req("grant-del-agent"))
        .await
        .expect("create key");

    owner
        .create_workspace_grant(
            &ws.slug,
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant key");

    let grants_before = owner
        .list_api_key_grants(created.id)
        .await
        .expect("list before delete");
    assert_eq!(grants_before.len(), 1);

    let grant_id = grants_before[0].id;

    owner
        .delete_api_key_grant(created.id, grant_id)
        .await
        .expect("delete grant");

    let grants_after = owner
        .list_api_key_grants(created.id)
        .await
        .expect("list after delete");
    assert!(grants_after.is_empty(), "grant must be gone after delete");

    db.teardown().await;
}

#[tokio::test]
async fn delete_api_key_grant_non_owner_returns_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "grant-del-nonown-owner").await;
    let (other, _) = login_user(&server, &db, "grant-del-nonown-other").await;

    let created = owner
        .create_user_api_key(user_key_req("grant-del-nonown-agent"))
        .await
        .expect("create key");

    owner
        .create_workspace_grant(
            &ws.slug,
            atlas_api::dtos::CreateGrantRequest {
                principal: atlas_api::dtos::GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: created.id,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant key");

    let grants = owner
        .list_api_key_grants(created.id)
        .await
        .expect("list grants");
    let grant_id = grants[0].id;

    let err = other
        .delete_api_key_grant(created.id, grant_id)
        .await
        .expect_err("non-owner must be rejected");

    match err {
        atlas_client::ClientError::Api(p) => assert!(
            p.status == 403 || p.status == 404,
            "expected 403 or 404 for non-owner, got {}",
            p.status
        ),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Attribution: key_type in member/principal DTOs
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_key_member_dto_includes_key_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "attr-user1").await;

    let req = CreateUserApiKeyRequest {
        name: "attr-bot".to_string(),
        r#type: Some("bot".to_string()),
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "editor".to_string(),
        }),
        scopes: None,
    };

    let key_created = owner
        .create_user_api_key(req)
        .await
        .expect("create key with grant");

    let members = owner
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let key_member = members
        .iter()
        .find(|m| m.principal_type == "api_key" && m.id == key_created.id)
        .expect("api key must appear as workspace member after grant");

    assert_eq!(
        key_member.key_type.as_deref(),
        Some("bot"),
        "key_type must be 'bot' in member DTO"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// Capability scopes: defaults, explicit selection, PATCH-partial, validation
// ---------------------------------------------------------------------------

const DEFAULT_READ_ONLY_SCOPES: [ApiKeyScope; 5] = [
    ApiKeyScope::TasksRead,
    ApiKeyScope::DocsRead,
    ApiKeyScope::BoardsRead,
    ApiKeyScope::FoldersRead,
    ApiKeyScope::ProjectsRead,
];

#[tokio::test]
async fn create_user_api_key_default_scopes_grant_read_but_not_write() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _) = login_user_with_workspace(&server, &db, "cuk-default-scopes").await;

    let req = CreateUserApiKeyRequest {
        name: "default-scope-agent".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: Some(InitialGrantRequest {
            workspace: ws.slug.clone(),
            role: "editor".to_string(),
        }),
        scopes: None,
    };

    let created = owner
        .create_user_api_key(req)
        .await
        .expect("create default-scope key");

    assert_eq!(
        created.scopes,
        DEFAULT_READ_ONLY_SCOPES.to_vec(),
        "a key created with no scopes selected must default to the 5 read-only scopes"
    );

    let page = owner
        .list_user_api_keys(None, None)
        .await
        .expect("list keys");
    let listed = page
        .items
        .iter()
        .find(|k| k.id == created.id)
        .expect("key must be listed");
    assert_eq!(
        listed.scopes,
        DEFAULT_READ_ONLY_SCOPES.to_vec(),
        "the persisted default scopes must round-trip through the read path"
    );

    let agent = atlas_client::AtlasClient::new(server.base_url()).with_token(created.secret);

    agent
        .list_projects(&ws.slug, None, None)
        .await
        .expect("read-only default must allow reading projects");

    let create_err = agent
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "should-fail".to_string(),
                slug: "should-fail".to_string(),
                task_prefix: "SFL".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect_err("read-only default must forbid creating a project");

    match create_err {
        atlas_client::ClientError::Api(p) => {
            assert_eq!(p.status, 403);
            assert!(
                p.detail
                    .as_deref()
                    .unwrap_or("")
                    .contains("lacks required scope"),
                "must be a scope denial, got: {p:?}"
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_explicit_scopes_round_trip() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _) = login_user(&server, &db, "cuk-explicit-scopes").await;

    let req = CreateUserApiKeyRequest {
        name: "explicit-scope-agent".to_string(),
        r#type: None,
        expires_at: None,
        initial_grant: None,
        scopes: Some(vec![
            ApiKeyScope::TasksUpdate,
            ApiKeyScope::TasksRead,
            ApiKeyScope::TasksRead, // duplicate on purpose: must be deduplicated
        ]),
    };

    let created = owner
        .create_user_api_key(req)
        .await
        .expect("create explicit-scope key");

    assert_eq!(
        created.scopes,
        vec![ApiKeyScope::TasksRead, ApiKeyScope::TasksUpdate],
        "explicit scopes must be deduplicated and canonically ordered"
    );

    db.teardown().await;
}

#[tokio::test]
async fn create_user_api_key_rejects_unknown_scope_with_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _) = login_user(&server, &db, "cuk-unknown-scope").await;

    let response = owner
        .http_client()
        .post(format!("{}/v1/api-keys", server.base_url()))
        .header("x-atlas-csrf", "1")
        .bearer_auth(owner.token().expect("token"))
        .json(&serde_json::json!({
            "name": "bad-scope-key",
            "scopes": ["tasks:manage"],
        }))
        .send()
        .await
        .expect("send request");

    assert_eq!(
        response.status().as_u16(),
        422,
        "an unknown scope string must be rejected with 422"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_user_api_key_replaces_full_scope_set() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _) = login_user(&server, &db, "cuk-update-scopes").await;

    let created = owner
        .create_user_api_key(user_key_req("update-scope-key"))
        .await
        .expect("create key");

    let updated = owner
        .set_api_key_scopes(
            created.id,
            vec![ApiKeyScope::DocsCreate, ApiKeyScope::DocsRead],
        )
        .await
        .expect("update scopes");

    assert_eq!(
        updated.scopes,
        vec![ApiKeyScope::DocsRead, ApiKeyScope::DocsCreate],
        "update must replace the full set, deduplicated and canonically ordered"
    );

    db.teardown().await;
}

#[tokio::test]
async fn update_user_api_key_empty_scopes_returns_400() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _) = login_user(&server, &db, "cuk-empty-scopes").await;

    let created = owner
        .create_user_api_key(user_key_req("empty-scope-key"))
        .await
        .expect("create key");

    let err = owner
        .set_api_key_scopes(created.id, vec![])
        .await
        .expect_err("empty scopes must be rejected");

    match err {
        atlas_client::ClientError::Api(p) => assert_eq!(
            p.status, 400,
            "an explicit empty scope list must be rejected with 400"
        ),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn update_user_api_key_omitting_scopes_leaves_them_unchanged() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, _) = login_user(&server, &db, "cuk-omit-scopes").await;

    let created = owner
        .create_user_api_key(user_key_req("omit-scope-key"))
        .await
        .expect("create key");

    let scopes_before = created.scopes.clone();

    let after_toggle = owner
        .set_api_key_global(created.id, true)
        .await
        .expect("toggle is_global without touching scopes");

    assert_eq!(
        after_toggle.scopes, scopes_before,
        "a PATCH that omits scopes must leave them unchanged"
    );
    assert!(after_toggle.is_global, "is_global must have been toggled");

    db.teardown().await;
}
