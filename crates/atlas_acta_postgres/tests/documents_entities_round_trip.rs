#![allow(clippy::expect_used)]

//! Characterization tests for the documents-family entity conversions,
//! ported from `atlas_server::persistence::entities::{workspace_core,
//! documents, comments}` before the ten documents-family tables move into
//! this crate (S4 PR2, T2.1). Pinning these conversions here first proves
//! the move is verbatim: the assertions below must keep passing, unmodified,
//! once `entities::documents` lands (T2.4).
//!
//! `property_definitions`, `projects`, and `folders` already moved into
//! `atlas_acta_postgres::entities::workspace_core` in PR1, but PR1 did not
//! add round-trip coverage for them (see PR1's deviation note); T2.1 closes
//! that gap here alongside the seven tables new to this PR.
//!
//! `comment_attachment_drafts`/`comment_attachment_draft_uploads`' domain
//! conversion functions (`comment_attachment_draft_from`,
//! `comment_attachment_draft_upload_from`) stay in `atlas_server` for this PR
//! because they call `actor_from_columns`, which lives in
//! `atlas_server::persistence::entities::boards_tasks` and only moves in PR3.
//! Only the two tables' sea-orm entity structs move here now; the tests below
//! for them assert the moved `Model` preserves every column, which is the
//! invariant this PR's move actually protects.

use atlas_acta_postgres::entities::documents::{
    attachment, attachment_from, attachment_write_intent, attachment_write_intent_from,
    comment_attachment_draft, comment_attachment_draft_upload, document, document_from,
    document_link, document_link_from, document_revision, document_summary_from, revision_from,
    revision_meta_from,
};
use atlas_acta_postgres::entities::workspace_core::{
    folder, folder_from, project, project_from, property_definition, property_definition_from,
};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn property_definition_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_by_user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = property_definition::Model {
        id,
        workspace_id,
        key: "status".to_string(),
        name: "Status".to_string(),
        kind: "select".to_string(),
        options: None,
        applies_to: "task".to_string(),
        created_by_user_id: Some(created_by_user_id),
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = property_definition_from(model).expect("known kind/applies_to must parse");

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.key, "status");
    assert_eq!(
        domain.created_by_user_id.map(|u| u.0),
        Some(created_by_user_id)
    );
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn project_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = project::Model {
        id,
        workspace_id,
        name: "Atlas".to_string(),
        slug: "atlas".to_string(),
        task_prefix: "ATL".to_string(),
        next_task_number: 42,
        visibility: "workspace".to_string(),
        visibility_role: Some("viewer".to_string()),
        created_by_user_id: None,
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = project_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.slug, "atlas");
    assert_eq!(domain.task_prefix, "ATL");
    assert_eq!(domain.next_task_number, 42);
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn folder_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let project_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = folder::Model {
        id,
        workspace_id,
        project_id: Some(project_id),
        parent_folder_id: None,
        name: "Docs".to_string(),
        created_by_user_id: None,
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = folder_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.project_id.map(|p| p.0), Some(project_id));
    assert_eq!(domain.name, "Docs");
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

fn base_doc_model(rev: Uuid) -> document::Model {
    document::Model {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        project_id: None,
        folder_id: None,
        title: "Test".into(),
        slug: Some("test".into()),
        content: "body".into(),
        frontmatter: serde_json::json!({}),
        current_revision_id: Some(rev),
        current_revision_seq: 1,
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        created_at: Utc::now(),
        updated_at: Utc::now(),
        deleted_at: None,
    }
}

#[test]
fn document_from_roundtrips_slug() {
    let rev = Uuid::now_v7();
    let m = base_doc_model(rev);
    let slug = m.slug.clone();

    let doc = document_from(m).expect("document_from must succeed");

    assert_eq!(doc.slug, slug);
}

#[test]
fn document_from_roundtrips_created_by_api_key_id() {
    let rev = Uuid::now_v7();
    let key_id = Uuid::now_v7();
    let mut m = base_doc_model(rev);
    m.created_by_user_id = None;
    m.created_by_api_key_id = Some(key_id);

    let doc = document_from(m).expect("document_from must succeed");

    assert_eq!(doc.created_by_api_key_id.map(|k| k.0), Some(key_id));
    assert!(doc.created_by_user_id.is_none());
}

#[test]
fn document_summary_from_round_trips_every_column() {
    let rev = Uuid::now_v7();
    let m = base_doc_model(rev);
    let title = m.title.clone();

    let summary = document_summary_from(m).expect("document_summary_from must succeed");

    assert_eq!(summary.title, title);
    assert_eq!(summary.current_revision_id.0, rev);
}

#[test]
fn revision_meta_from_carries_actor_ids() {
    let key_uuid = Uuid::now_v7();
    let rev_model = document_revision::Model {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        document_id: Uuid::now_v7(),
        seq: 3,
        patch: Some("patch".into()),
        snapshot: None,
        is_anchor: false,
        created_by_user_id: None,
        created_by_api_key_id: Some(key_uuid),
        created_at: Utc::now(),
    };

    let meta = revision_meta_from(rev_model.clone());

    assert_eq!(meta.id.0, rev_model.id);
    assert_eq!(meta.seq, 3);
    assert_eq!(meta.created_by_api_key_id.map(|k| k.0), Some(key_uuid));
    assert!(meta.created_by_user_id.is_none());
}

#[test]
fn revision_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let document_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = document_revision::Model {
        id,
        workspace_id,
        document_id,
        seq: 7,
        patch: None,
        snapshot: Some("snapshot".into()),
        is_anchor: true,
        created_by_user_id: None,
        created_by_api_key_id: None,
        created_at,
    };

    let domain = revision_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.document_id.0, document_id);
    assert_eq!(domain.seq, 7);
    assert!(domain.is_anchor);
    assert_eq!(domain.created_at, created_at);
}

#[test]
fn document_link_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let target_document_id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = document_link::Model {
        id,
        workspace_id,
        source_document_id: None,
        source_task_id: Some(Uuid::now_v7()),
        target_document_id: Some(target_document_id),
        target_task_id: None,
        target_attachment_id: None,
        target_title: "Linked doc".to_string(),
        created_at,
    };

    let domain = document_link_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(
        domain.target_document_id.map(|d| d.0),
        Some(target_document_id)
    );
    assert_eq!(domain.target_title, "Linked doc");
    assert_eq!(domain.created_at, created_at);
}

#[test]
fn attachment_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = attachment::Model {
        id,
        workspace_id,
        document_id: Some(Uuid::now_v7()),
        task_id: None,
        comment_id: None,
        draft_id: None,
        file_name: "spec.pdf".to_string(),
        content_type: "application/pdf".to_string(),
        size_bytes: 1024,
        sha256: "deadbeef".to_string(),
        created_by_user_id: None,
        created_by_api_key_id: None,
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = attachment_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.file_name, "spec.pdf");
    assert_eq!(domain.size_bytes, 1024);
    assert_eq!(domain.sha256, "deadbeef");
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn attachment_write_intent_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let created_at = Utc::now();

    let model = attachment_write_intent::Model {
        id,
        digest: "digest-value".to_string(),
        created_at,
    };

    let domain = attachment_write_intent_from(model);

    assert_eq!(domain.id, id);
    assert_eq!(domain.digest, "digest-value");
    assert_eq!(domain.created_at, created_at);
}

#[test]
fn comment_attachment_draft_model_preserves_every_column() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let expires_at = Utc::now();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = comment_attachment_draft::Model {
        id,
        workspace_id,
        task_id: Some(Uuid::now_v7()),
        document_id: None,
        created_by_user_id: Some(Uuid::now_v7()),
        created_by_api_key_id: None,
        create_token: "token".to_string(),
        create_digest: vec![1, 2, 3],
        state: "active".to_string(),
        finalized_comment_id: None,
        final_body_digest: None,
        final_request_digest: None,
        expires_at,
        terminal_at: None,
        created_at,
        updated_at,
    };

    assert_eq!(model.id, id);
    assert_eq!(model.workspace_id, workspace_id);
    assert_eq!(model.create_token, "token");
    assert_eq!(model.create_digest, vec![1, 2, 3]);
    assert_eq!(model.state, "active");
    assert_eq!(model.expires_at, expires_at);
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}

#[test]
fn comment_attachment_draft_upload_model_preserves_every_column() {
    let draft_id = Uuid::now_v7();
    let original_attachment_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = comment_attachment_draft_upload::Model {
        draft_id,
        upload_token: "upload-token".to_string(),
        original_attachment_id,
        attachment_id: None,
        request_digest: vec![4, 5, 6],
        payload_digest: vec![7, 8, 9],
        file_name: "diagram.png".to_string(),
        content_type: "image/png".to_string(),
        size_bytes: 2048,
        deleted_at: None,
        created_at,
        updated_at,
    };

    assert_eq!(model.draft_id, draft_id);
    assert_eq!(model.original_attachment_id, original_attachment_id);
    assert_eq!(model.upload_token, "upload-token");
    assert_eq!(model.request_digest, vec![4, 5, 6]);
    assert_eq!(model.payload_digest, vec![7, 8, 9]);
    assert_eq!(model.file_name, "diagram.png");
    assert_eq!(model.size_bytes, 2048);
    assert_eq!(model.created_at, created_at);
    assert_eq!(model.updated_at, updated_at);
}
