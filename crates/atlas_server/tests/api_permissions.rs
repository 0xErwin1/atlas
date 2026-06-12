#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    CreateApiKeyRequest, CreateGrantRequest, CreateProjectRequest, GrantPrincipal,
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
            password_hash: hash,
            is_root: false,
        })
        .await
        .expect("create user");

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
        .create_api_key(
            &ws.slug,
            CreateApiKeyRequest {
                name: "test-agent-key".to_string(),
                expires_at: None,
            },
        )
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
        result.is_err(),
        "agent must not be allowed to create grants"
    );

    if let Err(atlas_client::ClientError::Api(problem)) = result {
        assert_eq!(
            problem.status, 403,
            "agent grant attempt must return 403, got: {problem:?}"
        );
    }

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
        result.is_err(),
        "editor must not be allowed to grant admin (above own level)"
    );

    if let Err(atlas_client::ClientError::Api(problem)) = result {
        assert_eq!(
            problem.status, 403,
            "editor granting admin must return 403, got: {problem:?}"
        );
    }

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
            },
        )
        .await;

    assert!(
        result.is_err(),
        "viewer must not be allowed to update a project (editor-min route)"
    );

    if let Err(atlas_client::ClientError::Api(problem)) = result {
        assert_eq!(
            problem.status, 403,
            "viewer updating project must return 403, got: {problem:?}"
        );
    }

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
        result.is_err(),
        "non-member must not be able to access another workspace's project"
    );

    if let Err(atlas_client::ClientError::Api(problem)) = result {
        assert!(
            problem.status == 403 || problem.status == 404,
            "non-member accessing workspace resource must return 403 or 404, got: {problem:?}"
        );
    }

    db.teardown().await;
}
