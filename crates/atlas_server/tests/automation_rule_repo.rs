#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_server::persistence::repos::{AutomationRulePatch, PgAutomationRuleRepo};
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

fn valid_action_params() -> serde_json::Value {
    serde_json::json!({
        "board_id": uuid::Uuid::new_v4(),
        "column_id": uuid::Uuid::new_v4(),
        "title_template": "CI failed: {{workflow_name}}"
    })
}

// ---------------------------------------------------------------------------
// create and get — happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_get_rule() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-create").await;

    let created = PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "Fail rule".to_string(),
        "external.github.workflow_run".to_string(),
        Some(serde_json::json!({"conclusion": "failure"})),
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await
    .expect("create");

    assert_eq!(created.workspace_id, ws.id.0);
    assert_eq!(created.trigger_event_type, "external.github.workflow_run");
    assert!(created.is_active);
    assert!(created.deleted_at.is_none());

    let found = PgAutomationRuleRepo::get(db.conn(), ws.id.0, created.id)
        .await
        .expect("get")
        .expect("must find rule");

    assert_eq!(found.id, created.id);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// list_active_for_workspace_event — returns matching active rules
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_active_for_workspace_event_returns_matching_rules() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-list-event").await;

    PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "Fail rule".to_string(),
        "external.github.workflow_run".to_string(),
        None,
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await
    .expect("create");

    PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "PR rule".to_string(),
        "external.github.pull_request".to_string(),
        None,
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await
    .expect("create 2");

    let matching = PgAutomationRuleRepo::list_active_for_workspace_event(
        db.conn(),
        ws.id.0,
        "external.github.workflow_run",
        None,
    )
    .await
    .expect("list_active_for_workspace_event");

    assert_eq!(matching.len(), 1, "must only return rules matching the event type");
    assert_eq!(matching[0].trigger_event_type, "external.github.workflow_run");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// patch — updates name and is_active
// ---------------------------------------------------------------------------

#[tokio::test]
async fn patch_updates_name_and_is_active() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-patch").await;

    let created = PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "Original".to_string(),
        "external.github.workflow_run".to_string(),
        None,
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await
    .expect("create");

    let patched = PgAutomationRuleRepo::patch(
        db.conn(),
        ws.id.0,
        created.id,
        AutomationRulePatch {
            name: Some("Updated".to_string()),
            is_active: Some(false),
            trigger_filter: None,
            action_params: None,
        },
    )
    .await
    .expect("patch");

    assert_eq!(patched.name, "Updated");
    assert!(!patched.is_active);

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// soft_delete — hides rule from future queries
// ---------------------------------------------------------------------------

#[tokio::test]
async fn soft_delete_hides_rule() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-soft-del").await;

    let created = PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "To delete".to_string(),
        "external.github.workflow_run".to_string(),
        None,
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await
    .expect("create");

    PgAutomationRuleRepo::soft_delete(db.conn(), ws.id.0, created.id)
        .await
        .expect("soft_delete");

    let after = PgAutomationRuleRepo::get(db.conn(), ws.id.0, created.id)
        .await
        .expect("get after delete");

    assert!(after.is_none(), "get must return None after soft_delete");

    let matching = PgAutomationRuleRepo::list_active_for_workspace_event(
        db.conn(),
        ws.id.0,
        "external.github.workflow_run",
        None,
    )
    .await
    .expect("list after delete");

    assert!(matching.is_empty(), "deleted rule must not appear in list");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// app layer rejects trigger_event_type that does not match 'external.*'
// ---------------------------------------------------------------------------

#[tokio::test]
async fn app_layer_rejects_internal_event_type() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-app-reject").await;

    let result = PgAutomationRuleRepo::create(
        db.conn(),
        ws.id.0,
        "Bad rule".to_string(),
        "task.created".to_string(),
        None,
        None,
        "create_task".to_string(),
        valid_action_params(),
        user.id.0,
    )
    .await;

    assert!(
        result.is_err(),
        "create must reject non-external.* trigger_event_type at app layer"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// DB CHECK rejects trigger_event_type that does not match 'external.*'
// ---------------------------------------------------------------------------

#[tokio::test]
async fn db_check_rejects_internal_event_type() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ar-db-check").await;

    let result = db
        .conn()
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            r#"
            INSERT INTO automation_rules (
                id, workspace_id, name, is_active, trigger_event_type,
                trigger_filter, project_id, action_type, action_params,
                created_by_user_id, created_at, updated_at, deleted_at
            ) VALUES (
                gen_random_uuid(), $1, 'bad rule', true, 'task.created',
                NULL, NULL, 'create_task',
                '{"board_id": "00000000-0000-0000-0000-000000000001",
                  "column_id": "00000000-0000-0000-0000-000000000002",
                  "title_template": "t"}'::jsonb,
                $2, now(), now(), NULL
            )
            "#,
            [ws.id.0.into(), user.id.0.into()],
        ))
        .await;

    assert!(
        result.is_err(),
        "DB CHECK constraint must reject non-external.* trigger_event_type"
    );

    db.teardown().await;
}
