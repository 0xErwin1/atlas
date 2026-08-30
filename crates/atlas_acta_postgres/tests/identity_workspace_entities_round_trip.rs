#![allow(clippy::expect_used)]

//! Characterization tests for the `workspace` and `workspace_membership`
//! entity conversions, ported from `atlas_server::persistence::entities::identity`
//! before that module moves into this crate (S4 PR1, T1.2). Pinning these
//! conversions here first proves the move is verbatim: the assertions below
//! must keep passing, unmodified, once `entities::identity` lands (T1.6).

use atlas_acta::entities::identity::MemberRole;
use atlas_acta_postgres::entities::identity::{
    membership, membership_from, workspace, workspace_from,
};
use chrono::Utc;
use uuid::Uuid;

#[test]
fn workspace_from_round_trips_every_column() {
    let id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = workspace::Model {
        id,
        name: "Atlas".to_string(),
        slug: "atlas".to_string(),
        created_at,
        updated_at,
        deleted_at: None,
    };

    let domain = workspace_from(model);

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.name, "Atlas");
    assert_eq!(domain.slug, "atlas");
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn membership_from_round_trips_every_column_and_known_role() {
    let id = Uuid::now_v7();
    let workspace_id = Uuid::now_v7();
    let user_id = Uuid::now_v7();
    let created_at = Utc::now();
    let updated_at = Utc::now();

    let model = membership::Model {
        id,
        workspace_id,
        user_id,
        role: "admin".to_string(),
        created_at,
        updated_at,
    };

    let domain = membership_from(model).expect("known role must parse");

    assert_eq!(domain.id.0, id);
    assert_eq!(domain.workspace_id.0, workspace_id);
    assert_eq!(domain.user_id.0, user_id);
    assert!(matches!(domain.role, MemberRole::Admin));
    assert_eq!(domain.created_at, created_at);
    assert_eq!(domain.updated_at, updated_at);
}

#[test]
fn membership_from_rejects_unknown_role() {
    let model = membership::Model {
        id: Uuid::now_v7(),
        workspace_id: Uuid::now_v7(),
        user_id: Uuid::now_v7(),
        role: "not-a-role".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    assert!(membership_from(model).is_err());
}
