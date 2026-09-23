//! Public-surface behavioral tests for the pure V2 authorization evaluator
//! (E5 S1). The evaluator consumes caller-supplied facts and never performs
//! I/O; these tests pin the externally visible decision contract that the
//! E7 orchestration layer will build on.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_core::ids::{ActionId, PrincipalSetId, ResourcePath, ResourceRef};
use atlas_custos::eval::{
    ActionSet, Ceiling, Decision, DenyCause, DenyMode, DenyRule, EvalError, EvalRequest,
    EvaluationFacts, Existence, FactFailure, Grant, GrantTarget, Membership, MembershipFacts,
    Subject, evaluate,
};
use atlas_custos::ids::{GroupId, PrincipalId};

fn action(raw: &str) -> ActionId {
    raw.parse().unwrap()
}

fn actions(raw: &[&str]) -> ActionSet {
    ActionSet::new(raw.iter().map(|raw| action(raw))).unwrap()
}

fn ref_target(raw: &str) -> GrantTarget {
    GrantTarget::Ref(raw.parse().unwrap())
}

fn path_target(raw: &str) -> GrantTarget {
    GrantTarget::Path(raw.parse().unwrap())
}

fn selector_target(raw: &str) -> GrantTarget {
    GrantTarget::Selector(raw.parse().unwrap())
}

fn grant(subject: Subject, target: GrantTarget, granted: &[&str]) -> Grant {
    Grant::new(subject, target, actions(granted)).unwrap()
}

fn deny(subject: Subject, target: GrantTarget, denied: &[&str]) -> DenyRule {
    DenyRule::new(subject, target, actions(denied)).unwrap()
}

const DOC: &str = "acta::document::d1";
const READ: &str = "acta::document::read";
const UPDATE: &str = "acta::document::update";
const DELETE: &str = "acta::document::delete";

/// One test fixture: the acting principal, its subject form for grants and
/// denies, and the membership facts bound to it.
struct Fixture {
    actor: PrincipalId,
    alice: Subject,
    membership: MembershipFacts,
}

impl Fixture {
    fn new() -> Self {
        let actor = PrincipalId::new();

        Self {
            actor,
            alice: Subject::Principal(actor),
            membership: MembershipFacts::new(actor),
        }
    }

    fn facts<'a>(&'a self, grants: &'a [Grant], denies: &'a [DenyRule]) -> EvaluationFacts<'a> {
        EvaluationFacts {
            grants,
            denies,
            membership: &self.membership,
        }
    }
}

fn request(actor: PrincipalId, action_id: ActionId, target: ResourceRef) -> EvalRequest {
    EvalRequest {
        actor,
        is_root: false,
        action: action_id,
        target,
        path: Some(doc_path()),
        existence: Existence::Exists,
        ceiling: Ceiling::Unrestricted,
        deny_mode: DenyMode::Enforced,
    }
}

fn doc_path() -> ResourcePath {
    "acta::workspace::w1/folder::f1/document::d1"
        .parse()
        .unwrap()
}

fn read_request(actor: PrincipalId) -> EvalRequest {
    request(actor, action(READ), DOC.parse().unwrap())
}

#[test]
fn a_confirmed_grant_on_the_target_allows_the_requested_action() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
    assert!(outcome.would_block.is_empty());
}

#[test]
fn an_enforced_ancestor_deny_beats_a_target_allow() {
    let fixture = Fixture::new();
    let path: ResourcePath = "acta::workspace::w1/folder::f1/document::d1"
        .parse()
        .unwrap();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target(DOC),
        &[READ, UPDATE],
    )];
    let denies = [deny(
        fixture.alice.clone(),
        path_target("acta::workspace::w1"),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);
    let mut req = read_request(fixture.actor);
    req.path = Some(path);

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::DenyRuleEnforced
        }
    );
}

#[test]
fn a_missing_target_is_not_found_even_when_grants_would_allow_it() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &[]);
    let mut req = read_request(fixture.actor);
    req.existence = Existence::Missing;

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
}

#[test]
fn an_existing_target_without_any_authority_is_not_found() {
    let fixture = Fixture::new();

    let outcome = evaluate(&read_request(fixture.actor), &fixture.facts(&[], &[])).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
    assert!(outcome.would_block.is_empty());
}

#[test]
fn another_effective_action_makes_the_target_visible_but_denied() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
}

#[test]
fn a_ceiling_that_removes_all_authority_hides_the_target() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &[]);
    let mut req = read_request(fixture.actor);
    req.ceiling = Ceiling::Restricted([action(UPDATE)].into_iter().collect());

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
}

#[test]
fn enforced_denies_that_remove_all_authority_hide_the_target() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target(DOC),
        &[READ, UPDATE],
    )];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ, UPDATE],
    )];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
    assert!(outcome.would_block.is_empty());
}

#[test]
fn a_denied_read_stays_visible_while_update_remains_allowed() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target(DOC),
        &[READ, UPDATE],
    )];
    let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);

    let read = evaluate(&read_request(fixture.actor), &facts).unwrap();
    let update = evaluate(
        &request(fixture.actor, action(UPDATE), DOC.parse().unwrap()),
        &facts,
    )
    .unwrap();

    assert_eq!(
        read.decision,
        Decision::Denied {
            because: DenyCause::DenyRuleEnforced
        }
    );
    assert_eq!(update.decision, Decision::Allowed);
}

#[test]
fn an_unrelated_kind_grant_does_not_disclose_the_target() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &["acta::workspace::read"],
    )];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
}

#[test]
fn descendant_actions_on_an_ancestor_grant_disclose_the_target() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[UPDATE],
    )];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
}

#[test]
fn the_nearest_level_wins_before_ref_outranks_selector() {
    let fixture = Fixture::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    let read = evaluate(&read_request(fixture.actor), &facts).unwrap();
    let update = evaluate(
        &request(fixture.actor, action(UPDATE), DOC.parse().unwrap()),
        &facts,
    )
    .unwrap();

    assert_eq!(read.decision, Decision::Allowed);
    assert_eq!(
        update.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );

    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ],
    )];
    let hidden = evaluate(
        &read_request(fixture.actor),
        &fixture.facts(&grants, &denies),
    )
    .unwrap();

    assert_eq!(hidden.decision, Decision::NotFound);
}

#[test]
fn an_unknown_group_grant_at_the_winning_tier_fails_instead_of_falling_back() {
    let fixture = Fixture::new();
    let editors = GroupId::new();
    let grants = [
        grant(Subject::Group(editors), ref_target(DOC), &[READ]),
        grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == editors
    ));
}

#[test]
fn an_indeterminate_group_grant_at_the_same_tier_fails() {
    let mut fixture = Fixture::new();
    let editors = GroupId::new();
    fixture
        .membership
        .set_group(editors, Membership::Indeterminate);
    let grants = [
        grant(fixture.alice.clone(), ref_target(DOC), &[READ]),
        grant(Subject::Group(editors), ref_target(DOC), &[UPDATE]),
    ];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == editors
    ));
}

#[test]
fn a_shadowed_unknown_group_grant_is_ignored() {
    let fixture = Fixture::new();
    let editors = GroupId::new();
    let grants = [
        grant(
            Subject::Group(editors),
            ref_target("acta::workspace::w1"),
            &[READ, UPDATE],
        ),
        grant(fixture.alice.clone(), ref_target(DOC), &[READ]),
    ];
    let facts = fixture.facts(&grants, &[]);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn an_unknown_group_deny_on_the_only_other_candidate_fails_closed() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target(DOC),
        &[READ, UPDATE],
    )];
    let denies = [
        deny(fixture.alice.clone(), ref_target(DOC), &[READ]),
        deny(Subject::Group(banned), ref_target(DOC), &[UPDATE]),
    ];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == banned
    ));
}

#[test]
fn an_unknown_group_deny_on_a_candidate_is_irrelevant_once_another_stays_visible() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target(DOC),
        &[READ, UPDATE, DELETE],
    )];
    let denies = [
        deny(fixture.alice.clone(), ref_target(DOC), &[READ]),
        deny(Subject::Group(banned), ref_target(DOC), &[DELETE]),
    ];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::DenyRuleEnforced
        }
    );
}

#[test]
fn an_unknown_group_deny_on_a_requested_action_that_is_not_effective_is_irrelevant() {
    let banned = GroupId::new();
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];

    for recorded in [None, Some(Membership::Indeterminate)] {
        let mut fixture = Fixture::new();
        if let Some(membership) = recorded {
            fixture.membership.set_group(banned, membership);
        }
        let visible = [grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];

        let denied = evaluate(
            &read_request(fixture.actor),
            &fixture.facts(&visible, &denies),
        )
        .unwrap();
        let hidden = evaluate(&read_request(fixture.actor), &fixture.facts(&[], &denies)).unwrap();

        assert_eq!(
            denied.decision,
            Decision::Denied {
                because: DenyCause::NotGranted
            }
        );
        assert_eq!(hidden.decision, Decision::NotFound);
    }
}

#[test]
fn an_unknown_group_grant_alone_at_the_winning_tier_fails_even_without_disclosing_actions() {
    let editors = GroupId::new();

    for group_actions in [&[][..], &["acta::workspace::read"][..]] {
        let fixture = Fixture::new();
        let grants = [
            grant(Subject::Group(editors), ref_target(DOC), group_actions),
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/**"),
                &[READ],
            ),
        ];

        let outcome = evaluate(&read_request(fixture.actor), &fixture.facts(&grants, &[]));

        assert!(matches!(
            outcome,
            Err(EvalError::GroupMembershipUnavailable { group }) if group == editors
        ));
    }
}

#[test]
fn an_unknown_group_grant_at_a_weaker_tier_of_the_winning_level_is_ignored() {
    let fixture = Fixture::new();
    let grants = [
        grant(fixture.alice.clone(), ref_target(DOC), &[READ]),
        grant(
            Subject::Group(GroupId::new()),
            path_target("acta::workspace::w1/folder::f1/document::d1"),
            &[UPDATE],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);

    let read = evaluate(&read_request(fixture.actor), &facts).unwrap();
    let update = evaluate(
        &request(fixture.actor, action(UPDATE), DOC.parse().unwrap()),
        &facts,
    )
    .unwrap();

    assert_eq!(read.decision, Decision::Allowed);
    assert_eq!(
        update.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
}

#[test]
fn audit_mode_hides_a_target_without_authority_and_exposes_no_evidence() {
    let fixture = Fixture::new();
    let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&[], &denies);
    let mut req = read_request(fixture.actor);
    req.deny_mode = DenyMode::Audit;

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::NotFound);
    assert!(outcome.would_block.is_empty());
}

#[test]
fn audit_mode_reports_denies_that_would_hide_the_target() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];
    let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];
    let facts = fixture.facts(&grants, &denies);
    let mut req = read_request(fixture.actor);
    req.deny_mode = DenyMode::Audit;

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(
        outcome.decision,
        Decision::Denied {
            because: DenyCause::NotGranted
        }
    );
    assert_eq!(outcome.would_block.len(), 1);
    assert!(outcome.would_block[0].actions.contains(&action(UPDATE)));
}

#[test]
fn cross_product_requests_fail_closed_with_a_technical_error() {
    let fixture = Fixture::new();
    let mut req = request(
        fixture.actor,
        action("custos::grants::read"),
        DOC.parse().unwrap(),
    );

    let outcome = evaluate(&req, &fixture.facts(&[], &[]));

    assert!(matches!(
        outcome,
        Err(EvalError::CrossProductRequest { .. })
    ));
    req.is_root = true;
    assert!(matches!(
        evaluate(&req, &fixture.facts(&[], &[])),
        Err(EvalError::CrossProductRequest { .. })
    ));
}

#[test]
fn a_ref_grant_keeps_authorizing_after_a_move_while_a_path_grant_does_not() {
    let fixture = Fixture::new();
    let new_path: ResourcePath = "acta::workspace::w1/folder::moved/document::d1"
        .parse()
        .unwrap();
    let grants = [
        grant(fixture.alice.clone(), ref_target(DOC), &[READ]),
        grant(
            fixture.alice.clone(),
            path_target("acta::workspace::w1/folder::old/document::d1"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);
    let mut req = read_request(fixture.actor);
    req.path = Some(new_path);

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn group_grants_apply_only_through_confirmed_membership_facts() {
    let mut fixture = Fixture::new();
    let editors = GroupId::new();
    let grants = [grant(Subject::Group(editors), ref_target(DOC), &[READ])];
    fixture.membership.set_group(editors, Membership::Member);
    let facts = fixture.facts(&grants, &[]);

    let member = evaluate(&read_request(fixture.actor), &facts).unwrap();
    assert_eq!(member.decision, Decision::Allowed);

    let mut outsider = Fixture::new();
    outsider
        .membership
        .set_group(editors, Membership::NotMember);
    let hidden = evaluate(&read_request(outsider.actor), &outsider.facts(&grants, &[])).unwrap();
    assert_eq!(hidden.decision, Decision::NotFound);

    let unknown = Fixture::new();
    let unavailable = evaluate(&read_request(unknown.actor), &unknown.facts(&grants, &[]));
    assert!(matches!(
        unavailable,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == editors
    ));

    let mut indeterminate = Fixture::new();
    indeterminate
        .membership
        .set_group(editors, Membership::Indeterminate);
    let unresolved = evaluate(
        &read_request(indeterminate.actor),
        &indeterminate.facts(&grants, &[]),
    );
    assert!(matches!(
        unresolved,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == editors
    ));
}

#[test]
fn principal_set_grants_apply_only_to_confirmed_members() {
    let reviewers: PrincipalSetId = "acta::workspace::w1::reviewers".parse().unwrap();
    let grants = [grant(
        Subject::PrincipalSet(reviewers.clone()),
        ref_target(DOC),
        &[READ],
    )];

    for (recorded, expected) in [
        (Some(Membership::Member), Decision::Allowed),
        (Some(Membership::Indeterminate), Decision::NotFound),
        (None, Decision::NotFound),
    ] {
        let mut fixture = Fixture::new();
        if let Some(membership) = recorded {
            fixture
                .membership
                .set_principal_set(reviewers.clone(), membership);
        }

        let outcome = evaluate(&read_request(fixture.actor), &fixture.facts(&grants, &[])).unwrap();

        assert_eq!(outcome.decision, expected);
    }
}

#[test]
fn a_non_root_existing_target_without_a_current_path_fails_closed() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &[]);
    let mut req = read_request(fixture.actor);
    req.path = None;

    let outcome = evaluate(&req, &facts);

    assert!(matches!(outcome, Err(EvalError::MissingAncestry)));
}

#[test]
fn root_without_a_current_path_still_allows_a_confirmed_existing_target() {
    let fixture = Fixture::new();
    let mut req = read_request(fixture.actor);
    req.is_root = true;
    req.path = None;

    let outcome = evaluate(&req, &fixture.facts(&[], &[])).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn a_confirmed_resource_with_no_ancestors_evaluates_from_its_single_segment() {
    let fixture = Fixture::new();
    let workspace: ResourceRef = "acta::workspace::w1".parse().unwrap();
    let read_workspace = action("acta::workspace::read");
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &["acta::workspace::read"],
    )];
    let facts = fixture.facts(&grants, &[]);
    let mut req = request(fixture.actor, read_workspace, workspace);
    req.path = Some("acta::workspace::w1".parse().unwrap());

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);

    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &["acta::workspace::read"],
    )];
    let hidden = evaluate(&req, &fixture.facts(&grants, &denies)).unwrap();

    assert_eq!(hidden.decision, Decision::NotFound);
}

#[test]
fn a_relevant_group_deny_with_missing_membership_fails_closed() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == banned
    ));
}

#[test]
fn a_relevant_group_deny_with_indeterminate_membership_fails_closed() {
    let mut fixture = Fixture::new();
    let banned = GroupId::new();
    fixture
        .membership
        .set_group(banned, Membership::Indeterminate);
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { group }) if group == banned
    ));
}

#[test]
fn an_ancestor_group_deny_with_missing_membership_fails_closed() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(
        Subject::Group(banned),
        path_target("acta::workspace::w1"),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { .. })
    ));
}

#[test]
fn audit_mode_fails_closed_on_a_relevant_group_deny_with_missing_membership() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);
    let mut req = read_request(fixture.actor);
    req.deny_mode = DenyMode::Audit;

    let outcome = evaluate(&req, &facts);

    assert!(matches!(
        outcome,
        Err(EvalError::GroupMembershipUnavailable { .. })
    ));
}

#[test]
fn a_confirmed_non_member_group_deny_is_skipped() {
    let mut fixture = Fixture::new();
    let banned = GroupId::new();
    fixture.membership.set_group(banned, Membership::NotMember);
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn an_irrelevant_group_deny_with_missing_membership_is_ignored() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [
        deny(
            Subject::Group(banned),
            ref_target(DOC),
            &["acta::document::delete"],
        ),
        deny(
            Subject::Group(banned),
            ref_target("acta::document::other"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn disabled_mode_ignores_a_relevant_group_deny_with_missing_membership() {
    let fixture = Fixture::new();
    let banned = GroupId::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &denies);
    let mut req = read_request(fixture.actor);
    req.deny_mode = DenyMode::Disabled;

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn a_principal_set_deny_with_indeterminate_membership_does_not_block() {
    let mut fixture = Fixture::new();
    let reviewers: PrincipalSetId = "acta::workspace::w1::reviewers".parse().unwrap();
    fixture
        .membership
        .set_principal_set(reviewers.clone(), Membership::Indeterminate);
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(
        Subject::PrincipalSet(reviewers),
        ref_target(DOC),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);

    let outcome = evaluate(&read_request(fixture.actor), &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
}

#[test]
fn unavailable_existence_is_a_typed_technical_error_never_a_decision() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];

    for cause in [
        FactFailure::Timeout,
        FactFailure::Provider,
        FactFailure::Internal,
        FactFailure::Indeterminate,
    ] {
        let mut req = read_request(fixture.actor);
        req.existence = Existence::Unavailable(cause);

        let outcome = evaluate(&req, &fixture.facts(&grants, &[]));

        assert_eq!(outcome, Err(EvalError::FactsUnavailable { cause }));
        assert!(!outcome.unwrap_err().to_string().contains(DOC));
    }
}

#[test]
fn membership_facts_bound_to_another_principal_are_rejected() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let foreign = MembershipFacts::new(PrincipalId::new());
    let facts = EvaluationFacts {
        grants: &grants,
        denies: &[],
        membership: &foreign,
    };

    let outcome = evaluate(&read_request(fixture.actor), &facts);

    assert!(matches!(outcome, Err(EvalError::InconsistentFacts { .. })));

    let mut root = read_request(fixture.actor);
    root.is_root = true;

    assert!(matches!(
        evaluate(&root, &facts),
        Err(EvalError::InconsistentFacts { .. })
    ));
}

#[test]
fn a_multi_product_ceiling_narrows_without_adding_authority() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let facts = fixture.facts(&grants, &[]);
    let mut req = read_request(fixture.actor);
    req.ceiling = Ceiling::Restricted(
        [action("custos::grants::read"), action(READ)]
            .into_iter()
            .collect(),
    );

    assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);

    req.ceiling = Ceiling::Restricted(
        [action("custos::grants::read"), action(UPDATE)]
            .into_iter()
            .collect(),
    );

    assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::NotFound);
}

#[test]
fn audit_mode_keeps_authority_and_reports_the_blocking_deny() {
    let fixture = Fixture::new();
    let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);
    let mut req = read_request(fixture.actor);
    req.deny_mode = DenyMode::Audit;

    let outcome = evaluate(&req, &facts).unwrap();

    assert_eq!(outcome.decision, Decision::Allowed);
    assert_eq!(outcome.would_block.len(), 1);
    assert_eq!(outcome.would_block[0].subject, fixture.alice);
    assert_eq!(
        outcome.would_block[0].target,
        ref_target("acta::workspace::w1")
    );
    assert!(outcome.would_block[0].actions.contains(&action(READ)));
}
