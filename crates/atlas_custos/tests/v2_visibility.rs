//! Public-surface behavioral tests for the list visibility predicate
//! (EVAL-5, PROV-3). Every scenario pins equivalence with the single-target
//! evaluator: the predicate permits a path exactly when `evaluate` allows
//! the same action on the path's leaf with confirmed existence.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::ids::{PrincipalSetId, ResourcePath};
use atlas_custos::eval::{
    Ceiling, Decision, DenyMode, DenyRule, EvalError, EvalRequest, Evaluated, EvaluationFacts,
    Existence, Grant, Membership, MembershipFacts, RuleEffect, Subject, VisibilityPredicate,
    evaluate, visibility_filter,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use support::{
    Fixture, READ, UPDATE, action, ceiling, deny, grant, path_target, ref_target, selector_target,
};

const PATHS: [&str; 6] = [
    "acta::workspace::w1/folder::f1/document::d1",
    "acta::workspace::w1/folder::f1/document::d2",
    "acta::workspace::w1/folder::f2/document::d3",
    "acta::workspace::w1/document::d4",
    "acta::workspace::w2/folder::f3/document::d5",
    "acta::workspace::w1/folder::f1/folder::f9/document::d6",
];

/// The shared facts of one scenario besides grants, denies and membership.
struct Scenario {
    is_root: bool,
    ceiling: Ceiling,
    deny_mode: DenyMode,
}

impl Scenario {
    fn enforced() -> Self {
        Self {
            is_root: false,
            ceiling: Ceiling::Unrestricted,
            deny_mode: DenyMode::Enforced,
        }
    }

    fn with_mode(deny_mode: DenyMode) -> Self {
        Self {
            deny_mode,
            ..Self::enforced()
        }
    }

    fn filter(
        &self,
        actor: PrincipalId,
        action_raw: &str,
        facts: &EvaluationFacts<'_>,
    ) -> Result<VisibilityPredicate, EvalError> {
        visibility_filter(
            actor,
            self.is_root,
            "document",
            &action(action_raw),
            &self.ceiling,
            self.deny_mode,
            facts,
        )
    }

    fn allowed(
        &self,
        actor: PrincipalId,
        action_raw: &str,
        path: &ResourcePath,
        facts: &EvaluationFacts<'_>,
    ) -> Result<Evaluated, EvalError> {
        let request = EvalRequest {
            actor,
            is_root: self.is_root,
            action: action(action_raw),
            target: path.leaf_ref(),
            path: Some(path.clone()),
            existence: Existence::Exists,
            ceiling: self.ceiling.clone(),
            deny_mode: self.deny_mode,
        };

        evaluate(&request, facts)
    }

    /// Asserts predicate/evaluator equivalence for `action_raw` over every
    /// path and returns the ids of the visible documents.
    fn assert_equivalent(
        &self,
        actor: PrincipalId,
        facts: &EvaluationFacts<'_>,
        action_raw: &str,
    ) -> Vec<String> {
        self.assert_equivalent_with_errors(actor, facts, action_raw, &[])
    }

    /// Like [`Self::assert_equivalent`], but `evaluate` must fail exactly on
    /// the documents in `failing` and succeed everywhere else, so an Ok
    /// predicate is never compared against an evaluator error by accident.
    fn assert_equivalent_with_errors(
        &self,
        actor: PrincipalId,
        facts: &EvaluationFacts<'_>,
        action_raw: &str,
        failing: &[&str],
    ) -> Vec<String> {
        let predicate = self.filter(actor, action_raw, facts).unwrap();
        let mut visible = Vec::new();

        for raw in PATHS {
            let path: ResourcePath = raw.parse().unwrap();
            let evaluated = self.allowed(actor, action_raw, &path, facts);
            let expect_error = failing.contains(&path.leaf_ref().id());

            assert_eq!(
                evaluated.is_err(),
                expect_error,
                "{action_raw} on {raw}: {evaluated:?}"
            );

            let allowed = matches!(
                evaluated,
                Ok(Evaluated {
                    decision: Decision::Allowed,
                    ..
                })
            );

            assert_eq!(
                predicate.permits(&path),
                allowed,
                "{action_raw} on {raw} with {predicate:?}"
            );

            if allowed {
                visible.push(path.leaf_ref().id().to_string());
            }
        }

        visible
    }
}

fn facts<'a>(
    grants: &'a [Grant],
    denies: &'a [DenyRule],
    membership: &'a MembershipFacts,
) -> EvaluationFacts<'a> {
    EvaluationFacts {
        grants,
        denies,
        membership,
    }
}

#[test]
fn the_nearest_level_shadows_farther_grants() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ, UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[UPDATE],
        ),
    ];

    let facts = fixture.facts(&grants, &[]);
    let scenario = Scenario::enforced();

    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, READ),
        ["d3", "d4"]
    );
    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, UPDATE),
        ["d1", "d2", "d3", "d4", "d6"]
    );
}

#[test]
fn the_strongest_tier_wins_within_a_level_and_equal_tiers_union() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            path_target("acta::workspace::w1/folder::f1/document::d2"),
            &[UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::d3"),
            &[UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::d3"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w2"),
            &[UPDATE],
        ),
    ];

    let facts = fixture.facts(&grants, &[]);
    let scenario = Scenario::enforced();

    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, READ),
        ["d1", "d3", "d4", "d6"]
    );
    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, UPDATE),
        ["d2", "d3", "d5"]
    );
}

#[test]
fn a_ceiling_without_the_action_is_nothing() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ, UPDATE],
    )];
    let facts = fixture.facts(&grants, &[]);
    let scenario = Scenario {
        ceiling: ceiling(&[UPDATE, "custos::grant::read"]),
        ..Scenario::enforced()
    };

    assert_eq!(
        scenario.filter(fixture.actor, READ, &facts).unwrap(),
        VisibilityPredicate::Nothing
    );
    assert!(
        scenario
            .assert_equivalent(fixture.actor, &facts, READ)
            .is_empty()
    );
    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, UPDATE),
        ["d1", "d2", "d3", "d4", "d6"]
    );
}

#[test]
fn enforced_denies_block_on_the_target_or_an_ancestor_while_audit_and_disabled_do_not() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        selector_target("acta::workspace::w1/**"),
        &[READ, UPDATE],
    )];
    let denies = [
        deny(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[READ],
        ),
        deny(
            fixture.alice.clone(),
            ref_target("acta::document::d4"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &denies);

    assert_eq!(
        Scenario::enforced().assert_equivalent(fixture.actor, &facts, READ),
        ["d3"]
    );
    assert_eq!(
        Scenario::enforced().assert_equivalent(fixture.actor, &facts, UPDATE),
        ["d1", "d2", "d3", "d4", "d6"]
    );
    for mode in [DenyMode::Audit, DenyMode::Disabled] {
        assert_eq!(
            Scenario::with_mode(mode).assert_equivalent(fixture.actor, &facts, READ),
            ["d1", "d2", "d3", "d4", "d6"]
        );
    }
}

#[test]
fn unrelated_denies_and_grants_are_ignored() {
    let fixture = Fixture::new();
    let bob = Subject::Principal(PrincipalId::new());
    let unknown = Subject::Group(GroupId::new());
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
        grant(bob.clone(), ref_target("acta::document::d1"), &[]),
        grant(unknown.clone(), ref_target("custos::grant::g1"), &[]),
    ];
    let denies = [
        deny(bob, ref_target("acta::workspace::w1"), &[READ]),
        deny(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[UPDATE],
        ),
        deny(
            unknown,
            ref_target("custos::grant::g1"),
            &["custos::grant::read"],
        ),
    ];

    let facts = fixture.facts(&grants, &denies);
    let scenario = Scenario::enforced();

    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &facts, READ),
        ["d1", "d2", "d3", "d4", "d6"]
    );
    assert!(
        scenario
            .assert_equivalent(fixture.actor, &facts, UPDATE)
            .is_empty()
    );
}

#[test]
fn group_and_principal_set_grants_apply_only_through_confirmed_membership() {
    let fixture = Fixture::new();
    let editors = GroupId::new();
    let outsiders = GroupId::new();
    let members: PrincipalSetId = "acta::workspace::w2::members".parse().unwrap();
    let pending: PrincipalSetId = "acta::workspace::w1::pending".parse().unwrap();
    let mut membership = MembershipFacts::new(fixture.actor);
    membership.set_group(editors, Membership::Member);
    membership.set_group(outsiders, Membership::NotMember);
    membership.set_principal_set(members.clone(), Membership::Member);
    membership.set_principal_set(pending.clone(), Membership::Indeterminate);
    let grants = [
        grant(
            Subject::Group(editors),
            ref_target("acta::folder::f1"),
            &[READ],
        ),
        grant(
            Subject::Group(outsiders),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
        grant(
            Subject::PrincipalSet(members),
            ref_target("acta::workspace::w2"),
            &[READ],
        ),
        grant(
            Subject::PrincipalSet(pending),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
    ];

    let visible = Scenario::enforced().assert_equivalent(
        fixture.actor,
        &facts(&grants, &[], &membership),
        READ,
    );

    assert_eq!(visible, ["d1", "d2", "d5", "d6"]);
}

#[test]
fn root_sees_everything_without_grants_or_ceiling() {
    let fixture = Fixture::new();
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ],
    )];
    let facts = fixture.facts(&[], &denies);
    let scenario = Scenario {
        is_root: true,
        ceiling: Ceiling::Restricted(Default::default()),
        ..Scenario::enforced()
    };

    assert_eq!(
        scenario.filter(fixture.actor, READ, &facts).unwrap(),
        VisibilityPredicate::All
    );
    assert_eq!(
        scenario
            .assert_equivalent(fixture.actor, &facts, READ)
            .len(),
        PATHS.len()
    );
}

#[test]
fn no_authority_for_the_action_is_nothing() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[UPDATE],
    )];
    let facts = fixture.facts(&grants, &[]);

    assert_eq!(
        Scenario::enforced()
            .filter(fixture.actor, READ, &facts)
            .unwrap(),
        VisibilityPredicate::Nothing
    );
    assert!(
        Scenario::enforced()
            .assert_equivalent(fixture.actor, &facts, READ)
            .is_empty()
    );
}

#[test]
fn an_unknown_group_grant_that_could_win_a_tier_is_an_error() {
    let fixture = Fixture::new();
    let unknown = GroupId::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        ),
        grant(
            Subject::Group(unknown),
            ref_target("acta::document::d3"),
            &[],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);
    let d3: ResourcePath = PATHS[2].parse().unwrap();

    assert!(matches!(
        Scenario::enforced().filter(fixture.actor, READ, &facts),
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
    assert!(matches!(
        Scenario::enforced().allowed(fixture.actor, READ, &d3, &facts),
        Err(EvalError::GroupMembershipUnavailable { .. })
    ));
}

#[test]
fn an_unknown_group_grant_is_irrelevant_when_nothing_could_allow_the_action() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[UPDATE],
        ),
        grant(
            Subject::Group(GroupId::new()),
            ref_target("acta::folder::f1"),
            &[UPDATE],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    assert_eq!(
        Scenario::enforced()
            .filter(fixture.actor, READ, &facts)
            .unwrap(),
        VisibilityPredicate::Nothing
    );
}

#[test]
fn an_unknown_group_deny_on_the_action_is_an_error_unless_denies_are_disabled() {
    let fixture = Fixture::new();
    let unknown = GroupId::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ, UPDATE],
    )];
    let denies = [deny(
        Subject::Group(unknown),
        ref_target("acta::folder::f2"),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);

    for mode in [DenyMode::Enforced, DenyMode::Audit] {
        assert!(matches!(
            Scenario::with_mode(mode).filter(fixture.actor, READ, &facts),
            Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
        ));
    }
    assert_eq!(
        Scenario::with_mode(DenyMode::Disabled).assert_equivalent(fixture.actor, &facts, READ),
        ["d1", "d2", "d3", "d4", "d6"]
    );
    assert_eq!(
        Scenario::enforced().assert_equivalent(fixture.actor, &facts, UPDATE),
        ["d1", "d2", "d3", "d4", "d6"]
    );
}

#[test]
fn audit_mode_errors_when_an_unknown_deny_could_decide_a_denied_action() {
    let fixture = Fixture::new();
    let unknown = GroupId::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ, UPDATE],
    )];
    let denies = [
        deny(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[READ],
        ),
        deny(
            Subject::Group(unknown),
            ref_target("acta::folder::f1"),
            &[UPDATE],
        ),
    ];
    let facts = fixture.facts(&grants, &denies);
    let d1: ResourcePath = PATHS[0].parse().unwrap();
    let audit = Scenario::with_mode(DenyMode::Audit);

    assert!(matches!(
        audit.filter(fixture.actor, READ, &facts),
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown
    ));
    assert!(matches!(
        audit.allowed(fixture.actor, READ, &d1, &facts),
        Err(EvalError::GroupMembershipUnavailable { .. })
    ));
    assert_eq!(
        Scenario::enforced().assert_equivalent_with_errors(
            fixture.actor,
            &facts,
            READ,
            &["d1", "d2", "d6"]
        ),
        ["d3", "d4"]
    );
}

#[test]
fn a_nearer_level_decides_between_allow_and_block_rules() {
    let fixture = Fixture::new();
    let scenario = Scenario::enforced();
    let nearer_allow = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[READ],
        ),
    ];
    let nearer_block = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::folder::f1"),
            &[UPDATE],
        ),
    ];

    for grants in [&nearer_allow, &nearer_block] {
        let VisibilityPredicate::Rules { grants: rules, .. } = scenario
            .filter(fixture.actor, READ, &fixture.facts(grants, &[]))
            .unwrap()
        else {
            panic!("expected grant rules");
        };
        let effects: Vec<RuleEffect> = rules.iter().map(|rule| rule.effect).collect();

        assert_eq!(rules.len(), 2);
        assert!(effects.contains(&RuleEffect::Allow));
        assert!(effects.contains(&RuleEffect::Block));
    }

    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &fixture.facts(&nearer_allow, &[]), READ),
        ["d1", "d2", "d6"]
    );
    assert_eq!(
        scenario.assert_equivalent(fixture.actor, &fixture.facts(&nearer_block, &[]), READ),
        ["d3", "d4"]
    );
}

#[test]
fn membership_facts_for_another_principal_are_rejected() {
    let fixture = Fixture::new();
    let foreign = MembershipFacts::new(PrincipalId::new());
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ],
    )];

    assert!(matches!(
        Scenario::enforced().filter(fixture.actor, READ, &facts(&grants, &[], &foreign)),
        Err(EvalError::InconsistentFacts { .. })
    ));
}

#[test]
fn the_predicate_converts_to_its_storage_neutral_form_unchanged() {
    use atlas_core::visibility::{ListVisibility, VisibilityGrant, VisibilityTarget};
    use atlas_custos::eval::{GrantTarget, VisibilityRule};

    let workspace = ref_target("acta::workspace::w1");
    let folder = path_target("acta::workspace::w1/folder::f1");
    let documents = selector_target("acta::workspace::w1/**");
    let predicate = VisibilityPredicate::Rules {
        grants: vec![
            VisibilityRule {
                target: workspace.clone(),
                effect: RuleEffect::Allow,
            },
            VisibilityRule {
                target: folder.clone(),
                effect: RuleEffect::Block,
            },
        ],
        denies: vec![documents.clone()],
    };

    let neutral = |target: GrantTarget| match target {
        GrantTarget::Ref(reference) => VisibilityTarget::Ref(reference),
        GrantTarget::Path(path) => VisibilityTarget::Path(path),
        GrantTarget::Selector(selector) => VisibilityTarget::Selector(selector),
    };

    assert_eq!(
        ListVisibility::from(&predicate),
        ListVisibility::Rules {
            grants: vec![
                VisibilityGrant {
                    target: neutral(workspace),
                    allow: true,
                },
                VisibilityGrant {
                    target: neutral(folder),
                    allow: false,
                },
            ],
            denies: vec![neutral(documents)],
        }
    );
    assert_eq!(
        ListVisibility::from(&VisibilityPredicate::All),
        ListVisibility::All
    );
    assert_eq!(
        ListVisibility::from(&VisibilityPredicate::Nothing),
        ListVisibility::Nothing
    );
}
