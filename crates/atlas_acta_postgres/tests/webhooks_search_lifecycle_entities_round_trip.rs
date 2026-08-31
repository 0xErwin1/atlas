#![allow(clippy::expect_used)]

//! Characterization tests for the webhooks/automations/integration-configs/
//! saved-searches/task-views/lifecycle entity conversions, ported from
//! `atlas_server::persistence::entities::{automation_rule, integration_config,
//! lifecycle, saved_searches, task_views, webhook_delivery,
//! webhook_subscription}` before those eight tables move into this crate (S4
//! PR5, T5.1). The verbatim-move proof is the blob-level diff against the
//! pre-move files (empty for all seven), recorded in the PR; these tests pin
//! the conversion behavior and the core column set they assert — not every
//! column — and must keep passing, unmodified, once the `entities::*`
//! modules land (T5.4).
//!
//! `automation_rules`, `purge_operations`, `purge_operation_digests`,
//! `webhook_delivery_log`, and `webhook_subscriptions` have no domain `_from`
//! conversion function today (their repos read the sea-orm `Model` fields
//! directly); their tests below pin the moved `Model`'s column set the same
//! way PR2/PR4 did for conversion-less entities.
//!
//! `search_embeddings` and `search_index_queue` (design D1 batch 5) have no
//! sea-orm entity struct at all — their repos (`semantic_search.rs`,
//! `search_index_queue.rs`) read/write those tables with raw SQL only. There
//! is nothing to move at the entity layer for those two tables in this PR;
//! their repo move (and any characterization coverage) lands with the repo
//! moves in PR8.

use atlas_acta::entities::saved_searches::Owner as SavedSearchOwner;
use atlas_acta::entities::task_views::Owner as TaskViewOwner;
use atlas_acta_postgres::entities::automation_rule::automation_rules;
use atlas_acta_postgres::entities::integration_config::integration_configs;
use atlas_acta_postgres::entities::lifecycle::{purge_operation, purge_operation_digest};
use atlas_acta_postgres::entities::saved_searches::{
    owner_from_columns, saved_search, saved_search_from,
};
use atlas_acta_postgres::entities::task_views::{
    owner_from_columns as task_view_owner_from_columns, task_view, task_view_from,
};
use atlas_acta_postgres::entities::webhook_delivery::webhook_delivery_log;
use atlas_acta_postgres::entities::webhook_subscription::webhook_subscriptions;
use atlas_core::principal::{ApiKeyId, UserId};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn automation_rule_model_preserves_core_columns() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_by_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = automation_rules::Model {
        id,
        workspace_id,
        name: "auto-close stale tasks".to_string(),
        is_active: true,
        trigger_event_type: "task.status_changed".to_string(),
        trigger_filter: Some(serde_json::json!({"status": "done"})),
        project_id: None,
        action_type: "notify".to_string(),
        action_params: serde_json::json!({"channel": "email"}),
        created_by_user_id,
        created_at,
        updated_at,
        deleted_at: None,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.name, "auto-close stale tasks");
    assert!(model.is_active);
    assert_eq!(model.trigger_event_type, "task.status_changed");
    assert_eq!(model.created_by_user_id, created_by_user_id);
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}

#[test]
fn integration_config_model_preserves_core_columns() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let integration_api_key_id = Uuid::now_v7();
    let created_by_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = integration_configs::Model {
        id,
        workspace_id,
        integration: "slack".to_string(),
        encrypted_secret: vec![1, 2, 3],
        secret_nonce: vec![4, 5, 6],
        integration_api_key_id,
        is_active: true,
        created_by_user_id,
        created_at,
        updated_at,
        deleted_at: None,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.integration, "slack");
    assert_eq!(model.integration_api_key_id, integration_api_key_id);
    assert_eq!(model.created_by_user_id, created_by_user_id);
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}

#[test]
fn purge_operation_model_preserves_core_columns() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let target_id = Uuid::now_v7();
    let original_actor_user_id = Uuid::now_v7();
    let commit_audit_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = purge_operation::Model {
        id,
        workspace_id,
        target_kind: "document".to_string(),
        target_id,
        original_actor_user_id,
        commit_audit_id,
        status: "pending".to_string(),
        attempts: 0,
        last_action: "enqueue".to_string(),
        last_executor_type: "system".to_string(),
        last_executor_id: None,
        last_error: None,
        last_attempt_at: None,
        created_at,
        updated_at,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.target_kind, "document");
    assert_eq!(model.target_id, target_id);
    assert_eq!(model.original_actor_user_id, original_actor_user_id);
    assert_eq!(model.commit_audit_id, commit_audit_id);
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}

#[test]
fn purge_operation_digest_model_preserves_core_columns() {
    let operation_id = Uuid::now_v7();

    let model = purge_operation_digest::Model {
        operation_id,
        digest: "sha256:abc".to_string(),
        status: "pending".to_string(),
        attempts: 0,
        last_error: None,
        last_attempt_at: None,
    };

    assert_eq!(model.operation_id, operation_id);
    assert_eq!(model.digest, "sha256:abc");
    assert_eq!(model.status, "pending");
}

#[test]
fn saved_search_from_round_trips_user_owner() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let owner_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = saved_search::Model {
        id,
        workspace_id,
        name: "my open tasks".to_string(),
        query: "status:open assignee:me".to_string(),
        owner_user_id: Some(owner_user_id),
        owner_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = saved_search_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.name, "my open tasks");
    assert_eq!(domain.query, "status:open assignee:me");
    assert_eq!(domain.owner, SavedSearchOwner::User(UserId(owner_user_id)));
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn saved_search_owner_from_columns_resolves_api_key_owner() {
    let api_key_id = Uuid::now_v7();

    let owner = owner_from_columns(None, Some(api_key_id));

    assert_eq!(owner, SavedSearchOwner::ApiKey(ApiKeyId(api_key_id)));
}

#[test]
fn task_view_from_round_trips_filters_and_owner() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let owner_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = task_view::Model {
        id,
        workspace_id,
        name: "my board view".to_string(),
        filters: serde_json::json!({"status": ["open", "in_progress"]}),
        owner_user_id: Some(owner_user_id),
        owner_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = task_view_from(model).expect("filters JSON must deserialize");

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.name, "my board view");
    assert_eq!(domain.owner, TaskViewOwner::User(UserId(owner_user_id)));
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn task_view_owner_from_columns_resolves_api_key_owner() {
    let api_key_id = Uuid::now_v7();

    let owner = task_view_owner_from_columns(None, Some(api_key_id));

    assert_eq!(owner, TaskViewOwner::ApiKey(ApiKeyId(api_key_id)));
}

#[test]
fn webhook_delivery_log_model_preserves_core_columns() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let subscription_id = Uuid::now_v7();
    let outbox_event_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = webhook_delivery_log::Model {
        id,
        workspace_id,
        subscription_id,
        outbox_event_id,
        attempt_no: 1,
        outcome: "success".to_string(),
        status_code: Some(200),
        response_snippet: Some("ok".to_string()),
        error: None,
        duration_ms: Some(42),
        created_at,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.subscription_id, subscription_id);
    assert_eq!(model.outbox_event_id, outbox_event_id);
    assert_eq!(model.outcome, "success");
    assert_eq!(model.status_code, Some(200));
    assert_eq!(model.created_at, created_at);
}

#[test]
fn webhook_subscription_model_preserves_core_columns() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_by_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = webhook_subscriptions::Model {
        id,
        workspace_id,
        target_url: "https://example.com/hook".to_string(),
        event_types: vec!["task.created".to_string()],
        scope_type: "workspace".to_string(),
        scope_id: None,
        encrypted_secret: vec![7, 8, 9],
        secret_nonce: vec![10, 11, 12],
        is_active: true,
        label: Some("primary".to_string()),
        created_by_user_id: Some(created_by_user_id),
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.target_url, "https://example.com/hook");
    assert_eq!(model.event_types, vec!["task.created".to_string()]);
    assert_eq!(model.created_by_user_id, Some(created_by_user_id));
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}
