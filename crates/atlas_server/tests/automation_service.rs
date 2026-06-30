#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_server::persistence::repos::{PgAutomationRuleRepo, PgIntegrationConfigRepo};
use sea_orm::TransactionTrait;

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
