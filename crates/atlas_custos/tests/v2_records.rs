//! Public-surface behavioral tests for the conversions between stored
//! authorization records and the evaluator model (E5 S5a): subjects and
//! targets round-trip, stored grants resolve through the catalog, and
//! stored deny rules become evaluator deny rules.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::ids::{ActionId, PrincipalSetId};
use atlas_custos::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantAuthority, GrantId, GrantRecord, RoleId,
    SubjectRecord, TargetRecord,
};
use atlas_custos::eval::{
    CatalogError, DenyRule, EvalError, GrantTarget, RoleRef, Subject, grant_spec,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use chrono::Utc;
use support::{CUSTOS_READ, READ, UPDATE, action, actions, catalog};

const GRANT_CREATE: &str = "custos::grant::create";

fn subjects() -> Vec<SubjectRecord> {
    vec![
        SubjectRecord::Principal(PrincipalId::new()),
        SubjectRecord::Group(GroupId::new()),
        SubjectRecord::PrincipalSet("acta::workspace::w1::members".parse().unwrap()),
    ]
}

fn targets() -> Vec<TargetRecord> {
    vec![
        TargetRecord::Ref("acta::document::d1".parse().unwrap()),
        TargetRecord::Path(
            "acta::workspace::w1/folder::f1/document::d1"
                .parse()
                .unwrap(),
        ),
        TargetRecord::Selector("acta::workspace::w1/**".parse().unwrap()),
    ]
}

fn stored_grant(
    subject: SubjectRecord,
    target: TargetRecord,
    authority: GrantAuthority,
) -> GrantRecord {
    GrantRecord {
        id: GrantId::new(),
        subject,
        target,
        authority,
        created_by: PrincipalId::new(),
        created_at: Utc::now(),
    }
}

fn stored_role(granted: &[&str]) -> CustomRole {
    CustomRole {
        id: RoleId::new(),
        product: "acta".to_string(),
        name: "reviewer".to_string(),
        actions: granted.iter().map(|raw| action(raw)).collect(),
        created_by: PrincipalId::new(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    }
}

fn stored_deny(subject: SubjectRecord, target: TargetRecord, denied: &[&str]) -> DenyRecord {
    DenyRecord {
        id: DenyRuleId::new(),
        subject,
        target,
        actions: denied.iter().map(|raw| action(raw)).collect(),
        created_by: PrincipalId::new(),
        created_at: Utc::now(),
    }
}

fn builtin(name: &str, version: u32) -> GrantAuthority {
    GrantAuthority::Builtin {
        name: name.to_string(),
        version,
    }
}

fn explicit(granted: &[&str]) -> GrantAuthority {
    GrantAuthority::Actions(
        granted
            .iter()
            .map(|raw| action(raw))
            .collect::<Vec<ActionId>>(),
    )
}

#[test]
fn every_subject_shape_round_trips_through_the_evaluator_subject() {
    for record in subjects() {
        let subject = Subject::from(&record);

        match (&record, &subject) {
            (SubjectRecord::Principal(stored), Subject::Principal(converted)) => {
                assert_eq!(stored, converted)
            }
            (SubjectRecord::Group(stored), Subject::Group(converted)) => {
                assert_eq!(stored, converted)
            }
            (SubjectRecord::PrincipalSet(stored), Subject::PrincipalSet(converted)) => {
                assert_eq!(stored, converted)
            }
            other => panic!("subject shape changed: {other:?}"),
        }

        assert_eq!(SubjectRecord::from(&subject), record);
    }
}

#[test]
fn every_target_shape_round_trips_through_the_evaluator_target() {
    for record in targets() {
        let target = GrantTarget::from(&record);

        match (&record, &target) {
            (TargetRecord::Ref(stored), GrantTarget::Ref(converted)) => {
                assert_eq!(stored, converted)
            }
            (TargetRecord::Path(stored), GrantTarget::Path(converted)) => {
                assert_eq!(stored, converted)
            }
            (TargetRecord::Selector(stored), GrantTarget::Selector(converted)) => {
                assert_eq!(stored, converted)
            }
            other => panic!("target shape changed: {other:?}"),
        }

        assert_eq!(TargetRecord::from(&target), record);
        assert_eq!(target.product(), record.product());
    }
}

#[test]
fn a_stored_grant_carries_its_subject_and_target_into_the_spec() {
    let catalog = catalog();

    for subject in subjects() {
        for target in targets() {
            let record = stored_grant(subject.clone(), target.clone(), builtin("viewer", 1));

            let spec = grant_spec(&record, &catalog, &[]).unwrap();

            assert_eq!(spec.target, Some(GrantTarget::from(&target)));
            match &subject {
                SubjectRecord::Principal(id) => {
                    assert_eq!(spec.principal, Some(*id));
                    assert!(spec.group.is_none() && spec.principal_set.is_none());
                }
                SubjectRecord::Group(id) => {
                    assert_eq!(spec.group, Some(*id));
                    assert!(spec.principal.is_none() && spec.principal_set.is_none());
                }
                SubjectRecord::PrincipalSet(set) => {
                    assert_eq!(spec.principal_set.as_ref(), Some(set));
                    assert!(spec.principal.is_none() && spec.group.is_none());
                }
            }

            let grant = catalog.resolve_grant(spec).unwrap();

            assert_eq!(SubjectRecord::from(grant.subject()), subject);
            assert_eq!(TargetRecord::from(grant.target()), target);
        }
    }
}

#[test]
fn a_builtin_authority_names_the_role_of_the_target_product() {
    let catalog = catalog();
    let record = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        builtin("editor", 2),
    );

    let spec = grant_spec(&record, &catalog, &[]).unwrap();

    assert_eq!(
        spec.builtin_role,
        Some(RoleRef {
            product: "acta".to_string(),
            name: "editor".to_string(),
            version: 2,
        })
    );
    assert!(spec.custom_role.is_none() && spec.actions.is_none());
    assert_eq!(
        catalog.resolve_grant(spec).unwrap().actions(),
        catalog.builtin_role("acta", "editor", 2).unwrap().actions()
    );
}

#[test]
fn a_custom_role_authority_is_rebuilt_from_its_stored_row() {
    let catalog = catalog();
    let role = stored_role(&[READ, UPDATE]);
    let unrelated = stored_role(&[READ]);
    let record = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        GrantAuthority::CustomRole(role.id),
    );

    let spec = grant_spec(&record, &catalog, &[unrelated, role]).unwrap();

    assert!(spec.builtin_role.is_none() && spec.actions.is_none());
    assert_eq!(
        catalog.resolve_grant(spec).unwrap().actions(),
        &actions(&[READ, UPDATE])
    );
}

#[test]
fn a_custom_role_authority_without_its_row_or_with_custos_actions_is_rejected() {
    let catalog = catalog();
    let missing = RoleId::new();
    let tampered = stored_role(&[READ, GRANT_CREATE]);

    let without_row = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        GrantAuthority::CustomRole(missing),
    );
    let with_custos = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        GrantAuthority::CustomRole(tampered.id),
    );

    assert_eq!(
        grant_spec(&without_row, &catalog, &[]),
        Err(CatalogError::UnknownCustomRole { id: missing })
    );
    assert_eq!(
        grant_spec(&with_custos, &catalog, &[tampered]),
        Err(CatalogError::CustosActionInCustomRole {
            action: action(GRANT_CREATE)
        })
    );
}

#[test]
fn an_actions_authority_becomes_an_explicit_action_set() {
    let catalog = catalog();
    let record = stored_grant(
        subjects().remove(0),
        TargetRecord::Ref("acta::workspace::w1".parse().unwrap()),
        explicit(&[READ, GRANT_CREATE]),
    );
    let mixed = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        explicit(&[READ, "virtus::task::read"]),
    );

    let spec = grant_spec(&record, &catalog, &[]).unwrap();

    assert_eq!(spec.actions, Some(actions(&[READ, GRANT_CREATE])));
    assert!(spec.builtin_role.is_none() && spec.custom_role.is_none());
    assert!(catalog.resolve_grant(spec).is_ok());
    assert_eq!(
        grant_spec(&mixed, &catalog, &[]),
        Err(CatalogError::MixedProducts {
            first: "acta".to_string(),
            second: "virtus".to_string(),
        })
    );
}

#[test]
fn a_stored_deny_becomes_an_evaluator_deny_rule() {
    for subject in subjects() {
        for target in targets() {
            let record = stored_deny(subject.clone(), target.clone(), &[READ, GRANT_CREATE]);

            let rule = DenyRule::try_from(&record).unwrap();

            assert_eq!(SubjectRecord::from(rule.subject()), subject);
            assert_eq!(TargetRecord::from(rule.target()), target);
            assert_eq!(rule.actions(), &actions(&[READ, GRANT_CREATE]));
        }
    }
}

#[test]
fn a_stored_deny_with_actions_of_another_product_is_rejected() {
    let set: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();
    let foreign = stored_deny(
        SubjectRecord::PrincipalSet(set),
        targets().remove(0),
        &[CUSTOS_READ],
    );
    let mixed = stored_deny(
        SubjectRecord::Group(GroupId::new()),
        targets().remove(0),
        &[READ, "virtus::task::read"],
    );

    assert!(matches!(
        DenyRule::try_from(&foreign),
        Err(EvalError::CrossProductFact { .. })
    ));
    assert!(matches!(
        DenyRule::try_from(&mixed),
        Err(EvalError::CrossProductActions { .. })
    ));
}

#[test]
fn a_custom_role_stored_for_another_product_than_its_actions_is_rejected() {
    let catalog = catalog();
    let mut role = stored_role(&[READ, UPDATE]);
    role.product = "virtus".to_string();
    let record = stored_grant(
        subjects().remove(0),
        targets().remove(0),
        GrantAuthority::CustomRole(role.id),
    );

    assert_eq!(
        grant_spec(&record, &catalog, std::slice::from_ref(&role)),
        Err(CatalogError::CustomRoleProductMismatch {
            id: role.id,
            stored_product: "virtus".to_string(),
            actions_product: "acta".to_string(),
        })
    );
}
