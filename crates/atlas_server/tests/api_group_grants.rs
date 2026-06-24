#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Integration tests — B2 adversarial matrix for group-as-principal grants.
//!
//! Both directions are tested: gain (group membership confers access) and
//! revoke (membership removal, group soft-delete, and direct+group overlap).

mod support;

use atlas_api::dtos::{
    CreateGrantRequest, CreateProjectRequest, GrantPrincipal, UpdateProjectRequest,
    groups::{AddGroupMemberRequest, CreateGroupRequest},
};
use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{identity::MemberRole, permissions::NewPermissionGrant},
    ids::{ApiKeyId, ProjectId},
    permissions::ResourceRole,
};
use atlas_server::persistence::repos::{
    MembershipRepo, NewUser, PermissionGrantRepo, PgGroupRepo, PgPermissionGrantRepo, UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn create_member(
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
    username: &str,
) -> atlas_domain::entities::identity::User {
    let hash = atlas_server::auth::password::hash("TestPassword1!".to_string())
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
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add membership");

    user
}

async fn login_as(server: &TestServer, username: &str) -> atlas_client::AtlasClient {
    use atlas_api::dtos::LoginRequest;
    let mut client = atlas_client::AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login");
    client
}

fn private_proj(name: &str, slug: &str) -> CreateProjectRequest {
    CreateProjectRequest {
        name: name.to_string(),
        slug: slug.to_string(),
        task_prefix: "GGR".to_string(),
        visibility: Some("private".to_string()),
        visibility_role: None,
    }
}

fn group_grant_req(group_id: Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "group".to_string(),
            id: group_id,
        },
        role: role.to_string(),
    }
}

fn user_grant_req(user_id: Uuid, role: &str) -> CreateGrantRequest {
    CreateGrantRequest {
        principal: GrantPrincipal {
            r#type: "user".to_string(),
            id: user_id,
        },
        role: role.to_string(),
    }
}

// ---------------------------------------------------------------------------
// GAIN: user via group membership resolves a grant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn gain_via_group_membership() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-gain-owner").await;

    owner
        .create_project(&ws.slug, private_proj("GG Gain Project", "gg-gain-proj"))
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-gain-member").await;
    let member_client = login_as(&server, "gg-gain-member").await;

    // Before group grant: member has no access to the private project.
    let result = member_client.get_project(&ws.slug, "gg-gain-proj").await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "member without grant must get 404, got: {result:?}"
    );

    // Create a group and grant it Editor on the project.
    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-gain-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-gain-proj",
            group_grant_req(group.id, "editor"),
        )
        .await
        .expect("grant group editor");

    // Still no access: member is not in the group yet.
    let result = member_client.get_project(&ws.slug, "gg-gain-proj").await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "member not yet in group must still get 404, got: {result:?}"
    );

    // Add member to the group → now they should resolve Editor.
    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add member to group");

    let project = member_client
        .get_project(&ws.slug, "gg-gain-proj")
        .await
        .expect("member should see the project via group grant");

    assert_eq!(project.slug, "gg-gain-proj");

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REVOKE via membership removal
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_via_membership_removal() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "gg-revoke-mem-owner").await;

    owner
        .create_project(
            &ws.slug,
            private_proj("GG Revoke Mem Project", "gg-revoke-mem-proj"),
        )
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-revoke-mem-member").await;
    let member_client = login_as(&server, "gg-revoke-mem-member").await;

    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-revoke-mem-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-revoke-mem-proj",
            group_grant_req(group.id, "editor"),
        )
        .await
        .expect("grant group editor");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add member to group");

    // Member now has access.
    member_client
        .get_project(&ws.slug, "gg-revoke-mem-proj")
        .await
        .expect("member should see project via group");

    // Remove from group → should lose access.
    owner
        .remove_group_member(&ws.slug, group.id, member.id.0)
        .await
        .expect("remove member from group");

    let result = member_client
        .get_project(&ws.slug, "gg-revoke-mem-proj")
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "removed member must get 404, got: {result:?}"
    );

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// REVOKE via group soft-delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn revoke_via_group_soft_delete() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) =
        login_user_with_workspace(&server, &db, "gg-revoke-del-owner").await;

    owner
        .create_project(
            &ws.slug,
            private_proj("GG Revoke Del Project", "gg-revoke-del-proj"),
        )
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-revoke-del-member").await;
    let member_client = login_as(&server, "gg-revoke-del-member").await;

    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-revoke-del-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-revoke-del-proj",
            group_grant_req(group.id, "admin"),
        )
        .await
        .expect("grant group admin");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add member to group");

    // Member has access via the group grant.
    member_client
        .get_project(&ws.slug, "gg-revoke-del-proj")
        .await
        .expect("member should see project");

    // Soft-delete the group → the grant row remains, but deleted-group exclusion
    // should make the member lose access.
    owner
        .delete_group(&ws.slug, group.id)
        .await
        .expect("delete group");

    let result = member_client
        .get_project(&ws.slug, "gg-revoke-del-proj")
        .await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "member whose group is deleted must get 404, got: {result:?}"
    );

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// MAX-ROLE: group Admin wins over direct Viewer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_role_group_admin_over_direct_viewer() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-max-1-owner").await;

    owner
        .create_project(&ws.slug, private_proj("GG Max1 Project", "gg-max-1-proj"))
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-max-1-member").await;
    let member_client = login_as(&server, "gg-max-1-member").await;

    // Direct grant: Viewer.
    owner
        .create_project_grant(
            &ws.slug,
            "gg-max-1-proj",
            user_grant_req(member.id.0, "viewer"),
        )
        .await
        .expect("grant user viewer");

    // Group grant: Admin.
    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-max-1-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-max-1-proj",
            group_grant_req(group.id, "admin"),
        )
        .await
        .expect("grant group admin");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add to group");

    // The member must be able to rename the project (requires Admin).
    let updated = member_client
        .update_project(
            &ws.slug,
            "gg-max-1-proj",
            UpdateProjectRequest {
                name: Some("GG Max1 Project Updated".to_string()),
                visibility: None,
                visibility_role: None,
                task_prefix: None,
            },
        )
        .await
        .expect("member with group Admin should be able to rename project");

    assert_eq!(updated.name, "GG Max1 Project Updated");

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// MAX-ROLE: direct Admin wins over group Viewer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn max_role_direct_admin_over_group_viewer() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-max-2-owner").await;

    owner
        .create_project(&ws.slug, private_proj("GG Max2 Project", "gg-max-2-proj"))
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-max-2-member").await;
    let member_client = login_as(&server, "gg-max-2-member").await;

    // Direct grant: Admin.
    owner
        .create_project_grant(
            &ws.slug,
            "gg-max-2-proj",
            user_grant_req(member.id.0, "admin"),
        )
        .await
        .expect("grant user admin");

    // Group grant: Viewer.
    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-max-2-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-max-2-proj",
            group_grant_req(group.id, "viewer"),
        )
        .await
        .expect("grant group viewer");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add to group");

    // The member must be able to rename the project (requires Admin — direct grant wins).
    let updated = member_client
        .update_project(
            &ws.slug,
            "gg-max-2-proj",
            UpdateProjectRequest {
                name: Some("GG Max2 Project Updated".to_string()),
                visibility: None,
                visibility_role: None,
                task_prefix: None,
            },
        )
        .await
        .expect("member with direct Admin should rename project even though group is Viewer");

    assert_eq!(updated.name, "GG Max2 Project Updated");

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// INHERITANCE: group workspace-grant reaches a project (scope inheritance)
// ---------------------------------------------------------------------------
//
// A group grant at workspace scope confers access to child resources (projects)
// exactly like a direct user grant does. This verifies the inheritance path is
// not broken by the group-ids injection.
#[tokio::test]
async fn group_workspace_grant_reaches_project() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-inherit-owner").await;

    // Private project — no default visibility.
    owner
        .create_project(
            &ws.slug,
            private_proj("GG Inherit Project", "gg-inherit-proj"),
        )
        .await
        .expect("create project");

    let member = create_member(&db, ws.id, "gg-inherit-member").await;
    let member_client = login_as(&server, "gg-inherit-member").await;

    // Member cannot see the private project yet.
    let result = member_client.get_project(&ws.slug, "gg-inherit-proj").await;
    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "member without grant must not see private project"
    );

    // Create a group and grant it Editor at workspace scope.
    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-inherit-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_workspace_grant(&ws.slug, group_grant_req(group.id, "editor"))
        .await
        .expect("grant group editor on workspace");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: member.id.0,
            },
        )
        .await
        .expect("add member to group");

    // Group workspace editor → should see the private project via inheritance.
    let fetched = member_client
        .get_project(&ws.slug, "gg-inherit-proj")
        .await
        .expect("member with group workspace grant should see private project");

    assert_eq!(fetched.slug, "gg-inherit-proj");

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// API KEY UNAFFECTED: api_key resolution does not gather group grants
//
// The invariant: an api_key with NO direct grant has no access to a private
// project, even when the api_key's owner is in a group that HAS a grant on that
// project. Groups apply only to user-principal resolution, never to api_key
// resolution. The api_key cap (≤Editor) also remains intact.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn api_key_resolution_unaffected_by_groups() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-apikey-owner").await;

    owner
        .create_project(
            &ws.slug,
            private_proj("GG ApiKey Project", "gg-apikey-proj"),
        )
        .await
        .expect("create project");

    // Create an api key for the owner. Give it NO direct grant.
    use atlas_api::dtos::CreateUserApiKeyRequest;
    let key = owner
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "gg-apikey-key".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
        })
        .await
        .expect("create api key");

    let agent_client =
        atlas_client::AtlasClient::new(server.base_url().to_string()).with_token(key.secret);

    // Create a group with Admin and add the owner to it, then grant the group Admin.
    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-apikey-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .create_project_grant(
            &ws.slug,
            "gg-apikey-proj",
            group_grant_req(group.id, "admin"),
        )
        .await
        .expect("grant group admin");

    owner
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: owner_user.id.0,
            },
        )
        .await
        .expect("add owner to group");

    // The api_key has NO direct grant. The group has Admin for the owner-user,
    // but group grants must NOT bleed into api_key resolution.
    // So the agent must NOT be able to read the private project.
    let result = agent_client.get_project(&ws.slug, "gg-apikey-proj").await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 404),
        "api key with no direct grant must get 404 even when owner's group has Admin, got: {result:?}"
    );

    // Give the api_key a direct Editor grant — now it can read but the cap holds.
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    let project = owner
        .get_project(&ws.slug, "gg-apikey-proj")
        .await
        .expect("get project");

    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: Some(ApiKeyId(key.id)),
            group_id: None,
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("seed api key direct grant");

    // Now the agent CAN read the project (Editor sufficient).
    let got = agent_client
        .get_project(&ws.slug, "gg-apikey-proj")
        .await
        .expect("agent with direct Editor grant should read the project");

    assert_eq!(got.slug, "gg-apikey-proj");

    // Granting: the api_key (as a grant target) should not be grantable Admin —
    // the agent cap prevents it. This also implicitly confirms the cap is intact
    // and group grants haven't inflated the api_key's effective role.
    let member = create_member(&db, ws.id, "gg-apikey-grantee").await;
    let result = agent_client
        .create_project_grant(
            &ws.slug,
            "gg-apikey-proj",
            user_grant_req(member.id.0, "viewer"),
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 403),
        "api key principal must never manage grants (agents cannot share), got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// GRANT-TO-GROUP surfaces correctly in list_for_resource
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grant_to_group_appears_in_list_with_group_principal_type() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-list-owner").await;

    owner
        .create_project(&ws.slug, private_proj("GG List Project", "gg-list-proj"))
        .await
        .expect("create project");

    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-list-group".to_string(),
            },
        )
        .await
        .expect("create group");

    let grant = owner
        .create_project_grant(
            &ws.slug,
            "gg-list-proj",
            group_grant_req(group.id, "viewer"),
        )
        .await
        .expect("create group grant");

    // The grant principal type must be "group", not "unknown".
    assert_eq!(
        grant.principal.r#type, "group",
        "grant.principal.type must be 'group', got '{}'",
        grant.principal.r#type
    );
    assert_eq!(grant.principal.id, group.id);

    // list_for_resource must return the group grant with type "group".
    let page = owner
        .list_project_grants(&ws.slug, "gg-list-proj", None, None)
        .await
        .expect("list project grants");

    let found = page
        .items
        .iter()
        .find(|g| g.id == grant.id)
        .expect("grant must appear in list");

    assert_eq!(
        found.principal.r#type, "group",
        "listed grant.principal.type must be 'group', got '{}'",
        found.principal.r#type
    );
    assert_eq!(found.principal.id, group.id);
    assert_eq!(found.role, "viewer");

    // Revoking the group grant must work.
    owner
        .delete_project_grant(&ws.slug, "gg-list-proj", grant.id)
        .await
        .expect("delete group grant");

    let page_after = owner
        .list_project_grants(&ws.slug, "gg-list-proj", None, None)
        .await
        .expect("list after delete");

    assert!(
        !page_after.items.iter().any(|g| g.id == grant.id),
        "deleted group grant must not appear in list"
    );

    let _ = owner_user;
    db.teardown().await;
}

// ---------------------------------------------------------------------------
// PARSE_PRINCIPAL validation: wrong-workspace group → 400/422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grant_to_group_from_wrong_workspace_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;

    // Two separate workspaces.
    let (owner_a, ws_a, _) = login_user_with_workspace(&server, &db, "gg-xws-owner-a").await;
    let (owner_b, ws_b, _) = login_user_with_workspace(&server, &db, "gg-xws-owner-b").await;

    owner_a
        .create_project(&ws_a.slug, private_proj("GG XWS Project", "gg-xws-proj"))
        .await
        .expect("create project in ws_a");

    // Create a group in workspace B.
    let group_b = owner_b
        .create_group(
            &ws_b.slug,
            CreateGroupRequest {
                name: "gg-xws-group-b".to_string(),
            },
        )
        .await
        .expect("create group in ws_b");

    // Attempt to grant this ws_b group on a ws_a resource → must be rejected.
    let result = owner_a
        .create_project_grant(
            &ws_a.slug,
            "gg-xws-proj",
            group_grant_req(group_b.id, "editor"),
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 400 || p.status == 422),
        "granting a group from a different workspace must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// PARSE_PRINCIPAL validation: soft-deleted group → 400/422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn grant_to_deleted_group_rejected() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, _) = login_user_with_workspace(&server, &db, "gg-delgrp-owner").await;

    owner
        .create_project(
            &ws.slug,
            private_proj("GG Del Group Project", "gg-delgrp-proj"),
        )
        .await
        .expect("create project");

    let group = owner
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "gg-delgrp-group".to_string(),
            },
        )
        .await
        .expect("create group");

    owner
        .delete_group(&ws.slug, group.id)
        .await
        .expect("soft-delete group");

    // Attempt to grant the deleted group → must be rejected.
    let result = owner
        .create_project_grant(
            &ws.slug,
            "gg-delgrp-proj",
            group_grant_req(group.id, "viewer"),
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 400 || p.status == 422),
        "granting a soft-deleted group must be rejected, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// LOW-LEVEL: upsert_in for a group grant reads back the correct row (not wrong row)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn upsert_group_grant_reads_back_correct_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (owner, ws, owner_user) = login_user_with_workspace(&server, &db, "gg-upsert-owner").await;

    owner
        .create_project(
            &ws.slug,
            private_proj("GG Upsert Project", "gg-upsert-proj"),
        )
        .await
        .expect("create project");

    let project = owner
        .get_project(&ws.slug, "gg-upsert-proj")
        .await
        .expect("get project");

    let group_repo = PgGroupRepo {
        conn: db.conn().clone(),
    };
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };

    use atlas_domain::entities::groups::NewGroup;
    let group = group_repo
        .create(NewGroup {
            workspace_id: ws.id,
            name: "gg-upsert-group".to_string(),
            created_by: owner_user.id,
        })
        .await
        .expect("create group via repo");

    // First upsert: Viewer.
    let g1 = grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: None,
            group_id: Some(group.id),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Viewer,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("first upsert");

    assert_eq!(g1.group_id, Some(group.id));
    assert_eq!(g1.role, ResourceRole::Viewer);

    // Second upsert (same unique key): Editor — must UPDATE not duplicate.
    let g2 = grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: ws.id,
            user_id: None,
            api_key_id: None,
            group_id: Some(group.id),
            project_id: Some(ProjectId(project.id)),
            folder_id: None,
            document_id: None,
            board_id: None,
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_user.id),
            created_by_api_key_id: None,
        })
        .await
        .expect("second upsert");

    assert_eq!(g2.group_id, Some(group.id));
    assert_eq!(
        g2.role,
        ResourceRole::Editor,
        "role must be updated to Editor"
    );

    // Only one row must exist for this (workspace, group, project) combination.
    use atlas_server::persistence::repos::PermissionGrantRepo;
    let all = grant_repo
        .list_for_resource(
            ws.id,
            &atlas_domain::permissions::ResourceRef::Project(ProjectId(project.id)),
            None,
            100,
        )
        .await
        .expect("list for resource");

    let group_grants: Vec<_> = all
        .iter()
        .filter(|g| g.group_id == Some(group.id))
        .collect();

    assert_eq!(group_grants.len(), 1, "must be exactly one group grant row");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// GroupRepoTrait needed for .create() call in upsert test
// ---------------------------------------------------------------------------

use atlas_domain::ports::group_repo::GroupRepo as GroupRepoTrait;
