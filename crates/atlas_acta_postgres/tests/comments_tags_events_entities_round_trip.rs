#![allow(clippy::expect_used)]

//! Characterization tests for the comments/tags/events-outbox entity
//! conversions, ported from `atlas_server::persistence::entities::{comments,
//! tags, events_outbox}` before those five tables move into this crate (S4
//! PR4, T4.1). Pinning these conversions here first proves the move is
//! verbatim: the assertions below must keep passing, unmodified, once
//! `entities::comments`, `entities::tags`, and `entities::events_outbox` land
//! (T4.4).
//!
//! `comment_link_event` and `events_outbox` have no domain `_from` conversion
//! function today (their repos read the sea-orm `Model` fields directly);
//! their tests below pin the moved `Model`'s column set the same way PR2's
//! `comment_attachment_draft_model_preserves_every_column` did for a
//! conversion-less entity.

use atlas_acta::entities::comments::CommentLinkTarget;
use atlas_acta_postgres::entities::comments::{
    comment, comment_from, comment_link, comment_link_event, comment_link_from,
};
use atlas_acta_postgres::entities::events_outbox::event_outbox;
use atlas_acta_postgres::entities::tags::{tag, tag_from};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn comment_from_round_trips_task_owner_and_body() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let task_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = comment::Model {
        id,
        workspace_id,
        task_id: Some(task_id),
        document_id: None,
        body: "hello".to_string(),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = comment_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.task_id.map(|id| id.0), Some(task_id));
    assert!(domain.document_id.is_none());
    assert_eq!(domain.body, "hello");
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn comment_from_resolves_api_key_author() {
    let key_id = Uuid::now_v7();

    let model = comment::Model {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        task_id: None,
        document_id: Some(Uuid::now_v7()),
        body: "from an api key".to_string(),
        created_by_user_id: None,
        created_by_api_key_id: Some(key_id),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    };

    let domain = comment_from(model);

    assert_eq!(
        domain.created_by,
        atlas_acta::actor::Actor::ApiKey(atlas_acta::actor::ApiKeyAttributionId(key_id))
    );
}

#[test]
fn comment_link_from_round_trips_document_target() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let comment_id = Uuid::now_v7();
    let target_document_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = comment_link::Model {
        id,
        workspace_id,
        comment_id,
        target_document_id: Some(target_document_id),
        target_task_id: None,
        target_attachment_id: None,
        created_at,
    };

    let domain = comment_link_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.comment_id.0, comment_id);
    assert_eq!(
        domain.target,
        CommentLinkTarget::Document(atlas_acta::ids::DocumentId(target_document_id))
    );
    assert_eq!(domain.created_at, created_at);
}

#[test]
fn comment_link_event_model_preserves_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let comment_id = Uuid::now_v7();
    let actor_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = comment_link_event::Model {
        id,
        workspace_id,
        parent_task_id: Some(Uuid::now_v7()),
        parent_document_id: None,
        comment_id,
        event_kind: "linked".to_string(),
        target_document_id: Some(Uuid::now_v7()),
        target_task_id: None,
        target_attachment_id: None,
        actor_type: "user".to_string(),
        actor_id,
        created_at,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.comment_id, comment_id);
    assert_eq!(model.event_kind, "linked");
    assert_eq!(model.actor_type, "user");
    assert_eq!(model.actor_id, actor_id);
    assert_eq!(model.created_at, created_at);
}

#[test]
fn tag_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = tag::Model {
        id,
        workspace_id,
        name: "urgent".to_string(),
        color: Some("red".to_string()),
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = tag_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.name, "urgent");
    assert_eq!(domain.color.as_deref(), Some("red"));
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn event_outbox_model_preserves_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let aggregate_id = Uuid::now_v7();
    let occurred_at = Utc::now();
    let next_attempt_at = Utc::now();

    let model = event_outbox::Model {
        id,
        workspace_id,
        event_type: "task.created".to_string(),
        event_version: 1,
        source: "atlas_server".to_string(),
        project_id: None,
        board_id: None,
        aggregate_type: "task".to_string(),
        aggregate_id,
        payload: serde_json::json!({"foo": "bar"}),
        occurred_at,
        status: "pending".to_string(),
        attempt_count: 0,
        next_attempt_at,
        locked_until: None,
        last_error: None,
        created_at: occurred_at,
        updated_at: occurred_at,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.event_type, "task.created");
    assert_eq!(model.aggregate_id, aggregate_id);
    assert_eq!(model.occurred_at, occurred_at);
    assert_eq!(model.next_attempt_at, next_attempt_at);
}
