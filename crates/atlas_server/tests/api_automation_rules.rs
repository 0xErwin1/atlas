#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use serde_json::Value;
use uuid::Uuid;

use atlas_domain::{Actor, WorkspaceCtx, entities::identity::MemberRole, ids::WorkspaceId};
use atlas_server::{
    auth::password,
    persistence::repos::{MembershipRepo, NewUser, PgMembershipRepo, PgUserRepo, UserRepo},
};

fn http() -> reqwest::Client {
    reqwest::Client::new()
}

async fn add_member_and_login(
    server: &support::TestServer,
    db: &support::TestDb,
    ws_id: Uuid,
    username: &str,
) -> String {
    use atlas_api::dtos::LoginRequest;

    let plaintext = "TestPassword1!";
    let hash = password::hash(plaintext.to_string()).await.expect("hash");

    let user_repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let membership_repo = PgMembershipRepo {
        conn: db.conn().clone(),
    };

    let user = user_repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(hash),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member user");

    support::activate_user_in_db(db, user.id.0).await;

    let ws_id_typed = WorkspaceId::from(ws_id);
    let ctx = WorkspaceCtx::new(ws_id_typed, Actor::User(user.id));
    membership_repo
        .add(&ctx, user.id, MemberRole::Member)
        .await
        .expect("add member");

    let resp = http()
        .post(format!("{}/v1/auth/login", server.base_url()))
        .json(&LoginRequest {
            username: username.to_string(),
            password: plaintext.to_string(),
        })
        .send()
        .await
        .expect("login");

    let body: Value = resp.json().await.expect("login body");
    body["token"].as_str().expect("token").to_string()
}

// ---------------------------------------------------------------------------
// B4.8 [I] Admin creates automation rule with valid payload → 201
// ---------------------------------------------------------------------------

#[tokio::test]
async fn admin_creates_automation_rule_returns_201() {
    use atlas_domain::{
        entities::boards_tasks::{NewBoard, PositionBetween},
        entities::workspace_core::NewProject,
        permissions::{Visibility, VisibilityRole},
    };
    use atlas_server::persistence::repos::{BoardRepo, PgProjectRepo, ProjectRepo};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) =
        support::login_user_with_workspace(&server, &db, "ar-admin-create").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = support::ctx(&ws, &user);
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "AR Admin Create Project".to_string(),
            slug: "ar-admin-proj".to_string(),
            task_prefix: "AAC".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("project");
    let board_repo = db.board_repo();
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                name: "B".to_string(),
                project_id: project.id,
            },
        )
        .await
        .expect("board");
    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "C".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("column");

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "CI failures",
            "trigger_event_type": "external.github.workflow_run",
            "trigger_filter": {"conclusion": "failure"},
            "action_type": "create_task",
            "action_params": {
                "board_id": board.id.0,
                "column_id": column.id.0,
                "title_template": "CI failed: {{workflow_name}}"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(resp.status(), 201, "admin create must return 201");

    let body: Value = resp.json().await.unwrap();
    assert!(body["id"].is_string(), "id must be present");
    assert_eq!(body["trigger_event_type"], "external.github.workflow_run");
    assert_eq!(body["is_active"], true);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// External event rules are workspace-scoped in v1; project_id is rejected.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn automation_rule_project_scope_rejected_for_external_triggers() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ar-project-scope").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "Project-scoped external rule",
            "trigger_event_type": "external.github.workflow_run",
            "project_id": Uuid::now_v7(),
            "action_type": "create_task",
            "action_params": {
                "board_id": Uuid::now_v7(),
                "column_id": Uuid::now_v7(),
                "title_template": "CI failed"
            }
        }))
        .send()
        .await
        .unwrap();

    assert_eq!(
        resp.status(),
        422,
        "external automation rules must reject project_id in v1"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Internal trigger type is rejected at the app layer → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn automation_rule_internal_trigger_rejected() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ar-internal-trigger").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "Bad rule",
            "trigger_event_type": "task.created",
            "action_type": "create_task",
            "action_params": {
                "board_id": Uuid::now_v7(),
                "column_id": Uuid::now_v7(),
                "title_template": "oops"
            }
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "internal trigger must be rejected with 400 or 422, got {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Invalid action type rejected → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn automation_rule_invalid_action_type_rejected() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ar-bad-action").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "Bad action",
            "trigger_event_type": "external.github.workflow_run",
            "action_type": "add_comment",
            "action_params": {}
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "invalid action type must be rejected, got {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Invalid action_params (missing required fields) → 422
// ---------------------------------------------------------------------------

#[tokio::test]
async fn automation_rule_invalid_action_params_rejected() {
    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, _user) =
        support::login_user_with_workspace(&server, &db, "ar-bad-params").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "Missing params",
            "trigger_event_type": "external.github.workflow_run",
            "action_type": "create_task",
            "action_params": {"something": "else"}
        }))
        .send()
        .await
        .unwrap();

    let status = resp.status().as_u16();
    assert!(
        status == 400 || status == 422,
        "invalid action_params must be rejected, got {status}"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Non-admin member receives 404 on all automation-rule endpoints
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_admin_rejected_on_automation_rule_endpoints() {
    use atlas_domain::{
        entities::boards_tasks::{NewBoard, PositionBetween},
        entities::workspace_core::NewProject,
        permissions::{Visibility, VisibilityRole},
    };
    use atlas_server::persistence::repos::{BoardRepo, PgProjectRepo, ProjectRepo};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (admin_client, ws, user) =
        support::login_user_with_workspace(&server, &db, "ar-nonadmin-admin").await;

    let admin_token = admin_client.token().expect("admin token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = support::ctx(&ws, &user);
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "AR Nonadmin Project".to_string(),
            slug: "ar-nonadmin-proj".to_string(),
            task_prefix: "ANP".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("project");
    let board_repo = db.board_repo();
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                name: "B".to_string(),
                project_id: project.id,
            },
        )
        .await
        .expect("board");
    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "C".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("column");

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(admin_token)
        .json(&serde_json::json!({
            "name": "Admin rule",
            "trigger_event_type": "external.github.workflow_run",
            "action_type": "create_task",
            "action_params": {
                "board_id": board.id.0,
                "column_id": column.id.0,
                "title_template": "{{workflow_name}}"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201, "admin create must succeed first");
    let created: Value = create_resp.json().await.unwrap();
    let rule_id = created["id"].as_str().unwrap().to_string();

    let member_token = add_member_and_login(&server, &db, ws.id.0, "ar-nonadmin-member").await;

    for (method, path, body) in [
        (
            "GET",
            format!("{base_url}/v1/workspaces/{ws_slug}/automation-rules"),
            None,
        ),
        (
            "GET",
            format!("{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"),
            None,
        ),
        (
            "POST",
            format!("{base_url}/v1/workspaces/{ws_slug}/automation-rules"),
            Some(
                serde_json::json!({"name":"x","trigger_event_type":"external.x","action_type":"create_task","action_params":{}}),
            ),
        ),
        (
            "PATCH",
            format!("{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"),
            Some(serde_json::json!({"is_active": false})),
        ),
        (
            "DELETE",
            format!("{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"),
            None,
        ),
    ] {
        let mut req = http().request(
            reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
            path.as_str(),
        );
        req = req.bearer_auth(&member_token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        let resp = req.send().await.unwrap();
        assert_eq!(resp.status(), 404, "non-admin {method} must be 404");
    }

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// B4.8 [I] Automation rule CRUD: get, list, patch, delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn automation_rule_crud() {
    use atlas_domain::{
        entities::boards_tasks::{NewBoard, PositionBetween},
        entities::workspace_core::NewProject,
        permissions::{Visibility, VisibilityRole},
    };
    use atlas_server::persistence::repos::{BoardRepo, PgProjectRepo, ProjectRepo};

    let db = support::TestDb::create().await.expect("TestDb");
    let server = support::TestServer::spawn(&db).await;
    let (client, ws, user) = support::login_user_with_workspace(&server, &db, "ar-crud").await;

    let token = client.token().expect("token");
    let base_url = server.base_url();
    let ws_slug = &ws.slug;

    let ctx = support::ctx(&ws, &user);
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        &ctx,
        NewProject {
            name: "AR CRUD Project".to_string(),
            slug: "ar-crud-proj".to_string(),
            task_prefix: "ACP".to_string(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("project");
    let board_repo = db.board_repo();
    let board = board_repo
        .create_board(
            &ctx,
            NewBoard {
                name: "B".to_string(),
                project_id: project.id,
            },
        )
        .await
        .expect("board");
    let column = board_repo
        .add_column(
            &ctx,
            board.id,
            "C".to_string(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("column");

    let create_resp = http()
        .post(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({
            "name": "CI rule",
            "trigger_event_type": "external.github.workflow_run",
            "trigger_filter": {"conclusion": "failure"},
            "action_type": "create_task",
            "action_params": {
                "board_id": board.id.0,
                "column_id": column.id.0,
                "title_template": "CI failed"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), 201);
    let created: Value = create_resp.json().await.unwrap();
    let rule_id = created["id"].as_str().unwrap().to_string();

    let get_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), 200);
    let get_body: Value = get_resp.json().await.unwrap();
    assert_eq!(get_body["name"], "CI rule");

    let list_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(list_resp.status(), 200);
    let list_body: Value = list_resp.json().await.unwrap();
    let items = list_body["items"].as_array().unwrap();
    assert!(!items.is_empty(), "list must return at least one rule");

    let patch_resp = http()
        .patch(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"
        ))
        .bearer_auth(token)
        .json(&serde_json::json!({"is_active": false, "name": "CI rule (disabled)"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patch_resp.status(), 200);
    let patched: Value = patch_resp.json().await.unwrap();
    assert_eq!(patched["is_active"], false);
    assert_eq!(patched["name"], "CI rule (disabled)");

    let delete_resp = http()
        .delete(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(delete_resp.status(), 204);

    let after_delete_resp = http()
        .get(format!(
            "{base_url}/v1/workspaces/{ws_slug}/automation-rules/{rule_id}"
        ))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        after_delete_resp.status(),
        404,
        "deleted rule must return 404"
    );

    db.teardown().await;
}
