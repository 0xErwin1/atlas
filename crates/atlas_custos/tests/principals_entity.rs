//! Unit tests for the pure `Principal` record, `PrincipalKind` enum and
//! `PrincipalId` conversions introduced by E4-S1. No database involved.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::str::FromStr;

use atlas_custos::entities::principals::{NewPrincipal, Principal, PrincipalKind};
use atlas_custos::ids::{PrincipalId, UserId};

#[test]
fn principal_kind_round_trips_through_as_str_and_from_str() {
    for (kind, text) in [
        (PrincipalKind::User, "user"),
        (PrincipalKind::Agent, "agent"),
    ] {
        assert_eq!(kind.as_str(), text);
        assert_eq!(PrincipalKind::from_str(text).unwrap(), kind);
    }
}

#[test]
fn principal_kind_rejects_unknown_text() {
    let err = PrincipalKind::from_str("cli").expect_err("cli is not a principal kind");
    assert!(err.contains("unknown principal kind"), "got: {err}");
}

#[test]
fn principal_kind_serializes_lowercase() {
    assert_eq!(serde_json::to_value(PrincipalKind::User).unwrap(), "user");
    assert_eq!(serde_json::to_value(PrincipalKind::Agent).unwrap(), "agent");
    assert_eq!(
        serde_json::from_value::<PrincipalKind>(serde_json::json!("agent")).unwrap(),
        PrincipalKind::Agent
    );
}

#[test]
fn user_id_converts_into_principal_id() {
    let user_id = UserId::new();
    let principal_id = PrincipalId::from(user_id);
    assert_eq!(principal_id.0, user_id.0);
}

#[test]
fn principal_record_carries_the_shared_identity_fields() {
    let created_at = chrono::Utc::now();
    let principal = Principal {
        id: PrincipalId::new(),
        kind: PrincipalKind::User,
        display_name: "Ada".to_string(),
        deactivated_at: None,
        created_at,
        updated_at: created_at,
    };
    assert_eq!(principal.display_name, "Ada");
    assert_eq!(principal.kind, PrincipalKind::User);
    assert_eq!(principal.deactivated_at, None);

    let new = NewPrincipal {
        id: PrincipalId::new(),
        kind: PrincipalKind::Agent,
        display_name: "ci-bot".to_string(),
        deactivated_at: None,
    };
    assert_eq!(new.kind, PrincipalKind::Agent);
}
