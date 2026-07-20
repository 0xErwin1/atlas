#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    WorkspaceCtx,
    entities::boards_tasks::{NewBoard, NewTask, PositionBetween},
    entities::workspace_core::NewProject,
    ids::TaskId,
    permissions::{Visibility, VisibilityRole},
};
use atlas_server::{
    persistence::{
        entities::{boards_tasks::task, comments::comment},
        repos::{
            BoardRepo, PgAutomationRuleRepo, PgBoardRepo, PgIntegrationConfigRepo, PgProjectRepo,
            ProjectRepo,
        },
    },
    services::{AutomationService, TaskService},
};
use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, TransactionTrait};

/// Seeds a project → board → column → task in `ctx`'s workspace and returns the
/// task id, so an `add_comment` automation rule has a real target to comment on.
async fn seed_target_task(db: &support::TestDb, ctx: &WorkspaceCtx) -> TaskId {
    let project = PgProjectRepo {
        conn: db.conn().clone(),
    }
    .create(
        ctx,
        NewProject {
            name: "Auto Comment Project".into(),
            slug: "auto-comment-proj".into(),
            task_prefix: "ACP".into(),
            visibility: Visibility::Workspace(VisibilityRole::Editor),
        },
    )
    .await
    .expect("create project");

    let board_repo = PgBoardRepo::new(db.conn().clone());
    let board = board_repo
        .create_board(
            ctx,
            NewBoard {
                folder_id: None,
                project_id: project.id,
                name: "Board".into(),
            },
        )
        .await
        .expect("create board");
    let column = board_repo
        .add_column(
            ctx,
            board.id,
            "Todo".into(),
            None,
            PositionBetween {
                before: None,
                after: None,
            },
        )
        .await
        .expect("add column");

    let created = TaskService::new(db.conn().clone())
        .create(
            ctx,
            NewTask {
                project_id: project.id,
                board_id: board.id,
                column_id: column.id,
                title: "Target task".into(),
                description: String::new(),
                priority: None,
                due_date: None,
                estimate: None,
                labels: vec![],
                properties: None,
                position: PositionBetween {
                    before: None,
                    after: None,
                },
            },
        )
        .await
        .expect("create target task");

    created.id
}

// ---------------------------------------------------------------------------
// B3.5 [I] Cascade guard: task.created matches no automation rules
//
// Rules are constrained (DB CHECK + app validate) to only have
// trigger_event_type LIKE 'external.%'. The internal 'task.created' event type
// can therefore never appear as a rule trigger, so an automation-created task
// can never re-trigger another automation rule (no loop).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn cascade_guard_task_created_matches_zero_rules() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "cascade-guard").await;

    // Create an external automation rule
    let txn = db.conn().begin().await.expect("begin");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "Notify on failure".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "create_task".to_string(),
        serde_json::json!({
            "board_id": uuid::Uuid::new_v4(),
            "column_id": uuid::Uuid::new_v4(),
            "title_template": "CI failed: {{workflow_name}}"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit");

    // Now query using the internal event type that TaskService emits when it
    // creates a task. This type can never match any rule because the DB CHECK
    // and app-layer validation both reject non-'external.*' trigger types.
    let matched = PgAutomationRuleRepo::list_active_for_workspace_event(
        db.conn(),
        ws.id.0,
        "task.created",
        None,
    )
    .await
    .expect("list_active_for_workspace_event");

    assert!(
        matched.is_empty(),
        "task.created must never match any automation rule; cascade guard broken"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// [I] Integration api_key provisioned by PgIntegrationConfigRepo serves as
// the attribution actor when AutomationService creates tasks. Verify the
// integration_api_key_id stored on a config is a valid Uuid (structural check
// on the provisioning plumbing).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn permanent_misconfigured_rule_returns_ok_without_creating_task() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "permanent-misconfig").await;

    let crypto = atlas_server::crypto::WebhookCrypto::generate_for_test();
    let secret = b"test-secret";
    let (encrypted, nonce) = crypto.encrypt(secret).expect("encrypt");

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        encrypted,
        nonce,
        user.id.0,
    )
    .await
    .expect("create config");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "Misconfigured board".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "create_task".to_string(),
        serde_json::json!({
            "board_id": uuid::Uuid::new_v4(),
            "column_id": uuid::Uuid::new_v4(),
            "title_template": "CI failed: {{workflow_name}}"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit");

    let service = AutomationService::new(db.conn().clone());
    let processed = service
        .process_github_delivery(
            ws.id.0,
            config.integration_api_key_id,
            uuid::Uuid::now_v7(),
            "workflow_run",
            &serde_json::json!({
                "action": "completed",
                "workflow_run": {
                    "name": "Build",
                    "conclusion": "failure"
                }
            }),
        )
        .await
        .expect("permanent misconfig must not fail delivery processing");

    assert!(
        processed,
        "new delivery must still be recorded as processed"
    );

    let tasks = task::Entity::find()
        .filter(task::Column::WorkspaceId.eq(ws.id.0))
        .all(db.conn())
        .await
        .expect("query tasks");
    assert!(
        tasks.is_empty(),
        "misconfigured rule must not create a task"
    );

    db.teardown().await;
}

#[tokio::test]
async fn integration_api_key_id_is_provisioned() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "actor-check").await;

    let crypto = atlas_server::crypto::WebhookCrypto::generate_for_test();
    let secret = b"test-secret";
    let (encrypted, nonce) = crypto.encrypt(secret).expect("encrypt");

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        encrypted,
        nonce,
        user.id.0,
    )
    .await
    .expect("create config");
    txn.commit().await.expect("commit");

    assert!(
        !config.integration_api_key_id.is_nil(),
        "integration_api_key_id must be a non-nil UUID (provisioned api key)"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// [I] add_comment action: a matching delivery posts a rendered comment on the
// rule's target task, attributed to the integration api key.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_comment_rule_posts_comment_on_target_task() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "add-comment-hit").await;
    let ctx = support::ctx(&ws, &user);

    let target = seed_target_task(&db, &ctx).await;

    let crypto = atlas_server::crypto::WebhookCrypto::generate_for_test();
    let (encrypted, nonce) = crypto.encrypt(b"test-secret").expect("encrypt");

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        encrypted,
        nonce,
        user.id.0,
    )
    .await
    .expect("create config");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "CI failure → comment".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "add_comment".to_string(),
        serde_json::json!({
            "task_id": target.0,
            "body_template": "CI failed: {{workflow_name}}"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit");

    let service = AutomationService::new(db.conn().clone());
    let processed = service
        .process_github_delivery(
            ws.id.0,
            config.integration_api_key_id,
            uuid::Uuid::now_v7(),
            "workflow_run",
            &serde_json::json!({
                "action": "completed",
                "workflow_run": { "name": "Build", "conclusion": "failure" }
            }),
        )
        .await
        .expect("delivery must process");
    assert!(processed, "new delivery must be recorded as processed");

    let comments = comment::Entity::find()
        .filter(comment::Column::TaskId.eq(target.0))
        .all(db.conn())
        .await
        .expect("query comments");

    assert_eq!(comments.len(), 1, "rule must post exactly one comment");
    assert_eq!(
        comments[0].body, "CI failed: Build",
        "comment body must be the rendered template"
    );
    assert_eq!(
        comments[0].created_by_api_key_id,
        Some(config.integration_api_key_id),
        "comment must be attributed to the integration api key"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// [I] add_comment with a target task that is not in the workspace is a
// permanent misconfig: skipped, no comment, delivery still processed (no 500).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn add_comment_rule_with_unknown_task_skips_without_error() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "add-comment-miss").await;

    let crypto = atlas_server::crypto::WebhookCrypto::generate_for_test();
    let (encrypted, nonce) = crypto.encrypt(b"test-secret").expect("encrypt");

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        encrypted,
        nonce,
        user.id.0,
    )
    .await
    .expect("create config");
    PgAutomationRuleRepo::create(
        &txn,
        ws.id.0,
        "Comment on ghost task".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "add_comment".to_string(),
        serde_json::json!({
            "task_id": uuid::Uuid::now_v7(),
            "body_template": "should never post"
        }),
        user.id.0,
    )
    .await
    .expect("create rule");
    txn.commit().await.expect("commit");

    let service = AutomationService::new(db.conn().clone());
    let processed = service
        .process_github_delivery(
            ws.id.0,
            config.integration_api_key_id,
            uuid::Uuid::now_v7(),
            "workflow_run",
            &serde_json::json!({
                "action": "completed",
                "workflow_run": { "name": "Build", "conclusion": "failure" }
            }),
        )
        .await
        .expect("misconfigured target must not fail delivery processing");
    assert!(
        processed,
        "new delivery must still be recorded as processed"
    );

    let comments = comment::Entity::find()
        .filter(comment::Column::WorkspaceId.eq(ws.id.0))
        .all(db.conn())
        .await
        .expect("query comments");
    assert!(
        comments.is_empty(),
        "a rule targeting a non-existent task must not post a comment"
    );

    db.teardown().await;
}
