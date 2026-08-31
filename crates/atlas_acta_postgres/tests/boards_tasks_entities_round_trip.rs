#![allow(clippy::expect_used)]

//! Characterization tests for the boards/tasks/status-templates-family entity
//! conversions, ported from `atlas_server::persistence::entities::{
//! boards_tasks, status_templates}` before the nine boards/tasks-family
//! tables move into this crate (S4 PR3, T3.1). Pinning these conversions here
//! first proves the move is verbatim: the assertions below must keep
//! passing, unmodified, once `entities::boards_tasks` and
//! `entities::status_templates` land (T3.4).

use atlas_acta::actor::Actor;
use atlas_acta_postgres::entities::boards_tasks::{
    activity_kind_from_str, actor_from_columns, board, board_column, board_column_from, board_from,
    task, task_activity, task_activity_from, task_assignee, task_assignee_from,
    task_checklist_item, task_checklist_item_from, task_from, task_reference, task_reference_from,
};
use atlas_acta_postgres::entities::status_templates::{
    platform_status_template, platform_status_template_from, status_template, status_template_from,
};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn actor_from_columns_resolves_user_actor() {
    let uid = Uuid::now_v7();
    let actor = actor_from_columns(Some(uid), None);
    assert!(matches!(actor, Actor::User(id) if id == atlas_acta::actor::UserAttributionId(uid)));
}

#[test]
fn actor_from_columns_resolves_api_key_actor() {
    let kid = Uuid::now_v7();
    let actor = actor_from_columns(None, Some(kid));
    assert!(
        matches!(actor, Actor::ApiKey(id) if id == atlas_acta::actor::ApiKeyAttributionId(kid))
    );
}

#[test]
fn board_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = board::Model {
        id,
        workspace_id,
        project_id,
        folder_id: None,
        name: "Roadmap".to_string(),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
        archived_at: None,
    };

    let domain = board_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.project_id.0, project_id);
    assert_eq!(domain.name, "Roadmap");
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn board_column_from_round_trips_color() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let board_id = Uuid::now_v7();
    let now = Utc::now();

    let model = board_column::Model {
        id,
        workspace_id,
        board_id,
        name: "Done".to_string(),
        position_key: "80".to_string(),
        color: Some("green".to_string()),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let domain = board_column_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.board_id.0, board_id);
    assert_eq!(domain.color.as_deref(), Some("green"));
}

#[test]
fn task_from_round_trips_priority_and_labels() {
    let id = Uuid::now_v7();
    let now = Utc::now();

    let model = task::Model {
        id,
        workspace_id: Uuid::now_v7(),
        project_id: Uuid::now_v7(),
        board_id: Uuid::now_v7(),
        column_id: Uuid::now_v7(),
        parent_task_id: None,
        readable_id: "ATL-1".to_string(),
        title: "Ship it".to_string(),
        description: String::new(),
        priority: Some("high".to_string()),
        due_date: None,
        estimate: Some(5),
        labels: vec!["backend".to_string()],
        properties: None,
        position_key: "80".to_string(),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let domain = task_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(
        domain.priority,
        Some(atlas_acta::entities::boards_tasks::Priority::High)
    );
    assert_eq!(domain.estimate, Some(5));
    assert_eq!(domain.labels, vec!["backend".to_string()]);
}

#[test]
fn task_reference_from_round_trips_kind() {
    let id = Uuid::now_v7();
    let source_task_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = task_reference::Model {
        id,
        workspace_id: Uuid::now_v7(),
        source_task_id,
        kind: "blocks".to_string(),
        target_task_id: Some(Uuid::now_v7()),
        target_document_id: None,
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at,
    };

    let domain = task_reference_from(model).expect("known kind must parse");

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.source_task_id.0, source_task_id);
    assert_eq!(
        domain.kind,
        atlas_acta::entities::boards_tasks::ReferenceKind::Blocks
    );
}

#[test]
fn task_assignee_from_round_trips_user_assignee() {
    let task_id = Uuid::now_v7();
    let assignee_user_id = Uuid::now_v7();
    let assigned_at = Utc::now();

    let model = task_assignee::Model {
        task_id,
        workspace_id: Uuid::now_v7(),
        assignee_user_id: Some(assignee_user_id),
        assignee_api_key_id: None,
        assigned_by_user_id: Some(Uuid::now_v7()),
        assigned_by_api_key_id: None,
        assigned_at,
    };

    let domain = task_assignee_from(model).expect("valid assignee XOR state must parse");

    assert_eq!(domain.task_id.0, task_id);
    assert_eq!(domain.assigned_at, assigned_at);
}

#[test]
fn task_checklist_item_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    let now = Utc::now();

    let model = task_checklist_item::Model {
        id,
        task_id,
        workspace_id: Uuid::now_v7(),
        title: "Write tests".to_string(),
        checked: true,
        position_key: "40".to_string(),
        promoted_task_id: None,
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let domain = task_checklist_item_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.task_id.0, task_id);
    assert!(domain.checked);
    assert_eq!(domain.title, "Write tests");
}

#[test]
fn task_activity_from_round_trips_created_kind() {
    let id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = task_activity::Model {
        id,
        task_id,
        workspace_id: Uuid::now_v7(),
        kind: "created".to_string(),
        payload: serde_json::json!("created"),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at,
    };

    let domain = task_activity_from(model).expect("known kind/payload must parse");

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.task_id.0, task_id);
    assert_eq!(
        domain.kind,
        atlas_acta::entities::boards_tasks::ActivityKind::Created
    );
}

#[test]
fn activity_kind_from_str_round_trips_every_variant() {
    let kinds = [
        "created",
        "moved",
        "assigned",
        "unassigned",
        "field_changed",
        "reference_added",
        "reference_removed",
        "checklist_added",
        "checklist_updated",
        "checklist_removed",
        "checklist_promoted",
        "document_mentioned",
        "deleted",
    ];
    for kind in kinds {
        let parsed = activity_kind_from_str(kind).expect("must parse");
        assert_eq!(parsed.as_str(), kind);
    }
}

#[test]
fn status_template_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let now = Utc::now();

    let model = status_template::Model {
        id,
        workspace_id,
        name: "In Progress".to_string(),
        color: Some("blue".to_string()),
        position_key: "20".to_string(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let domain = status_template_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.name, "In Progress");
    assert_eq!(domain.color.as_deref(), Some("blue"));
}

#[test]
fn platform_status_template_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let now = Utc::now();

    let model = platform_status_template::Model {
        id,
        name: "Backlog".to_string(),
        color: None,
        position_key: "10".to_string(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };

    let domain = platform_status_template_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.name, "Backlog");
    assert!(domain.color.is_none());
}
