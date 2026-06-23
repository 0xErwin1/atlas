#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, CreateUserApiKeyRequest, GrantPrincipal,
    UpdateProjectRequest,
};
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, PermissionGrantRepo, UserRepo};

fn proj_req(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "PRM".to_string(),
        visibility: None,
        visibility_role: None,
    }
}

fn private_proj_req(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "PRM".to_string(),
        visibility: Some("private".to_string()),
        visibility_role: None,
    }
}

fn grant_req(user_id: uuid::Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: role.to_string(),
    }
}

/// Creates a user with the given membership role in `ws`, logs in via the server,
/// and returns (authenticated client, user).
async fn add_member(
    db: &support::TestDb,
    server: &support::TestServer,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
    role: MemberRole,
) -> (
    atlas_client::AtlasClient,
    atlas_domain::entities::identity::User,
) {
    use atlas_api::dtos::LoginRequest;
    use atlas_server::auth::password;

    let hash = password::hash("TestPassword1!".to_string())
        .await
        .expect("hash");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    support::activate_user_in_db(db, user.id.0).await;

    let ctx = WorkspaceCtx::new(ws_id, Actor::User(user.id));
    db.membership_repo()
        .add(&ctx, user.id, role)
        .await
        .expect("add membership");

    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");

    (client, user)
}

#[tokio::test]
async fn agent_cannot_share_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-agent-owner").await;

    let project = owner
        .create_project(&ws.slug, proj_req("Agent Project", "agent-proj"))
        .await
        .expect("create project");

    let key_created = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "test-agent-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create api key");

    let key_created_id = key_created.id;
    let agent_client =
        atlas_client::AtlasClient::new(server.base_url()).with_token(key_created.secret.clone());

    let (_, another_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-agent-grantee",
        MemberRole::Member,
    )
    .await;

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::{ApiKeyId, ProjectId};
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key_created_id)),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed agent grant on project");

    let result = agent_client
        .create_project_grant(
            &ws.slug,
            "agent-proj",
            grant_req(another_user.id.0, "viewer"),
        )
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "agent grant attempt must return 403, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn editor_cannot_grant_admin() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-editor-owner").await;

    owner
        .create_project(
            &ws.slug,
            proj_req("Editor Guardrail Project", "editor-guard-proj"),
        )
        .await
        .expect("create project");

    let (editor, editor_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-editor-member",
        MemberRole::Member,
    )
    .await;

    let (_, grantee_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-editor-grantee",
        MemberRole::Member,
    )
    .await;

    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    use atlas_domain::entities::permissions::NewPermissionGrant;
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(editor_user.id),
            api_key_id: None,
            project_id: Some(
                owner
                    .get_project(&ws.slug, "editor-guard-proj")
                    .await
                    .expect("get project")
                    .id
                    .into(),
            ),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed editor grant");

    let result = editor
        .create_project_grant(
            &ws.slug,
            "editor-guard-proj",
            grant_req(grantee_user.id.0, "admin"),
        )
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "editor granting admin must return 403, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn viewer_cannot_update_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-viewer-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            private_proj_req("Viewer Test Project", "viewer-test-proj"),
        )
        .await
        .expect("create project");

    let (viewer, viewer_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-viewer-member",
        MemberRole::Member,
    )
    .await;

    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    use atlas_domain::entities::permissions::NewPermissionGrant;
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(viewer_user.id),
            api_key_id: None,
            project_id: Some(project.id.into()),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Viewer,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed viewer grant");

    let result = viewer
        .update_project(
            &ws.slug,
            "viewer-test-proj",
            UpdateProjectRequest {
                name: Some("New Name".to_string()),
                visibility: None,
                visibility_role: None,
                task_prefix: None,
            },
        )
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "viewer updating project must return 403, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn non_member_cannot_access_workspace_resource() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "perm-nonmember-owner").await;

    owner
        .create_project(
            &ws.slug,
            proj_req("Non-member Test Project", "nonmember-proj"),
        )
        .await
        .expect("create project");

    let (non_member, _, _) =
        support::login_user_with_workspace(&server, &db, "perm-nonmember-outsider").await;

    let result = non_member.get_project(&ws.slug, "nonmember-proj").await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403 || p.status == 404),
        "non-member accessing workspace resource must return 403 or 404, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn agent_with_grant_sees_private_project_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-agentvis-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            private_proj_req("Agent Visible Private", "agent-visible-priv"),
        )
        .await
        .expect("create private project");

    let key_created = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "agent-visibility-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create api key");

    let key_created_id = key_created.id;
    let agent_client =
        atlas_client::AtlasClient::new(server.base_url()).with_token(key_created.secret.clone());

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::{ApiKeyId, ProjectId};
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key_created_id)),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Viewer,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed agent grant on private project");

    let page = agent_client
        .list_projects(&ws.slug, None, None)
        .await
        .expect("agent with grant must be able to list projects");

    let found = page.items.iter().any(|p| p.id == project.id);
    assert!(
        found,
        "agent with explicit grant must see the private project in the list"
    );

    db.teardown().await;
}

/// An API key with NO explicit grant must be denied at the workspace gate.
/// Grant-based access means visibility rules never apply to ungranted keys:
/// they receive a uniform 404 (concealment) on every workspace endpoint.
#[tokio::test]
async fn agent_without_grant_cannot_see_workspace_visibility_project() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "perm-agentnogrant-owner").await;

    owner
        .create_project(
            &ws.slug,
            CreateProjectRequest {
                name: "Workspace Visible Project".to_string(),
                slug: "ws-vis-proj".to_string(),
                task_prefix: "WVP".to_string(),
                visibility: Some("workspace".to_string()),
                visibility_role: Some("viewer".to_string()),
            },
        )
        .await
        .expect("create workspace-visibility project");

    let key_created = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "no-grant-agent-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create api key without grant");

    let agent_client =
        atlas_client::AtlasClient::new(server.base_url()).with_token(key_created.secret.clone());

    // A key with no grant receives a uniform 404 at the workspace gate — it cannot
    // even reach the project list, which is a stronger guarantee than per-row filtering.
    let result = agent_client.list_projects(&ws.slug, None, None).await;
    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 404),
        "agent with no grant must be denied at the workspace gate (404), got: {result:?}"
    );

    db.teardown().await;
}

/// A non-creator workspace admin must see private projects owned by other users.
/// Implicit admin (Rule 1 of resolve()) applies to the list query.
#[tokio::test]
async fn workspace_admin_sees_other_users_private_project_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "perm-adminvis-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            private_proj_req("Admin Can See This", "admin-visible-priv"),
        )
        .await
        .expect("create private project");

    let (admin, _) = add_member(
        &db,
        &server,
        ws.id,
        "perm-adminvis-admin",
        MemberRole::Admin,
    )
    .await;

    let page = admin
        .list_projects(&ws.slug, None, None)
        .await
        .expect("admin list request must succeed");

    let found = page.items.iter().any(|p| p.id == project.id);
    assert!(
        found,
        "workspace admin must see private projects created by other members"
    );

    db.teardown().await;
}

/// A user holding only a workspace-scoped viewer grant (no direct project grant)
/// must see private projects — workspace-scope grants give access to everything in
/// the workspace per the grant resolution chain.
#[tokio::test]
async fn user_with_workspace_scoped_grant_sees_private_projects_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-wsgrant-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            private_proj_req("WS-Grant Visible", "ws-grant-priv"),
        )
        .await
        .expect("create private project");

    let (grantee, grantee_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-wsgrant-viewer",
        MemberRole::Member,
    )
    .await;

    use atlas_domain::entities::permissions::NewPermissionGrant;
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: Some(grantee_user.id),
            api_key_id: None,
            project_id: None,
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Viewer,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed workspace-scoped viewer grant");

    let page = grantee
        .list_projects(&ws.slug, None, None)
        .await
        .expect("grantee list request must succeed");

    let found = page.items.iter().any(|p| p.id == project.id);
    assert!(
        found,
        "user with workspace-scoped viewer grant must see private projects in the list"
    );

    db.teardown().await;
}

/// A 403 returned because of a ShareDenied guardrail must not expose the internal
/// variant name in the response body.
#[tokio::test]
async fn share_denied_403_does_not_leak_variant_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, owner_user) =
        support::login_user_with_workspace(&server, &db, "perm-sharedeny-owner").await;

    let project = owner
        .create_project(&ws.slug, proj_req("ShareDeny Project", "sharedeny-proj"))
        .await
        .expect("create project");

    let key_created = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "sharedeny-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create api key");

    use atlas_domain::entities::permissions::NewPermissionGrant;
    use atlas_domain::ids::{ApiKeyId, ProjectId};
    let grant_repo = atlas_server::persistence::repos::PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key_created.id)),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: atlas_domain::permissions::ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed agent editor grant");

    let (_, grantee_user) = add_member(
        &db,
        &server,
        ws.id,
        "perm-sharedeny-grantee",
        MemberRole::Member,
    )
    .await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!(
            "{}/v1/workspaces/{}/projects/sharedeny-proj/grants",
            server.base_url(),
            ws.slug
        ))
        .bearer_auth(key_created.secret.clone())
        .json(&CreateGrantRequest {
            principal: GrantPrincipal {
                r#type: "user".to_string(),
                id: grantee_user.id.0,
            },
            role: "viewer".to_string(),
        })
        .send()
        .await
        .expect("send request");

    assert_eq!(resp.status().as_u16(), 403, "must be 403");

    let body = resp.text().await.expect("read body");

    assert!(
        !body.contains("AgentsNeverManageGrants"),
        "403 body must not contain internal variant name 'AgentsNeverManageGrants'; got: {body}"
    );
    assert!(
        !body.contains("RoleExceedsGrantors"),
        "403 body must not contain internal variant name 'RoleExceedsGrantors'; got: {body}"
    );
    assert!(
        !body.contains("InsufficientRoleToShare"),
        "403 body must not contain internal variant name 'InsufficientRoleToShare'; got: {body}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn owner_sees_own_private_project_in_list() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let (owner, ws, _) =
        support::login_user_with_workspace(&server, &db, "perm-ownpriv-owner").await;

    let project = owner
        .create_project(
            &ws.slug,
            private_proj_req("Owner Private", "owner-private-proj"),
        )
        .await
        .expect("create private project");

    let page = owner
        .list_projects(&ws.slug, None, None)
        .await
        .expect("owner must be able to list projects");

    let found = page.items.iter().any(|p| p.id == project.id);
    assert!(
        found,
        "owner must see their own private project in the list"
    );

    db.teardown().await;
}
