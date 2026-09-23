//! Public-surface behavioral tests for the pure authorization catalog and
//! grant validation (GRANT-1/2/5): target validation, product-scoped custom
//! roles and grant spec resolution into evaluator grants.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::ids::PrincipalSetId;
use atlas_custos::eval::{Catalog, CatalogError, GrantSpec, GrantTarget, ProductSpec, Subject};
use atlas_custos::ids::{GroupId, PrincipalId};
use support::{
    CUSTOS_READ, READ, UPDATE, acta, action, actions, catalog, custos, path_target, ref_target,
    role, role_ref, selector_target,
};

fn spec_for(target: GrantTarget) -> GrantSpec {
    GrantSpec {
        principal: Some(PrincipalId::new()),
        target: Some(target),
        builtin_role: Some(role_ref("acta", "viewer", 1)),
        ..GrantSpec::default()
    }
}

fn invalid_catalog(products: Vec<ProductSpec>) -> bool {
    matches!(
        Catalog::new(products),
        Err(CatalogError::InvalidCatalog { .. })
    )
}

#[test]
fn a_catalog_rejects_inconsistent_plain_data() {
    let mut foreign_action = acta();
    foreign_action.actions.push(action(CUSTOS_READ));

    let mut undeclared_kind = acta();
    undeclared_kind.actions.push(action("acta::board::read"));

    let mut undeclared_role_action = acta();
    undeclared_role_action
        .roles
        .push(role("admin", 1, &["acta::document::delete"]));

    let mut empty_role = acta();
    empty_role.roles.push(role("nobody", 1, &[]));

    let mut duplicate_version = acta();
    duplicate_version.roles.push(role("viewer", 1, &[READ]));

    assert!(invalid_catalog(vec![foreign_action]));
    assert!(invalid_catalog(vec![undeclared_kind]));
    assert!(invalid_catalog(vec![undeclared_role_action]));
    assert!(invalid_catalog(vec![empty_role]));
    assert!(invalid_catalog(vec![duplicate_version]));
    assert!(invalid_catalog(vec![acta(), acta()]));
}

#[test]
fn builtin_roles_are_looked_up_by_product_name_and_version() {
    let catalog = catalog();

    let editor_v1 = catalog.builtin_role("acta", "editor", 1).unwrap();
    let editor_v2 = catalog.builtin_role("acta", "editor", 2).unwrap();

    assert_eq!(editor_v1.version(), 1);
    assert_eq!(editor_v1.product(), "acta");
    assert_eq!(editor_v1.name(), "editor");
    assert!(!editor_v1.actions().contains(&action("acta::folder::read")));
    assert!(editor_v2.actions().contains(&action("acta::folder::read")));
    assert!(catalog.builtin_role("acta", "editor", 3).is_none());
    assert!(catalog.builtin_role("custos", "editor", 1).is_none());
}

#[test]
fn targets_of_declared_products_and_kinds_are_valid() {
    let catalog = catalog();

    for target in [
        ref_target("acta::document::d1"),
        path_target("acta::workspace::w1/folder::f1/document::d1"),
        selector_target("acta::workspace::w1/*/document::d1"),
        selector_target("acta::workspace::w1/**"),
        ref_target("custos::grant::g1"),
    ] {
        assert_eq!(catalog.validate_target(&target), Ok(()), "{target:?}");
    }
}

#[test]
fn targets_with_an_unknown_product_or_kind_are_rejected() {
    let catalog = catalog();

    assert_eq!(
        catalog.validate_target(&ref_target("vigil::document::d1")),
        Err(CatalogError::UnknownProduct {
            product: "vigil".to_string()
        })
    );

    for (target, kind) in [
        (ref_target("acta::board::b1"), "board"),
        (
            path_target("acta::workspace::w1/board::b1/document::d1"),
            "board",
        ),
        (selector_target("acta::board::b1/**"), "board"),
        (selector_target("acta::*/card::c1"), "card"),
    ] {
        assert_eq!(
            catalog.validate_target(&target),
            Err(CatalogError::UnknownKind {
                product: "acta".to_string(),
                kind: kind.to_string(),
            }),
            "{target:?}"
        );
    }
}

#[test]
fn a_custom_role_is_a_non_empty_single_product_set_of_catalog_actions() {
    let catalog = catalog();

    let role = catalog.custom_role([action(READ), action(UPDATE)]).unwrap();

    assert!(role.actions().contains(&action(READ)));
    assert!(role.actions().contains(&action(UPDATE)));
    assert_eq!(role.actions().product(), Some("acta"));
}

#[test]
fn invalid_custom_roles_are_rejected_with_typed_errors() {
    let catalog = catalog();

    assert_eq!(catalog.custom_role([]), Err(CatalogError::EmptyActions));
    assert_eq!(
        catalog.custom_role([action(READ), action(CUSTOS_READ)]),
        Err(CatalogError::MixedProducts {
            first: "acta".to_string(),
            second: "custos".to_string(),
        })
    );
    assert_eq!(
        catalog.custom_role([action(CUSTOS_READ)]),
        Err(CatalogError::CustosActionInCustomRole {
            action: action(CUSTOS_READ)
        })
    );
    assert_eq!(
        catalog.custom_role([action("acta::document::delete")]),
        Err(CatalogError::UnknownAction {
            action: action("acta::document::delete")
        })
    );
}

#[test]
fn a_grant_spec_resolves_each_authority_kind_into_an_evaluator_grant() {
    let catalog = catalog();
    let principal = PrincipalId::new();
    let target = ref_target("acta::folder::f1");

    let builtin = catalog
        .resolve_grant(GrantSpec {
            principal: Some(principal),
            target: Some(target.clone()),
            builtin_role: Some(role_ref("acta", "editor", 2)),
            ..GrantSpec::default()
        })
        .unwrap();
    let custom = catalog
        .resolve_grant(GrantSpec {
            principal: Some(principal),
            target: Some(target.clone()),
            custom_role: Some(catalog.custom_role([action(UPDATE)]).unwrap()),
            ..GrantSpec::default()
        })
        .unwrap();
    let explicit = catalog
        .resolve_grant(GrantSpec {
            principal: Some(principal),
            target: Some(target.clone()),
            actions: Some(actions(&[READ])),
            ..GrantSpec::default()
        })
        .unwrap();

    assert_eq!(builtin.subject(), &Subject::Principal(principal));
    assert_eq!(builtin.target(), &target);
    assert_eq!(
        builtin.actions(),
        catalog.builtin_role("acta", "editor", 2).unwrap().actions()
    );
    assert_eq!(custom.actions(), &actions(&[UPDATE]));
    assert_eq!(explicit.actions(), &actions(&[READ]));
}

#[test]
fn a_grant_spec_needs_exactly_one_subject_and_a_target() {
    let catalog = catalog();
    let target = ref_target("acta::document::d1");

    let mut no_subject = spec_for(target.clone());
    no_subject.principal = None;
    let mut two_subjects = spec_for(target.clone());
    two_subjects.group = Some(GroupId::new());
    let mut no_target = spec_for(target);
    no_target.target = None;

    assert_eq!(
        catalog.resolve_grant(no_subject),
        Err(CatalogError::SubjectCount { found: 0 })
    );
    assert_eq!(
        catalog.resolve_grant(two_subjects),
        Err(CatalogError::SubjectCount { found: 2 })
    );
    assert_eq!(
        catalog.resolve_grant(no_target),
        Err(CatalogError::MissingTarget)
    );
}

#[test]
fn a_grant_spec_needs_exactly_one_authority() {
    let catalog = catalog();
    let target = ref_target("acta::document::d1");

    let mut none = spec_for(target.clone());
    none.builtin_role = None;
    let mut two = spec_for(target.clone());
    two.actions = Some(actions(&[READ]));
    let mut three = spec_for(target);
    three.actions = Some(actions(&[READ]));
    three.custom_role = Some(catalog.custom_role([action(READ)]).unwrap());

    assert_eq!(
        catalog.resolve_grant(none),
        Err(CatalogError::AuthorityCount { found: 0 })
    );
    assert_eq!(
        catalog.resolve_grant(two),
        Err(CatalogError::AuthorityCount { found: 2 })
    );
    assert_eq!(
        catalog.resolve_grant(three),
        Err(CatalogError::AuthorityCount { found: 3 })
    );
}

#[test]
fn the_authority_product_must_equal_the_target_product() {
    let catalog = catalog();
    let custos_target = ref_target("custos::grant::g1");
    let mismatch = Err(CatalogError::ProductMismatch {
        target_product: "custos".to_string(),
        authority_product: "acta".to_string(),
    });

    let builtin = spec_for(custos_target.clone());
    let mut custom = spec_for(custos_target.clone());
    custom.builtin_role = None;
    custom.custom_role = Some(catalog.custom_role([action(READ)]).unwrap());
    let mut explicit = spec_for(custos_target);
    explicit.builtin_role = None;
    explicit.actions = Some(actions(&[READ]));

    assert_eq!(catalog.resolve_grant(builtin), mismatch);
    assert_eq!(catalog.resolve_grant(custom), mismatch);
    assert_eq!(catalog.resolve_grant(explicit), mismatch);
}

#[test]
fn unknown_roles_actions_and_kinds_are_rejected_in_a_grant_spec() {
    let catalog = catalog();

    let mut unknown_version = spec_for(ref_target("acta::document::d1"));
    unknown_version.builtin_role = Some(role_ref("acta", "viewer", 9));
    let mut unknown_action = spec_for(ref_target("acta::document::d1"));
    unknown_action.builtin_role = None;
    unknown_action.actions = Some(actions(&["acta::document::delete"]));
    let mut empty_actions = spec_for(ref_target("acta::document::d1"));
    empty_actions.builtin_role = None;
    empty_actions.actions = Some(actions(&[]));
    let unknown_kind = spec_for(ref_target("acta::board::b1"));

    assert_eq!(
        catalog.resolve_grant(unknown_version),
        Err(CatalogError::UnknownRole {
            product: "acta".to_string(),
            name: "viewer".to_string(),
            version: 9,
        })
    );
    assert_eq!(
        catalog.resolve_grant(unknown_action),
        Err(CatalogError::UnknownAction {
            action: action("acta::document::delete")
        })
    );
    assert_eq!(
        catalog.resolve_grant(empty_actions),
        Err(CatalogError::EmptyActions)
    );
    assert!(matches!(
        catalog.resolve_grant(unknown_kind),
        Err(CatalogError::UnknownKind { .. })
    ));
}

#[test]
fn a_principal_set_subject_must_be_declared_by_its_product() {
    let catalog = catalog();
    let members: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();
    let guests: PrincipalSetId = "acta::workspace::w1::guests".parse().unwrap();

    let mut declared = spec_for(ref_target("acta::workspace::w1"));
    declared.principal = None;
    declared.principal_set = Some(members.clone());
    let mut undeclared = declared.clone();
    undeclared.principal_set = Some(guests.clone());

    let grant = catalog.resolve_grant(declared).unwrap();

    assert_eq!(grant.subject(), &Subject::PrincipalSet(members));
    assert_eq!(
        catalog.resolve_grant(undeclared),
        Err(CatalogError::UndeclaredPrincipalSet { set: guests })
    );
}

#[test]
fn a_custom_role_is_rechecked_against_the_catalog_that_resolves_the_grant() {
    let role = catalog().custom_role([action(UPDATE)]).unwrap();
    let mut narrowed = acta();
    narrowed
        .actions
        .retain(|declared| *declared != action(UPDATE));
    narrowed
        .roles
        .retain(|builtin| !builtin.actions.contains(&action(UPDATE)));
    let narrowed = Catalog::new([narrowed, custos()]).unwrap();

    let mut spec = spec_for(ref_target("acta::document::d1"));
    spec.builtin_role = None;
    spec.custom_role = Some(role);

    assert_eq!(
        narrowed.resolve_grant(spec),
        Err(CatalogError::UnknownAction {
            action: action(UPDATE)
        })
    );
}

const GRANT_CREATE: &str = "custos::grant::create";
const ADD_MEMBER: &str = "custos::group::add_member";

fn catalog_with_admin() -> Catalog {
    let mut product = acta();
    product
        .roles
        .push(role("admin", 1, &[READ, UPDATE, GRANT_CREATE, ADD_MEMBER]));

    Catalog::new([product, custos()]).unwrap()
}

#[test]
fn builtin_roles_may_carry_delegation_actions() {
    let catalog = catalog_with_admin();

    let admin = catalog.builtin_role("acta", "admin", 1).unwrap();

    assert!(admin.actions().contains(&action(GRANT_CREATE)));
    assert!(admin.actions().contains(&action(ADD_MEMBER)));
    assert_eq!(admin.actions().product(), Some("acta"));
}

#[test]
fn a_builtin_role_may_not_carry_other_custos_actions() {
    let mut product = acta();
    product.roles.push(role("auditor", 1, &[READ, CUSTOS_READ]));

    assert!(invalid_catalog(vec![product, custos()]));
}

#[test]
fn a_builtin_role_with_delegation_actions_resolves_into_a_grant_on_its_product() {
    let catalog = catalog_with_admin();
    let mut spec = spec_for(ref_target("acta::workspace::w1"));
    spec.builtin_role = Some(role_ref("acta", "admin", 1));

    let grant = catalog.resolve_grant(spec).unwrap();

    assert!(grant.actions().contains(&action(GRANT_CREATE)));
    assert!(grant.actions().contains(&action(READ)));
}

#[test]
fn explicit_actions_may_carry_delegation_actions() {
    let catalog = catalog();

    for granted in [&[READ, GRANT_CREATE][..], &[GRANT_CREATE, ADD_MEMBER][..]] {
        let mut spec = spec_for(ref_target("acta::workspace::w1"));
        spec.builtin_role = None;
        spec.actions = Some(actions(granted));

        let grant = catalog.resolve_grant(spec).unwrap();

        assert!(grant.actions().contains(&action(GRANT_CREATE)));
    }
}

#[test]
fn custom_roles_still_reject_every_custos_action_including_delegation() {
    let catalog = catalog();

    for rejected in [&[READ, GRANT_CREATE][..], &[ADD_MEMBER][..]] {
        assert!(matches!(
            catalog.custom_role(rejected.iter().map(|raw| action(raw))),
            Err(CatalogError::CustosActionInCustomRole { .. })
        ));
    }
}
