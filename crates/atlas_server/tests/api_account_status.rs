//! B1 tests: account_status field on assignee/member DTOs and assign-block rule.
//!
//! This test file validates the "keep + mark" tier-3 account lifecycle model:
//! disabled/pending users remain visible on assignee and member reads, marked
//! with their account state, and new assignments to a disabled user are blocked
//! with a 422.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    LoginRequest,
    boards_tasks::{
        AddAssigneeRequest, CreateBoardRequest, CreateColumnRequest, CreateTaskRequest,
    },
};
use atlas_client::{AtlasClient, ClientError};
use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole};
use atlas_server::persistence::repos::{MembershipRepo, NewUser, UserRepo};

// ---------------------------------------------------------------------------
// Shared fixture helpers
// ---------------------------------------------------------------------------

/// Creates a board, a column, and a task. Returns (board_id, col_id, readable_id).
async fn seed_task(
    client: &AtlasClient,
    ws: &str,
    project: &str,
    prefix: &str,
    name: &str,
) -> String {
    client
        .create_project(
            ws,
            atlas_api::dtos::CreateProjectRequest {
                name: format!("Project {name}"),
                slug: project.to_string(),
                task_prefix: prefix.to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = client
        .create_board(
            ws,
            project,
            CreateBoardRequest {
                name: "Board".to_string(),
            },
        )
        .await
        .expect("create board");

    let col = client
        .create_column(
            ws,
            board.id,
            CreateColumnRequest {
                name: "Todo".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("create column");

    let task = client
        .create_task(
            ws,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: name.to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("create task");

    task.readable_id
}

// ---------------------------------------------------------------------------
// T01: disabled assignee is still returned, with account_status = "deactivated"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disabled_assignee_is_visible_and_marked_deactivated() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t01-owner").await;

    // Create a second user (member) and assign them to a task.
    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t01-member".to_string(),
            display_name: "Member T01".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, member_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    let task_rid = seed_task(&client, &ws.slug, "acst-t01-proj", "T1", "Task T01").await;

    client
        .add_assignee(
            &ws.slug,
            &task_rid,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: member_user.id.0,
            },
        )
        .await
        .expect("add assignee");

    // Disable the user AFTER assigning.
    db.user_repo()
        .disable(member_user.id)
        .await
        .expect("disable user");

    let list = client
        .list_assignees(&ws.slug, &task_rid)
        .await
        .expect("list assignees");

    assert_eq!(list.len(), 1, "disabled assignee must still appear in list");

    let assignee_actor = &list[0].assignee;
    assert_eq!(
        assignee_actor.account_status.as_deref(),
        Some("deactivated"),
        "disabled user must have account_status = 'deactivated'"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T02: pending user is assignable and returns account_status = "pending"
// ---------------------------------------------------------------------------

#[tokio::test]
async fn pending_user_assignable_and_marked_pending() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t02-owner").await;

    // A pending user: created but NOT activated (activated_at IS NULL).
    let pending_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t02-pending".to_string(),
            display_name: "Pending T02".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create pending user");

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, pending_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    let task_rid = seed_task(&client, &ws.slug, "acst-t02-proj", "T2", "Task T02").await;

    let result = client
        .add_assignee(
            &ws.slug,
            &task_rid,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: pending_user.id.0,
            },
        )
        .await;

    assert!(
        result.is_ok(),
        "pending (activated_at NULL) user must be assignable; got: {result:?}"
    );

    let list = client
        .list_assignees(&ws.slug, &task_rid)
        .await
        .expect("list assignees");

    assert_eq!(list.len(), 1, "pending assignee must appear in list");

    let assignee_actor = &list[0].assignee;
    assert_eq!(
        assignee_actor.account_status.as_deref(),
        Some("pending"),
        "pending user must have account_status = 'pending'"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T03: new assignment to a DISABLED user → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn new_assignment_to_disabled_user_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t03-owner").await;

    let disabled_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t03-disabled".to_string(),
            display_name: "Disabled T03".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create disabled user");

    support::activate_user_in_db(&db, disabled_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, disabled_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    // Disable BEFORE attempting to assign.
    db.user_repo()
        .disable(disabled_user.id)
        .await
        .expect("disable user");

    let task_rid = seed_task(&client, &ws.slug, "acst-t03-proj", "T3", "Task T03").await;

    let result = client
        .add_assignee(
            &ws.slug,
            &task_rid,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: disabled_user.id.0,
            },
        )
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 422),
        "assigning a disabled user must return 422, got: {result:?}"
    );

    // Confirm the 422 carries an actionable message mentioning deactivated/re-enable.
    if let Err(ClientError::Api(ref p)) = result {
        let detail = p.detail.as_deref().unwrap_or("");
        let has_context = detail.to_lowercase().contains("deactivat")
            || detail.to_lowercase().contains("re-enable");
        assert!(
            has_context,
            "422 detail must mention deactivated/re-enable, got: '{detail}'"
        );
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T04: existing assignment to a user who is THEN disabled — still listed
// ---------------------------------------------------------------------------

#[tokio::test]
async fn existing_assignment_stays_visible_after_user_disabled() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t04-owner").await;

    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t04-member".to_string(),
            display_name: "Member T04".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, member_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    let task_rid = seed_task(&client, &ws.slug, "acst-t04-proj", "T4", "Task T04").await;

    client
        .add_assignee(
            &ws.slug,
            &task_rid,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: member_user.id.0,
            },
        )
        .await
        .expect("add assignee while active");

    db.user_repo()
        .disable(member_user.id)
        .await
        .expect("disable user");

    let list = client
        .list_assignees(&ws.slug, &task_rid)
        .await
        .expect("list assignees after disable");

    assert_eq!(
        list.len(),
        1,
        "existing assignee must still appear after being disabled"
    );
    assert_eq!(
        list[0].assignee.account_status.as_deref(),
        Some("deactivated"),
        "existing assignee after disable must show account_status = 'deactivated'"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T05: member list marks state correctly; api_key members have no account_status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn member_list_marks_account_status_correctly() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _owner) =
        support::login_user_with_workspace(&server, &db, "acst-t05-owner").await;

    let ctx = WorkspaceCtx::new(ws.id, atlas_domain::Actor::User(_owner.id));

    // Active member.
    let active_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t05-active".to_string(),
            display_name: "Active T05".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create active user");

    support::activate_user_in_db(&db, active_user.id.0).await;

    db.membership_repo()
        .add(&ctx, active_user.id, MemberRole::Member)
        .await
        .expect("add active membership");

    // Deactivated member.
    let deact_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t05-deact".to_string(),
            display_name: "Deactivated T05".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create deact user");

    support::activate_user_in_db(&db, deact_user.id.0).await;

    db.membership_repo()
        .add(&ctx, deact_user.id, MemberRole::Member)
        .await
        .expect("add deact membership");

    db.user_repo()
        .disable(deact_user.id)
        .await
        .expect("disable user");

    // Pending member (activated_at IS NULL).
    let pending_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t05-pending".to_string(),
            display_name: "Pending T05".to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create pending user");

    db.membership_repo()
        .add(&ctx, pending_user.id, MemberRole::Member)
        .await
        .expect("add pending membership");

    let members = client
        .list_workspace_members(&ws.slug)
        .await
        .expect("list members");

    let find = |uid: uuid::Uuid| {
        members
            .iter()
            .find(|m| m.id == uid)
            .unwrap_or_else(|| panic!("member {uid} not found in list"))
    };

    let active_m = find(active_user.id.0);
    assert_eq!(
        active_m.account_status.as_deref(),
        Some("active"),
        "active user must have account_status = 'active'"
    );

    let deact_m = find(deact_user.id.0);
    assert_eq!(
        deact_m.account_status.as_deref(),
        Some("deactivated"),
        "disabled user must have account_status = 'deactivated'"
    );

    let pending_m = find(pending_user.id.0);
    assert_eq!(
        pending_m.account_status.as_deref(),
        Some("pending"),
        "pending user must have account_status = 'pending'"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T06: attribution (created_by / assigned_by) does NOT carry account_status
// ---------------------------------------------------------------------------

#[tokio::test]
async fn attribution_actor_has_no_account_status() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t06-owner").await;

    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t06-member".to_string(),
            display_name: "Member T06".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, member_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    // Disable the user so it would carry account_status if we erroneously
    // populate attribution paths too.
    db.user_repo()
        .disable(member_user.id)
        .await
        .expect("disable user");

    let task_rid = seed_task(&client, &ws.slug, "acst-t06-proj", "T6", "Task T06").await;

    // `add_assignee` returns AssigneeDto containing both `assignee` (the user-assignee
    // actor — SHOULD carry account_status) and `assigned_by` (attribution — MUST NOT).
    let assignee_dto = client
        .add_assignee(
            &ws.slug,
            &task_rid,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: owner.id.0,
            },
        )
        .await
        .expect("add owner as assignee");

    // assigned_by is the attribution actor → must NOT carry account_status.
    assert!(
        assignee_dto.assigned_by.account_status.is_none(),
        "assigned_by (attribution) must not carry account_status, got: {:?}",
        assignee_dto.assigned_by.account_status
    );

    // Verify created_by on the task DTO also has no account_status.
    let task = client
        .get_task(&ws.slug, &task_rid)
        .await
        .expect("get task");

    assert!(
        task.created_by.account_status.is_none(),
        "created_by (attribution) must not carry account_status, got: {:?}",
        task.created_by.account_status
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T07: auth boundary regression locks — disabled user cannot log in
// ---------------------------------------------------------------------------

#[tokio::test]
async fn disabled_user_cannot_login() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "acst-t07-disabled").await;

    db.user_repo().disable(user.id).await.expect("disable user");

    let result = AtlasClient::new(server.base_url().to_string())
        .login(LoginRequest {
            username: "acst-t07-disabled".to_string(),
            password: "TestPassword1!".to_string(),
        })
        .await;

    assert!(
        matches!(result, Err(ClientError::Api(ref p)) if p.status == 401),
        "disabled user must return 401 on login, got: {result:?}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// T08: kanban board summary path also marks disabled assignees
// ---------------------------------------------------------------------------

#[tokio::test]
async fn board_summary_marks_disabled_assignee_deactivated() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, owner) =
        support::login_user_with_workspace(&server, &db, "acst-t08-owner").await;

    let member_user = db
        .user_repo()
        .create(NewUser {
            username: "acst-t08-member".to_string(),
            display_name: "Member T08".to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(&db, member_user.id.0).await;

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    db.membership_repo()
        .add(&ctx, member_user.id, MemberRole::Member)
        .await
        .expect("add membership");

    client
        .create_project(
            &ws.slug,
            atlas_api::dtos::CreateProjectRequest {
                name: "Project T08".to_string(),
                slug: "acst-t08-proj".to_string(),
                task_prefix: "T8".to_string(),
                visibility: None,
                visibility_role: None,
            },
        )
        .await
        .expect("create project");

    let board = client
        .create_board(
            &ws.slug,
            "acst-t08-proj",
            CreateBoardRequest {
                name: "Board T08".to_string(),
            },
        )
        .await
        .expect("board");

    let col = client
        .create_column(
            &ws.slug,
            board.id,
            CreateColumnRequest {
                name: "Col".to_string(),
                before: None,
                after: None,
                color: None,
            },
        )
        .await
        .expect("col");

    let task = client
        .create_task(
            &ws.slug,
            board.id,
            CreateTaskRequest {
                column_id: col.id,
                title: "Task T08".to_string(),
                description: None,
                properties: None,
                before: None,
                after: None,
            },
        )
        .await
        .expect("task");

    client
        .add_assignee(
            &ws.slug,
            &task.readable_id,
            AddAssigneeRequest {
                assignee_type: "user".to_string(),
                assignee_id: member_user.id.0,
            },
        )
        .await
        .expect("add assignee");

    db.user_repo()
        .disable(member_user.id)
        .await
        .expect("disable");

    // The board task list (board_assignees_by_task path) is exercised by list_tasks.
    let summaries = client
        .list_tasks(&ws.slug, board.id, None, None)
        .await
        .expect("list board tasks");

    let task_summary = summaries
        .items
        .iter()
        .find(|s| s.id == task.id)
        .expect("task must appear in board listing");

    let assignee_actor = task_summary
        .assignees
        .first()
        .expect("task must have one assignee in summary");

    assert_eq!(
        assignee_actor.account_status.as_deref(),
        Some("deactivated"),
        "board summary must mark disabled assignee as 'deactivated'"
    );

    db.teardown().await;
}
