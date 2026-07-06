#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    AdminUpdateWorkspaceRequest, CreateGrantRequest, GrantPrincipal, UpdateWorkspaceRequest,
};
use atlas_client::ClientError;
use atlas_domain::{Actor, WorkspaceCtx, entities::permissions::NewPermissionGrant};
use atlas_server::persistence::repos::{
    ApiKeyRepo, NewApiKey, PermissionGrantRepo, PgPermissionGrantRepo, UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};

#[tokio::test]
async fn list_workspaces_returns_the_users_own_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _user) = login_user_with_workspace(&server, &db, "ws-list-owner").await;

    let workspaces = client.list_workspaces().await.expect("list_workspaces");

    assert!(
        workspaces.iter().any(|w| w.slug == ws.slug),
        "the seeded workspace slug '{}' must appear in the list",
        ws.slug,
    );
}

#[tokio::test]
async fn list_workspaces_does_not_leak_other_tenants_workspace() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_client_a, ws_a, _user_a) =
        login_user_with_workspace(&server, &db, "ws-list-tenant-a").await;
    let (client_b, _ws_b, _user_b) =
        login_user_with_workspace(&server, &db, "ws-list-tenant-b").await;

    let workspaces_b = client_b
        .list_workspaces()
        .await
        .expect("list_workspaces for tenant-b");

    assert!(
        !workspaces_b.iter().any(|w| w.slug == ws_a.slug),
        "tenant-b must not see tenant-a's workspace '{}'",
        ws_a.slug,
    );
}

#[tokio::test]
async fn list_workspaces_returns_401_for_unauthenticated() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let err = server
        .client()
        .list_workspaces()
        .await
        .expect_err("unauthenticated list_workspaces must fail");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 401, "expected 401, got {}", p.status)
        }
        other => panic!("unexpected error: {other:?}"),
    }
}

/// A freshly created workspace is seeded with a default project, the default
/// status templates, and a default board whose columns mirror those templates,
/// so Tasks and Notes are usable immediately instead of presenting an empty,
/// broken-looking workspace.
#[tokio::test]
async fn create_workspace_seeds_a_default_project_and_board() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, _ws, _user) = login_user_with_workspace(&server, &db, "ws-seed-owner").await;

    let created = client
        .create_workspace("Seed Target")
        .await
        .expect("create_workspace");

    let projects = client
        .list_projects(&created.slug, None, None)
        .await
        .expect("list_projects");
    assert_eq!(
        projects.items.len(),
        1,
        "a new workspace must have exactly one seeded project"
    );
    let project = &projects.items[0];
    assert_eq!(project.slug, "general");

    let boards = client
        .list_boards(&created.slug, &project.slug, None, None)
        .await
        .expect("list_boards");
    assert_eq!(
        boards.items.len(),
        1,
        "a new workspace must have exactly one seeded board"
    );
    let board = &boards.items[0];

    let columns = client
        .list_columns(&created.slug, board.id)
        .await
        .expect("list_columns");
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        names,
        vec!["To Do", "In Progress", "Done"],
        "default board columns derive from the seeded status templates, in order"
    );
}

// ---- B3: workspace rename ----

#[tokio::test]
async fn rename_workspace_member_can_rename() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _user) = login_user_with_workspace(&server, &db, "ws-rename-member").await;

    let updated = client
        .update_workspace(
            &ws.slug,
            UpdateWorkspaceRequest {
                name: "Renamed Workspace".to_string(),
            },
        )
        .await
        .expect("update_workspace");

    assert_eq!(updated.name, "Renamed Workspace", "name must be updated");
    assert_eq!(updated.slug, ws.slug, "slug must not change");

    db.teardown().await;
}

#[tokio::test]
async fn rename_workspace_persists() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client, ws, _user) = login_user_with_workspace(&server, &db, "ws-rename-persist").await;

    client
        .update_workspace(
            &ws.slug,
            UpdateWorkspaceRequest {
                name: "Persisted Name".to_string(),
            },
        )
        .await
        .expect("update_workspace");

    let fetched = client
        .get_workspace(&ws.slug)
        .await
        .expect("get_workspace after rename");

    assert_eq!(fetched.name, "Persisted Name");
    assert_eq!(fetched.slug, ws.slug);

    db.teardown().await;
}

#[tokio::test]
async fn rename_workspace_non_member_gets_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner_client, ws, _owner) =
        login_user_with_workspace(&server, &db, "ws-rename-owner").await;
    let (non_member, _other_ws, _other_user) =
        login_user_with_workspace(&server, &db, "ws-rename-nonmember").await;

    let err = non_member
        .update_workspace(
            &ws.slug,
            UpdateWorkspaceRequest {
                name: "Hijacked Name".to_string(),
            },
        )
        .await
        .expect_err("non-member must be denied");

    match err {
        ClientError::Api(p) => {
            assert_eq!(
                p.status, 404,
                "expected 404 (concealment), got {}",
                p.status
            )
        }
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

/// An agent (API key) that is a workspace member but lacks `config:update` is
/// denied the rename with a scope-403, while the same route stays open to human
/// members — the capability gate fires only for the API-key principal.
#[tokio::test]
async fn rename_workspace_agent_without_config_update_gets_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "ws-rename-agent-deny").await;

    let plain = "atlas_ws_rename_deny_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "ws-rename-deny".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: vec![atlas_domain::permissions::Capability {
                    family: atlas_domain::permissions::CapabilityFamily::Tasks,
                    action: atlas_domain::permissions::CapabilityAction::Read,
                }],
            },
        )
        .await
        .expect("create agent key without config:update");

    owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key.id.0,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to agent");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);

    let err = agent
        .update_workspace(
            &ws.slug,
            UpdateWorkspaceRequest {
                name: "Agent Rename Attempt".to_string(),
            },
        )
        .await
        .expect_err("an agent without config:update must be denied the rename");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 403, "expected 403, got {}", p.status);
            assert!(
                p.detail
                    .as_deref()
                    .unwrap_or("")
                    .contains("lacks required scope"),
                "expected a scope-denial detail, got {:?}",
                p.detail
            );
        }
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

/// An agent (API key) that is a workspace member AND holds `config:update`
/// renames the workspace successfully, exactly like a human member.
#[tokio::test]
async fn rename_workspace_agent_with_config_update_succeeds() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "ws-rename-agent-allow").await;

    let plain = "atlas_ws_rename_allow_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "ws-rename-allow".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: vec![atlas_domain::permissions::Capability {
                    family: atlas_domain::permissions::CapabilityFamily::Config,
                    action: atlas_domain::permissions::CapabilityAction::Update,
                }],
            },
        )
        .await
        .expect("create agent key with config:update");

    owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key.id.0,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to agent");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);

    let updated = agent
        .update_workspace(
            &ws.slug,
            UpdateWorkspaceRequest {
                name: "Agent Renamed Workspace".to_string(),
            },
        )
        .await
        .expect("an agent holding config:update must rename the workspace");

    assert_eq!(
        updated.name, "Agent Renamed Workspace",
        "name must be updated"
    );
    assert_eq!(updated.slug, ws.slug, "slug must not change");

    db.teardown().await;
}

// ---- B4: root workspace list ----

#[tokio::test]
async fn admin_list_workspaces_root_sees_all() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_client_a, ws_a, _user_a) =
        login_user_with_workspace(&server, &db, "ws-admin-list-a").await;
    let (_client_b, ws_b, _user_b) =
        login_user_with_workspace(&server, &db, "ws-admin-list-b").await;

    let root = support::login_root_user(&server, &db).await;

    let all = root
        .admin_list_workspaces()
        .await
        .expect("admin_list_workspaces");

    assert!(
        all.iter().any(|w| w.slug == ws_a.slug),
        "root must see ws_a"
    );
    assert!(
        all.iter().any(|w| w.slug == ws_b.slug),
        "root must see ws_b"
    );

    db.teardown().await;
}

#[tokio::test]
async fn admin_list_workspaces_non_root_gets_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (non_root, _ws, _user) =
        login_user_with_workspace(&server, &db, "ws-admin-list-nonroot").await;

    let err = non_root
        .admin_list_workspaces()
        .await
        .expect_err("non-root must be denied");

    match err {
        ClientError::Api(p) => {
            assert_eq!(p.status, 403, "expected 403, got {}", p.status)
        }
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- admin workspace re-slug ----

#[tokio::test]
async fn admin_update_workspace_changes_slug() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _user) = login_user_with_workspace(&server, &db, "ws-reslug-target").await;

    let root = support::login_root_user(&server, &db).await;

    let updated = root
        .admin_update_workspace(
            &ws.slug,
            AdminUpdateWorkspaceRequest {
                name: None,
                slug: Some("brand-new-slug".to_string()),
            },
        )
        .await
        .expect("admin_update_workspace");

    assert_eq!(updated.slug, "brand-new-slug", "slug must change");
    assert_eq!(updated.id, ws.id.0, "id must be stable across a re-slug");

    let fetched = owner
        .get_workspace("brand-new-slug")
        .await
        .expect("get_workspace by the new slug");
    assert_eq!(fetched.id, ws.id.0);

    let old = owner.get_workspace(&ws.slug).await;
    match old {
        Err(ClientError::Api(p)) => assert_eq!(p.status, 404, "old slug must 404"),
        other => panic!("old slug must no longer resolve, got {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn admin_update_workspace_rejects_taken_slug() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_a, ws_a, _ua) = login_user_with_workspace(&server, &db, "ws-reslug-collide-a").await;
    let (_b, ws_b, _ub) = login_user_with_workspace(&server, &db, "ws-reslug-collide-b").await;

    let root = support::login_root_user(&server, &db).await;

    let err = root
        .admin_update_workspace(
            &ws_a.slug,
            AdminUpdateWorkspaceRequest {
                name: None,
                slug: Some(ws_b.slug.clone()),
            },
        )
        .await
        .expect_err("re-slugging onto a taken slug must fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn admin_update_workspace_rejects_invalid_slug() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, _user) = login_user_with_workspace(&server, &db, "ws-reslug-invalid").await;

    let root = support::login_root_user(&server, &db).await;

    let err = root
        .admin_update_workspace(
            &ws.slug,
            AdminUpdateWorkspaceRequest {
                name: None,
                slug: Some("Not A Slug!".to_string()),
            },
        )
        .await
        .expect_err("an invalid slug format must be rejected");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 422, "expected 422, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn admin_update_workspace_non_root_gets_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _user) = login_user_with_workspace(&server, &db, "ws-reslug-nonroot").await;

    let err = owner
        .admin_update_workspace(
            &ws.slug,
            AdminUpdateWorkspaceRequest {
                name: None,
                slug: Some("owner-attempt".to_string()),
            },
        )
        .await
        .expect_err("a non-admin must not re-slug via the admin endpoint");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- admin workspace soft-delete ----

#[tokio::test]
async fn admin_delete_workspace_hides_it_everywhere() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _user) = login_user_with_workspace(&server, &db, "ws-delete-target").await;

    let root = support::login_root_user(&server, &db).await;

    root.admin_delete_workspace(&ws.slug)
        .await
        .expect("admin_delete_workspace");

    let admin_list = root
        .admin_list_workspaces()
        .await
        .expect("admin_list_workspaces after delete");
    assert!(
        !admin_list.iter().any(|w| w.slug == ws.slug),
        "a soft-deleted workspace must not appear in the admin list"
    );

    let owner_list = owner
        .list_workspaces()
        .await
        .expect("owner list_workspaces after delete");
    assert!(
        !owner_list.iter().any(|w| w.slug == ws.slug),
        "a soft-deleted workspace must not appear in the owner's list"
    );

    let get = owner.get_workspace(&ws.slug).await;
    match get {
        Err(ClientError::Api(p)) => assert_eq!(p.status, 404, "deleted workspace must 404"),
        other => panic!("deleted workspace must no longer resolve, got {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn admin_delete_workspace_unknown_slug_gets_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let root = support::login_root_user(&server, &db).await;

    let err = root
        .admin_delete_workspace("no-such-workspace")
        .await
        .expect_err("deleting an unknown workspace must fail");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 404, "expected 404, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

#[tokio::test]
async fn admin_delete_workspace_non_root_gets_403() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, _user) = login_user_with_workspace(&server, &db, "ws-delete-nonroot").await;

    let err = owner
        .admin_delete_workspace(&ws.slug)
        .await
        .expect_err("a non-admin must not delete via the admin endpoint");

    match err {
        ClientError::Api(p) => assert_eq!(p.status, 403, "expected 403, got {}", p.status),
        other => panic!("unexpected error: {other:?}"),
    }

    db.teardown().await;
}

// ---- api_key workspace listing ----

/// An api_key that holds a workspace-scope grant appears in `GET /api/workspaces`.
#[tokio::test]
async fn api_key_with_grant_sees_workspace_in_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "ws-ak-list-single").await;

    let plain = "atlas_ak_list_single_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "ak-list-single".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    owner
        .create_workspace_grant(
            &ws.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key.id.0,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace editor to key");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);

    let workspaces = agent
        .list_workspaces()
        .await
        .expect("api_key list_workspaces must succeed");

    assert!(
        workspaces.iter().any(|w| w.slug == ws.slug),
        "api_key with grant must see workspace '{}' in list_workspaces, got: {:?}",
        ws.slug,
        workspaces
            .iter()
            .map(|w| w.slug.as_str())
            .collect::<Vec<_>>(),
    );

    db.teardown().await;
}

/// An api_key granted in two distinct workspaces sees both in `GET /api/workspaces`.
/// Also verifies no duplication when the key has multiple grants in the same workspace.
#[tokio::test]
async fn api_key_with_grants_in_two_workspaces_sees_both_distinct() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (owner_a, ws_a, owner_a_user) =
        login_user_with_workspace(&server, &db, "ws-ak-list-two-a").await;
    let (owner_b, ws_b, owner_b_user) =
        login_user_with_workspace(&server, &db, "ws-ak-list-two-b").await;

    let plain = "atlas_ak_list_two_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let ctx_a = WorkspaceCtx::new(ws_a.id, Actor::User(owner_a_user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx_a,
            NewApiKey {
                name: "ak-list-two".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key");

    owner_a
        .create_workspace_grant(
            &ws_a.slug,
            CreateGrantRequest {
                principal: GrantPrincipal {
                    r#type: "api_key".to_string(),
                    id: key.id.0,
                },
                role: "editor".to_string(),
            },
        )
        .await
        .expect("grant workspace-a editor");

    // Grant in workspace B directly via the repo so we can use a different workspace owner
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_b.id,
            user_id: None,
            api_key_id: Some(key.id),
            group_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Viewer,
            created_by_user_id: Some(owner_b_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("grant workspace-b viewer via repo");

    // Add a second grant in workspace-a to verify no duplicates are returned
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws_a.id,
            user_id: None,
            api_key_id: Some(key.id),
            group_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Viewer,
            created_by_user_id: Some(owner_a_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("second grant in workspace-a (for dedup test)");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);

    let workspaces = agent
        .list_workspaces()
        .await
        .expect("api_key list_workspaces must succeed");

    assert!(
        workspaces.iter().any(|w| w.slug == ws_a.slug),
        "api_key must see workspace-a '{}' in list_workspaces",
        ws_a.slug
    );
    assert!(
        workspaces.iter().any(|w| w.slug == ws_b.slug),
        "api_key must see workspace-b '{}' in list_workspaces",
        ws_b.slug
    );

    let ws_a_count = workspaces.iter().filter(|w| w.slug == ws_a.slug).count();
    assert_eq!(
        ws_a_count, 1,
        "workspace-a must appear exactly once even with multiple grants (dedup), got {ws_a_count}"
    );

    let _ = owner_b;
    db.teardown().await;
}

/// An api_key with NO grant sees an empty list from `GET /api/workspaces`.
#[tokio::test]
async fn api_key_with_no_grant_sees_empty_workspace_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "ws-ak-list-empty").await;

    let plain = "atlas_ak_list_empty_secret";
    let hash = atlas_server::auth::tokens::hash_token(plain);
    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner_user.id));
    db.api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "ak-list-empty".to_string(),
                token_hash: hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create api key with no grant");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);

    let workspaces = agent
        .list_workspaces()
        .await
        .expect("api_key with no grant: list_workspaces must return 200");

    assert!(
        workspaces.is_empty(),
        "api_key with no grant must see empty list, got: {:?}",
        workspaces
            .iter()
            .map(|w| w.slug.as_str())
            .collect::<Vec<_>>(),
    );

    db.teardown().await;
}

/// Regression: a normal user still gets exactly their member workspaces after the fix.
#[tokio::test]
async fn list_workspaces_user_still_sees_own_workspaces_after_fix() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (client_a, ws_a, _user_a) =
        login_user_with_workspace(&server, &db, "ws-regression-a").await;
    let (_client_b, ws_b, _user_b) =
        login_user_with_workspace(&server, &db, "ws-regression-b").await;

    let workspaces = client_a
        .list_workspaces()
        .await
        .expect("user list_workspaces must succeed");

    assert!(
        workspaces.iter().any(|w| w.slug == ws_a.slug),
        "user must see their own workspace '{}' after the fix",
        ws_a.slug
    );
    assert!(
        !workspaces.iter().any(|w| w.slug == ws_b.slug),
        "user must NOT see another tenant's workspace '{}' after the fix",
        ws_b.slug
    );

    db.teardown().await;
}

/// A global key created by root lists EVERY workspace (its creator's reach), not
/// just the workspaces where it holds a grant — a global key needs no grants.
#[tokio::test]
async fn global_api_key_created_by_root_lists_all_workspaces() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    let (_a, ws_a, _ua) = login_user_with_workspace(&server, &db, "glob-ls-a").await;
    let (_b, ws_b, _ub) = login_user_with_workspace(&server, &db, "glob-ls-b").await;

    let root = support::login_root_user(&server, &db).await;
    let root_user = db
        .user_repo()
        .find_root()
        .await
        .expect("find_root")
        .expect("root user exists");

    let plain = "atlas_glob_ls_secret";
    let key = db
        .api_key_repo()
        .create_for_user(
            root_user.id,
            NewApiKey {
                name: "glob-ls-key".to_string(),
                token_hash: atlas_server::auth::tokens::hash_token(plain),
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
                scopes: atlas_domain::permissions::Capability::ALL.to_vec(),
            },
        )
        .await
        .expect("create root-owned key");

    root.set_api_key_global(key.id.0, true)
        .await
        .expect("root marks own key global");

    let agent = atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(plain);
    let workspaces = agent.list_workspaces().await.expect("list_workspaces");
    let slugs: Vec<String> = workspaces.iter().map(|w| w.slug.clone()).collect();

    assert!(
        slugs.iter().any(|s| s == &ws_a.slug),
        "global key must list ws_a; got {slugs:?}"
    );
    assert!(
        slugs.iter().any(|s| s == &ws_b.slug),
        "global key must list ws_b; got {slugs:?}"
    );

    db.teardown().await;
}
