#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! Integration tests for group CRUD + member management + atomic security audit.
//!
//! TDD RED tests: these will fail until routes/groups.rs is wired in.

mod support;

use atlas_api::dtos::groups::{AddGroupMemberRequest, CreateGroupRequest};
use atlas_client::ClientError;
use atlas_domain::{
    Actor, WorkspaceCtx,
    entities::{identity::MemberRole, security_audit::AuditFilters},
};
use atlas_server::persistence::repos::{
    MembershipRepo, NewUser, PgSecurityAuditRepo, SecurityAuditRepo, UserRepo,
};
use support::{TestDb, TestServer, login_user_with_workspace};

async fn add_workspace_member(
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
        .expect("add member");

    user
}

async fn fetch_audit(
    db: &TestDb,
    ws_id: atlas_domain::ids::WorkspaceId,
) -> Vec<atlas_domain::entities::security_audit::SecurityAuditEvent> {
    let repo = PgSecurityAuditRepo {
        conn: db.conn().clone(),
    };
    repo.list_for_workspace(ws_id, &AuditFilters::default(), None, 100)
        .await
        .expect("list audit")
}

// ---------------------------------------------------------------------------
// Group create
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_group_returns_201() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-create-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Engineering".to_string(),
            },
        )
        .await
        .expect("create group");

    assert_eq!(group.name, "Engineering");
    assert_eq!(group.workspace_id, ws.id.0);
}

#[tokio::test]
async fn create_group_emits_audit_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, caller) = login_user_with_workspace(&server, &db, "grp-audit-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Audited".to_string(),
            },
        )
        .await
        .expect("create group");

    let rows = fetch_audit(&db, ws.id).await;

    let entry = rows
        .iter()
        .find(|r| r.action.as_str() == "group.created")
        .expect("group.created audit row");

    assert_eq!(entry.target_id, Some(group.id));
    assert_eq!(entry.actor, Actor::User(caller.id));
}

#[tokio::test]
async fn create_group_duplicate_name_returns_409() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-dup-1").await;

    client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Duped".to_string(),
            },
        )
        .await
        .expect("first create");

    let err = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Duped".to_string(),
            },
        )
        .await
        .expect_err("duplicate name");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 409),
        "expected 409, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Group list
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_groups_returns_all_active_groups() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-list-1").await;

    client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Alpha".to_string(),
            },
        )
        .await
        .expect("create alpha");
    client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Beta".to_string(),
            },
        )
        .await
        .expect("create beta");

    let groups = client.list_groups(&ws.slug).await.expect("list groups");
    assert_eq!(groups.len(), 2);
}

// ---------------------------------------------------------------------------
// Group soft-delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn delete_group_removes_it_from_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-del-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "ToDelete".to_string(),
            },
        )
        .await
        .expect("create");

    client
        .delete_group(&ws.slug, group.id)
        .await
        .expect("delete");

    let groups = client.list_groups(&ws.slug).await.expect("list groups");
    assert!(
        groups.iter().all(|g| g.id != group.id),
        "deleted group still in list"
    );
}

#[tokio::test]
async fn delete_group_emits_audit_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, caller) = login_user_with_workspace(&server, &db, "grp-del-audit-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Ephemeral".to_string(),
            },
        )
        .await
        .expect("create");

    client
        .delete_group(&ws.slug, group.id)
        .await
        .expect("delete");

    let rows = fetch_audit(&db, ws.id).await;

    assert!(
        rows.iter().any(|r| r.action.as_str() == "group.deleted"
            && r.target_id == Some(group.id)
            && r.actor == Actor::User(caller.id)),
        "group.deleted audit row not found"
    );
}

#[tokio::test]
async fn delete_group_not_found_returns_404() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-del-404").await;

    let err = client
        .delete_group(&ws.slug, uuid::Uuid::now_v7())
        .await
        .expect_err("should 404");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 404),
        "expected 404, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Member add
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_member_to_group_and_list() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-member-1").await;

    let target = add_workspace_member(&db, ws.id, "grp-member-target-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Team".to_string(),
            },
        )
        .await
        .expect("create group");

    let member = client
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: target.id.0,
            },
        )
        .await
        .expect("add member");

    assert_eq!(member.user_id, target.id.0);
    assert_eq!(member.group_id, group.id);

    let members = client
        .list_group_members(&ws.slug, group.id)
        .await
        .expect("list members");

    assert!(members.iter().any(|m| m.user_id == target.id.0));
}

#[tokio::test]
async fn add_member_emits_audit_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, caller) = login_user_with_workspace(&server, &db, "grp-madd-audit").await;

    let target = add_workspace_member(&db, ws.id, "grp-madd-target").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "WithAudit".to_string(),
            },
        )
        .await
        .expect("create group");

    client
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: target.id.0,
            },
        )
        .await
        .expect("add member");

    let rows = fetch_audit(&db, ws.id).await;

    assert!(
        rows.iter()
            .any(|r| r.action.as_str() == "group.member_added"
                && r.target_id == Some(group.id)
                && r.actor == Actor::User(caller.id)),
        "group.member_added audit row not found"
    );
}

#[tokio::test]
async fn add_member_not_workspace_member_returns_422() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-nonmember-1").await;

    let non_member = db
        .user_repo()
        .create(NewUser {
            username: "grp-nonmember-target".to_string(),
            display_name: "NonMember".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "ClosedGroup".to_string(),
            },
        )
        .await
        .expect("create group");

    let err = client
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: non_member.id.0,
            },
        )
        .await
        .expect_err("should 422");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 422),
        "expected 422, got {err:?}"
    );
}

// ---------------------------------------------------------------------------
// Member remove
// ---------------------------------------------------------------------------

#[tokio::test]
async fn remove_member_from_group() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, _) = login_user_with_workspace(&server, &db, "grp-mrem-1").await;

    let target = add_workspace_member(&db, ws.id, "grp-mrem-target-1").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "TempTeam".to_string(),
            },
        )
        .await
        .expect("create group");

    client
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: target.id.0,
            },
        )
        .await
        .expect("add member");

    client
        .remove_group_member(&ws.slug, group.id, target.id.0)
        .await
        .expect("remove member");

    let members = client
        .list_group_members(&ws.slug, group.id)
        .await
        .expect("list members");

    assert!(members.iter().all(|m| m.user_id != target.id.0));
}

#[tokio::test]
async fn remove_member_emits_audit_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (client, ws, caller) = login_user_with_workspace(&server, &db, "grp-mrem-audit").await;

    let target = add_workspace_member(&db, ws.id, "grp-mrem-audit-target").await;

    let group = client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "AuditedTeam".to_string(),
            },
        )
        .await
        .expect("create group");

    client
        .add_group_member(
            &ws.slug,
            group.id,
            AddGroupMemberRequest {
                user_id: target.id.0,
            },
        )
        .await
        .expect("add member");

    client
        .remove_group_member(&ws.slug, group.id, target.id.0)
        .await
        .expect("remove member");

    let rows = fetch_audit(&db, ws.id).await;

    assert!(
        rows.iter()
            .any(|r| r.action.as_str() == "group.member_removed"
                && r.target_id == Some(group.id)
                && r.actor == Actor::User(caller.id)),
        "group.member_removed audit row not found"
    );
}

// ---------------------------------------------------------------------------
// Authorization: only owner/admin can manage groups
// ---------------------------------------------------------------------------

#[tokio::test]
async fn plain_member_cannot_create_group() {
    let db = TestDb::create().await.expect("TestDb::create");
    let server = TestServer::spawn(&db).await;
    let (_, ws, _) = login_user_with_workspace(&server, &db, "grp-authz-owner").await;

    add_workspace_member(&db, ws.id, "grp-authz-member").await;

    use atlas_api::dtos::LoginRequest;
    let mut member_client = atlas_client::AtlasClient::new(server.base_url().to_string());
    member_client
        .login(LoginRequest {
            username: "grp-authz-member".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await
        .expect("login member");

    let err = member_client
        .create_group(
            &ws.slug,
            CreateGroupRequest {
                name: "Forbidden".to_string(),
            },
        )
        .await
        .expect_err("should be 403");

    assert!(
        matches!(err, ClientError::Api(ref p) if p.status == 403),
        "expected 403, got {err:?}"
    );
}
