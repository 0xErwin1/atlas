//! Public-surface behavioral tests for Custos delegation actions in the
//! evaluator model (E5 S5a): action sets mixing one product with delegation
//! actions, their evaluation against the grant's own target, and the pure
//! delegation check.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::ids::ResourcePath;
use atlas_custos::eval::{
    ActionSet, Decision, DelegationRefused, DenyCause, DenyMode, DenyRule, EffectiveActions,
    EffectiveRequest, EvalError, EvalRequest, Evaluated, Existence, FactFailure, Grant,
    can_delegate, effective_actions, evaluate, visibility_filter,
};
use support::{Fixture, READ, UPDATE, action, actions, ceiling, deny, grant, ref_target};

const GRANT_CREATE: &str = "custos::grant::create";
const GRANT_DELETE: &str = "custos::grant::delete";
const ADD_MEMBER: &str = "custos::group::add_member";
const WORKSPACE_READ: &str = "acta::workspace::read";

fn workspace_request(actor: atlas_custos::ids::PrincipalId, action_raw: &str) -> EvalRequest {
    EvalRequest {
        actor,
        is_root: false,
        action: action(action_raw),
        target: "acta::workspace::w1".parse().unwrap(),
        path: Some("acta::workspace::w1".parse().unwrap()),
        existence: Existence::Exists,
        ceiling: atlas_custos::eval::Ceiling::Unrestricted,
        deny_mode: DenyMode::Enforced,
    }
}

fn folder_request(actor: atlas_custos::ids::PrincipalId, action_raw: &str) -> EvalRequest {
    EvalRequest {
        target: "acta::folder::f1".parse().unwrap(),
        path: Some("acta::workspace::w1/folder::f1".parse().unwrap()),
        ..workspace_request(actor, action_raw)
    }
}

#[test]
fn an_action_set_may_combine_one_product_with_delegation_actions() {
    let set = ActionSet::new(
        [READ, GRANT_CREATE, GRANT_DELETE, ADD_MEMBER]
            .into_iter()
            .map(action),
    )
    .unwrap();

    assert_eq!(set.product(), Some("acta"));
    assert!(set.contains(&action(GRANT_CREATE)));
}

#[test]
fn custos_only_action_sets_stay_valid() {
    let set = ActionSet::new(
        ["custos::grant::read", GRANT_CREATE]
            .into_iter()
            .map(action),
    )
    .unwrap();

    assert_eq!(set.product(), Some("custos"));
}

#[test]
fn two_non_custos_products_are_still_rejected() {
    let mixed = ActionSet::new(
        [READ, "virtus::task::read", GRANT_CREATE]
            .into_iter()
            .map(action),
    );

    assert_eq!(
        mixed.unwrap_err(),
        EvalError::CrossProductActions {
            first: "acta".to_string(),
            second: "virtus".to_string(),
        }
    );
}

#[test]
fn non_delegation_custos_actions_do_not_mix_with_a_product() {
    let mixed = ActionSet::new([READ, "custos::grant::read"].into_iter().map(action));

    assert_eq!(
        mixed.unwrap_err(),
        EvalError::CrossProductActions {
            first: "acta".to_string(),
            second: "custos".to_string(),
        }
    );
}

#[test]
fn a_mixed_grant_or_deny_must_target_its_product() {
    let fixture = Fixture::new();
    let mixed = actions(&[READ, GRANT_CREATE]);

    assert!(
        Grant::new(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            mixed.clone()
        )
        .is_ok()
    );
    assert!(
        DenyRule::new(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            mixed.clone()
        )
        .is_ok()
    );
    assert!(matches!(
        Grant::new(
            fixture.alice.clone(),
            ref_target("custos::grant::g1"),
            mixed.clone()
        ),
        Err(EvalError::CrossProductFact { .. })
    ));
    assert!(matches!(
        DenyRule::new(
            fixture.alice.clone(),
            ref_target("custos::grant::g1"),
            mixed
        ),
        Err(EvalError::CrossProductFact { .. })
    ));
}

#[test]
fn a_delegation_only_grant_is_valid_on_product_and_custos_targets() {
    let fixture = Fixture::new();
    let delegation = actions(&[GRANT_CREATE, GRANT_DELETE]);

    for target in ["acta::workspace::w1", "custos::grant::g1"] {
        assert!(
            Grant::new(
                fixture.alice.clone(),
                ref_target(target),
                delegation.clone()
            )
            .is_ok()
        );
        assert!(
            DenyRule::new(
                fixture.alice.clone(),
                ref_target(target),
                delegation.clone()
            )
            .is_ok()
        );
    }

    assert!(matches!(
        Grant::new(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            actions(&["custos::grant::read"])
        ),
        Err(EvalError::CrossProductFact { .. })
    ));
}

#[test]
fn a_delegation_action_is_evaluated_against_the_granted_target() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[WORKSPACE_READ, READ, UPDATE, GRANT_CREATE],
    )];
    let facts = fixture.facts(&grants, &[]);

    let workspace = evaluate(&workspace_request(fixture.actor, GRANT_CREATE), &facts).unwrap();
    let folder = evaluate(&folder_request(fixture.actor, GRANT_CREATE), &facts).unwrap();

    assert_eq!(workspace.decision, Decision::Allowed);
    assert_eq!(folder.decision, Decision::Allowed);
}

#[test]
fn an_agent_ceiling_without_grant_create_denies_delegation_despite_an_admin_like_grant() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[WORKSPACE_READ, READ, UPDATE, GRANT_CREATE, GRANT_DELETE],
    )];
    let facts = fixture.facts(&grants, &[]);
    let mut request = workspace_request(fixture.actor, GRANT_CREATE);
    request.ceiling = ceiling(&[WORKSPACE_READ, READ, UPDATE]);

    let outcome = evaluate(&request, &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
}

#[test]
fn delegation_actions_follow_precedence_and_denies_like_any_action() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[WORKSPACE_READ, GRANT_CREATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &["acta::folder::read", GRANT_DELETE],
        ),
    ];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[GRANT_CREATE],
    )];

    let shadowed = evaluate(
        &folder_request(fixture.actor, GRANT_CREATE),
        &fixture.facts(&grants, &[]),
    )
    .unwrap();
    let denied = evaluate(
        &workspace_request(fixture.actor, GRANT_CREATE),
        &fixture.facts(&grants, &denies),
    )
    .unwrap();

    assert_eq!(
        shadowed.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
    assert_eq!(
        denied.decision,
        Decision::Denied {
            because: DenyCause::DenyRuleEnforced
        }
    );
}

#[test]
fn the_visibility_predicate_agrees_with_evaluate_for_a_delegation_action() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ, GRANT_CREATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);
    let predicate = visibility_filter(
        fixture.actor,
        false,
        "document",
        &action(GRANT_CREATE),
        &atlas_custos::eval::Ceiling::Unrestricted,
        DenyMode::Enforced,
        &facts,
    )
    .unwrap();
    let mut visible = Vec::new();

    for raw in [
        "acta::workspace::w1/folder::f1/document::d1",
        "acta::workspace::w1/folder::f2/document::d2",
        "acta::workspace::w2/document::d3",
    ] {
        let path: ResourcePath = raw.parse().unwrap();
        let request = EvalRequest {
            target: path.leaf_ref(),
            path: Some(path.clone()),
            ..workspace_request(fixture.actor, GRANT_CREATE)
        };
        let allowed = matches!(
            evaluate(&request, &facts),
            Ok(Evaluated {
                decision: Decision::Allowed,
                ..
            })
        );

        assert_eq!(predicate.permits(&path), allowed, "{raw}");

        if allowed {
            visible.push(path.leaf_ref().id().to_string());
        }
    }

    assert_eq!(visible, ["d1", "d2"]);
}

/// Asserts predicate/evaluator equivalence for `action_raw` over
/// `paths` and returns the ids of the visible documents.
fn assert_equivalent(
    actor: atlas_custos::ids::PrincipalId,
    facts: &atlas_custos::eval::EvaluationFacts<'_>,
    deny_mode: DenyMode,
    action_raw: &str,
    paths: &[&str],
) -> Vec<String> {
    let predicate = visibility_filter(
        actor,
        false,
        "document",
        &action(action_raw),
        &atlas_custos::eval::Ceiling::Unrestricted,
        deny_mode,
        facts,
    )
    .unwrap();
    let mut visible = Vec::new();

    for raw in paths {
        let path: ResourcePath = raw.parse().unwrap();
        let request = EvalRequest {
            target: path.leaf_ref(),
            path: Some(path.clone()),
            deny_mode,
            ..workspace_request(actor, action_raw)
        };
        let evaluated = evaluate(&request, facts);

        assert!(evaluated.is_ok(), "{action_raw} on {raw}: {evaluated:?}");
        let allowed = matches!(
            evaluated,
            Ok(Evaluated {
                decision: Decision::Allowed,
                ..
            })
        );
        assert_eq!(predicate.permits(&path), allowed, "{action_raw} on {raw}");

        if allowed {
            visible.push(path.leaf_ref().id().to_string());
        }
    }

    visible
}

const DOCS: [&str; 3] = [
    "acta::workspace::w1/folder::f1/document::d1",
    "acta::workspace::w1/folder::f2/document::d2",
    "acta::workspace::w2/document::d3",
];

fn document_request(actor: atlas_custos::ids::PrincipalId, action_raw: &str) -> EvalRequest {
    let path: ResourcePath = DOCS[0].parse().unwrap();

    EvalRequest {
        target: path.leaf_ref(),
        path: Some(path),
        ..workspace_request(actor, action_raw)
    }
}

#[test]
fn a_delegation_only_grant_does_not_shadow_product_authority() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ, UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::d1"),
            &[GRANT_CREATE],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    let read = evaluate(&document_request(fixture.actor, READ), &facts).unwrap();
    let create = evaluate(&document_request(fixture.actor, GRANT_CREATE), &facts).unwrap();

    assert_eq!(read.decision, Decision::Allowed);
    assert_eq!(create.decision, Decision::Allowed);
    assert_eq!(
        assert_equivalent(fixture.actor, &facts, DenyMode::Enforced, READ, &DOCS),
        ["d1", "d2"]
    );
    assert_eq!(
        assert_equivalent(
            fixture.actor,
            &facts,
            DenyMode::Enforced,
            GRANT_CREATE,
            &DOCS
        ),
        ["d1"]
    );
}

#[test]
fn a_product_only_grant_does_not_shadow_delegation_authority() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[WORKSPACE_READ, GRANT_CREATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &["acta::folder::read"],
        ),
    ];

    let outcome = evaluate(
        &folder_request(fixture.actor, GRANT_CREATE),
        &fixture.facts(&grants, &[]),
    )
    .unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn an_unknown_delegation_only_group_grant_only_matters_for_delegation_requests() {
    let fixture = Fixture::new();
    let unknown = atlas_custos::ids::GroupId::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ, GRANT_CREATE],
        ),
        grant(
            atlas_custos::eval::Subject::Group(unknown),
            ref_target("acta::document::d1"),
            &[GRANT_CREATE],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    let read = evaluate(&document_request(fixture.actor, READ), &facts).unwrap();
    let create = evaluate(&document_request(fixture.actor, GRANT_CREATE), &facts);

    assert_eq!(read.decision, Decision::Allowed);
    assert!(matches!(
        create,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
    assert_eq!(
        assert_equivalent(fixture.actor, &facts, DenyMode::Enforced, READ, &DOCS),
        ["d1", "d2"]
    );
}

#[test]
fn an_allowed_delegation_request_needs_no_product_discovery_facts() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[GRANT_CREATE],
        ),
        grant(
            atlas_custos::eval::Subject::Group(atlas_custos::ids::GroupId::new()),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    for mode in [DenyMode::Disabled, DenyMode::Audit, DenyMode::Enforced] {
        let mut request = document_request(fixture.actor, GRANT_CREATE);
        request.deny_mode = mode;

        assert_eq!(
            evaluate(&request, &facts).unwrap().decision,
            Decision::Allowed
        );
        assert_eq!(
            assert_equivalent(fixture.actor, &facts, mode, GRANT_CREATE, &DOCS),
            ["d1", "d2"]
        );
    }
}

#[test]
fn audit_delegation_lists_fail_when_a_denied_row_would_need_unknown_discovery_facts() {
    let fixture = Fixture::new();
    let unknown = atlas_custos::ids::GroupId::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[GRANT_CREATE],
        ),
        grant(
            atlas_custos::eval::Subject::Group(unknown),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
    ];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::folder::f1"),
        &[GRANT_CREATE],
    )];
    let facts = fixture.facts(&grants, &denies);
    let mut request = document_request(fixture.actor, GRANT_CREATE);
    request.deny_mode = DenyMode::Audit;

    assert!(matches!(
        evaluate(&request, &facts),
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
    assert!(matches!(
        visibility_filter(
            fixture.actor,
            false,
            "document",
            &action(GRANT_CREATE),
            &atlas_custos::eval::Ceiling::Unrestricted,
            DenyMode::Audit,
            &facts,
        ),
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
}

fn effective_request(actor: atlas_custos::ids::PrincipalId) -> EffectiveRequest {
    EffectiveRequest {
        actor,
        is_root: false,
        target: "acta::workspace::w1".parse().unwrap(),
        path: Some("acta::workspace::w1".parse().unwrap()),
        existence: Existence::Exists,
        ceiling: atlas_custos::eval::Ceiling::Unrestricted,
        deny_mode: DenyMode::Enforced,
    }
}

/// The effective actions of an actor holding `held` through one Ref grant
/// on the workspace, computed on that workspace.
fn held_on_workspace(held: &[&str]) -> EffectiveActions {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        held,
    )];

    effective_actions(
        &effective_request(fixture.actor),
        &fixture.facts(&grants, &[]),
    )
    .unwrap()
}

#[test]
fn delegation_requires_grant_create_in_the_actor_effective_actions() {
    let effective = held_on_workspace(&[READ, UPDATE]);

    assert_eq!(
        can_delegate(&effective, &actions(&[READ])),
        Err(DelegationRefused::MissingGrantCreate)
    );
}

#[test]
fn delegation_refuses_actions_beyond_the_actor_authority_and_lists_them() {
    let effective = held_on_workspace(&[READ, GRANT_CREATE]);

    assert_eq!(
        can_delegate(&effective, &actions(&[UPDATE, READ, GRANT_DELETE])),
        Err(DelegationRefused::BeyondAuthority {
            actions: vec![action(UPDATE), action(GRANT_DELETE)],
        })
    );
}

#[test]
fn delegation_allows_a_subset_or_the_whole_effective_authority() {
    let effective = held_on_workspace(&[READ, UPDATE, GRANT_CREATE]);

    assert_eq!(can_delegate(&effective, &actions(&[READ])), Ok(()));
    assert_eq!(
        can_delegate(&effective, &actions(&[READ, UPDATE, GRANT_CREATE])),
        Ok(())
    );
}

#[test]
fn delegating_an_empty_action_set_still_requires_grant_create() {
    assert_eq!(
        can_delegate(&held_on_workspace(&[GRANT_CREATE]), &actions(&[])),
        Ok(())
    );
    assert_eq!(
        can_delegate(&held_on_workspace(&[READ]), &actions(&[])),
        Err(DelegationRefused::MissingGrantCreate)
    );
}

const UNIVERSE: [&str; 8] = [
    READ,
    UPDATE,
    "acta::document::delete",
    WORKSPACE_READ,
    "acta::folder::read",
    GRANT_CREATE,
    GRANT_DELETE,
    ADD_MEMBER,
];

/// Asserts that `effective` holds exactly the universe actions `evaluate`
/// allows on the request's target and returns the held ones.
fn assert_matches_evaluate(
    request: &EffectiveRequest,
    facts: &atlas_custos::eval::EvaluationFacts<'_>,
    effective: &EffectiveActions,
) -> Vec<String> {
    let mut held = Vec::new();

    for raw in UNIVERSE {
        let single = EvalRequest {
            actor: request.actor,
            is_root: request.is_root,
            action: action(raw),
            target: request.target.clone(),
            path: request.path.clone(),
            existence: request.existence,
            ceiling: request.ceiling.clone(),
            deny_mode: request.deny_mode,
        };
        let allowed = matches!(
            evaluate(&single, facts),
            Ok(Evaluated {
                decision: Decision::Allowed,
                ..
            })
        );

        assert_eq!(effective.contains(&action(raw)), allowed, "{raw}");

        if allowed {
            held.push(raw.to_string());
        }
    }

    held
}

#[test]
fn effective_actions_match_evaluate_across_lanes_ceiling_denies_and_modes() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ, UPDATE, GRANT_DELETE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &["acta::folder::read", READ, "acta::document::delete"],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[GRANT_CREATE, ADD_MEMBER],
        ),
    ];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &["acta::document::delete", ADD_MEMBER],
    )];
    let facts = fixture.facts(&grants, &denies);
    let mut request = EffectiveRequest {
        target: "acta::document::d1".parse().unwrap(),
        path: Some(DOCS[0].parse().unwrap()),
        ceiling: ceiling(&[
            READ,
            "acta::document::delete",
            "acta::folder::read",
            GRANT_CREATE,
            ADD_MEMBER,
        ]),
        ..effective_request(fixture.actor)
    };

    let enforced = effective_actions(&request, &facts).unwrap();
    assert_eq!(
        assert_matches_evaluate(&request, &facts, &enforced),
        [READ, "acta::folder::read", GRANT_CREATE]
    );

    for mode in [DenyMode::Audit, DenyMode::Disabled] {
        request.deny_mode = mode;
        let unenforced = effective_actions(&request, &facts).unwrap();

        assert_eq!(
            assert_matches_evaluate(&request, &facts, &unenforced),
            [
                READ,
                "acta::document::delete",
                "acta::folder::read",
                GRANT_CREATE,
                ADD_MEMBER
            ]
        );
    }
}

#[test]
fn effective_actions_include_delegation_from_a_delegation_only_grant() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[WORKSPACE_READ],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[GRANT_CREATE],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);
    let request = effective_request(fixture.actor);

    let effective = effective_actions(&request, &facts).unwrap();

    assert_eq!(
        assert_matches_evaluate(&request, &facts, &effective),
        [WORKSPACE_READ, GRANT_CREATE]
    );
    assert_eq!(
        can_delegate(&effective, &actions(&[WORKSPACE_READ])),
        Ok(())
    );
}

#[test]
fn root_holds_every_action_and_may_delegate_anything() {
    let fixture = Fixture::new();
    let mut request = effective_request(fixture.actor);
    request.is_root = true;
    request.ceiling = atlas_custos::eval::Ceiling::Restricted(Default::default());

    let effective = effective_actions(&request, &fixture.facts(&[], &[])).unwrap();

    assert!(effective.is_all());
    assert_eq!(
        assert_matches_evaluate(&request, &fixture.facts(&[], &[]), &effective).len(),
        UNIVERSE.len()
    );
    assert_eq!(
        can_delegate(&effective, &actions(&[READ, UPDATE, GRANT_CREATE])),
        Ok(())
    );
}

#[test]
fn effective_actions_follow_existence_ancestry_and_binding_like_evaluate() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[WORKSPACE_READ, GRANT_CREATE],
    )];
    let facts = fixture.facts(&grants, &[]);

    let mut missing = effective_request(fixture.actor);
    missing.existence = Existence::Missing;
    let mut unavailable = effective_request(fixture.actor);
    unavailable.existence = Existence::Unavailable(FactFailure::Timeout);
    let mut no_path = effective_request(fixture.actor);
    no_path.path = None;
    let foreign = atlas_custos::eval::MembershipFacts::new(atlas_custos::ids::PrincipalId::new());

    let hidden = effective_actions(&missing, &facts).unwrap();
    assert!(!hidden.is_all() && hidden.iter().next().is_none());
    assert_eq!(
        effective_actions(&unavailable, &facts),
        Err(EvalError::FactsUnavailable {
            cause: FactFailure::Timeout
        })
    );
    assert_eq!(
        effective_actions(&no_path, &facts),
        Err(EvalError::MissingAncestry)
    );
    assert!(matches!(
        effective_actions(
            &effective_request(fixture.actor),
            &atlas_custos::eval::EvaluationFacts {
                grants: &grants,
                denies: &[],
                membership: &foreign,
            }
        ),
        Err(EvalError::InconsistentFacts { .. })
    ));
}

#[test]
fn effective_actions_fail_closed_on_an_unknown_deny_evaluate_would_need() {
    let fixture = Fixture::new();
    let unknown = atlas_custos::ids::GroupId::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[WORKSPACE_READ, GRANT_CREATE],
    )];
    let denies = [deny(
        atlas_custos::eval::Subject::Group(unknown),
        ref_target("acta::workspace::w1"),
        &[GRANT_CREATE],
    )];

    assert!(matches!(
        effective_actions(
            &effective_request(fixture.actor),
            &fixture.facts(&grants, &denies)
        ),
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
}
