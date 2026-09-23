//! Public-surface behavioral tests for batch evaluation (EVAL-4): one actor
//! and one action over many targets, each result matching the single-target
//! contract except that an unavailable target collapses to `NotFound`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_custos::eval::{
    BatchRequest, BatchTarget, Ceiling, Decision, DenyMode, EvalError, EvalRequest, Evaluated,
    EvaluationFacts, Existence, FactFailure, MembershipFacts, Subject, evaluate, evaluate_batch,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use support::{Fixture, READ, UPDATE, action, deny, grant, ref_target, selector_target};

fn target(id: &str, existence: Existence) -> BatchTarget {
    BatchTarget {
        target: format!("acta::document::{id}").parse().unwrap(),
        path: Some(
            format!("acta::workspace::w1/folder::f1/document::{id}")
                .parse()
                .unwrap(),
        ),
        existence,
    }
}

fn batch(actor: PrincipalId, deny_mode: DenyMode, targets: Vec<BatchTarget>) -> BatchRequest {
    BatchRequest {
        actor,
        is_root: false,
        action: action(READ),
        ceiling: Ceiling::Unrestricted,
        deny_mode,
        targets,
    }
}

fn single(request: &BatchRequest, entry: &BatchTarget) -> EvalRequest {
    EvalRequest {
        actor: request.actor,
        is_root: request.is_root,
        action: request.action.clone(),
        target: entry.target.clone(),
        path: entry.path.clone(),
        existence: entry.existence,
        ceiling: request.ceiling.clone(),
        deny_mode: request.deny_mode,
    }
}

/// Asserts that every batch result equals the single evaluation of the same
/// target, except that an unavailable target is `NotFound` without evidence.
fn assert_matches_single_evaluations(request: &BatchRequest, facts: &EvaluationFacts<'_>) {
    let results = evaluate_batch(request, facts).unwrap();

    assert_eq!(results.len(), request.targets.len());

    for (entry, result) in request.targets.iter().zip(&results) {
        let expected = match evaluate(&single(request, entry), facts) {
            Err(EvalError::FactsUnavailable { .. }) => Ok(Evaluated {
                decision: Decision::NotFound,
                would_block: Vec::new(),
            }),
            other => other,
        };

        assert_eq!(result, &expected, "target {}", entry.target);
    }
}

#[test]
fn batch_results_equal_single_evaluations_in_input_order_in_every_mode() {
    let fixture = Fixture::new();
    let unknown_group = GroupId::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::allowed"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::update_only"),
            &[UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::read_denied"),
            &[READ, UPDATE],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::hidden_by_deny"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::unavailable"),
            &[READ],
        ),
        grant(
            Subject::Group(unknown_group),
            ref_target("acta::document::unknown_group"),
            &[READ],
        ),
        grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        ),
    ];
    let denies = [
        deny(
            fixture.alice.clone(),
            ref_target("acta::document::read_denied"),
            &[READ],
        ),
        deny(
            fixture.alice.clone(),
            ref_target("acta::document::hidden_by_deny"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &denies);

    let mut missing_ancestry = target("allowed", Existence::Exists);
    missing_ancestry.path = None;
    let mut cross_product = target("allowed", Existence::Exists);
    cross_product.target = "custos::grant::g1".parse().unwrap();
    cross_product.path = None;

    let targets = vec![
        target("update_only", Existence::Exists),
        target("allowed", Existence::Exists),
        target("unavailable", Existence::Unavailable(FactFailure::Timeout)),
        target("read_denied", Existence::Exists),
        target("hidden_by_deny", Existence::Exists),
        target("gone", Existence::Missing),
        target("unknown_group", Existence::Exists),
        missing_ancestry,
        cross_product,
        target("no_grant", Existence::Exists),
    ];

    for mode in [DenyMode::Disabled, DenyMode::Audit, DenyMode::Enforced] {
        assert_matches_single_evaluations(&batch(fixture.actor, mode, targets.clone()), &facts);
    }
}

#[test]
fn batch_keeps_per_target_outcomes_and_errors_separate() {
    let fixture = Fixture::new();
    let unknown_group = GroupId::new();
    let grants = [
        grant(
            fixture.alice.clone(),
            ref_target("acta::document::allowed"),
            &[READ],
        ),
        grant(
            Subject::Group(unknown_group),
            ref_target("acta::document::unknown_group"),
            &[READ],
        ),
    ];
    let facts = fixture.facts(&grants, &[]);
    let mut missing_ancestry = target("allowed", Existence::Exists);
    missing_ancestry.path = None;
    let request = batch(
        fixture.actor,
        DenyMode::Enforced,
        vec![
            target("unknown_group", Existence::Exists),
            missing_ancestry,
            target("allowed", Existence::Exists),
            target("allowed", Existence::Unavailable(FactFailure::Provider)),
        ],
    );

    let results = evaluate_batch(&request, &facts).unwrap();

    assert_eq!(results.len(), 4);
    assert!(matches!(
        results[0],
        Err(EvalError::GroupMembershipUnavailable { group }) if group == unknown_group
    ));
    assert_eq!(results[1], Err(EvalError::MissingAncestry));
    assert_eq!(results[2].as_ref().unwrap().decision, Decision::Allowed);
    assert_eq!(results[3].as_ref().unwrap().decision, Decision::NotFound);
}

#[test]
fn an_unavailable_target_is_not_found_without_evidence_in_audit_mode() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        selector_target("acta::workspace::w1/**"),
        &[READ],
    )];
    let denies = [deny(
        fixture.alice.clone(),
        ref_target("acta::workspace::w1"),
        &[READ],
    )];
    let facts = fixture.facts(&grants, &denies);
    let request = batch(
        fixture.actor,
        DenyMode::Audit,
        vec![
            target("available", Existence::Exists),
            target("unavailable", Existence::Unavailable(FactFailure::Internal)),
        ],
    );

    let results = evaluate_batch(&request, &facts).unwrap();

    assert_eq!(results.len(), 2);
    let available = results[0].as_ref().unwrap();
    assert_eq!(available.decision, Decision::Allowed);
    assert_eq!(available.would_block.len(), 1);
    assert_eq!(
        results[1],
        Ok(Evaluated {
            decision: Decision::NotFound,
            would_block: Vec::new(),
        })
    );
}

#[test]
fn root_batches_follow_physical_existence_per_target() {
    let fixture = Fixture::new();
    let mut without_path = target("no_path", Existence::Exists);
    without_path.path = None;
    let mut request = batch(
        fixture.actor,
        DenyMode::Enforced,
        vec![
            target("present", Existence::Exists),
            target("gone", Existence::Missing),
            target("unreachable", Existence::Unavailable(FactFailure::Timeout)),
            without_path,
        ],
    );
    request.is_root = true;
    request.ceiling = Ceiling::Restricted(Default::default());

    let decisions: Vec<Decision> = evaluate_batch(&request, &fixture.facts(&[], &[]))
        .unwrap()
        .into_iter()
        .map(|result| result.unwrap().decision)
        .collect();

    assert_eq!(
        decisions,
        vec![
            Decision::Allowed,
            Decision::NotFound,
            Decision::NotFound,
            Decision::Allowed,
        ]
    );
}

#[test]
fn membership_facts_for_another_principal_fail_the_whole_batch() {
    let fixture = Fixture::new();
    let grants = [grant(
        fixture.alice.clone(),
        ref_target("acta::document::allowed"),
        &[READ],
    )];
    let foreign = MembershipFacts::new(PrincipalId::new());
    let facts = EvaluationFacts {
        grants: &grants,
        denies: &[],
        membership: &foreign,
    };
    let request = batch(
        fixture.actor,
        DenyMode::Enforced,
        vec![target("allowed", Existence::Exists)],
    );

    assert!(matches!(
        evaluate_batch(&request, &facts),
        Err(EvalError::InconsistentFacts { .. })
    ));
}

#[test]
fn an_empty_batch_returns_no_results() {
    let fixture = Fixture::new();
    let request = batch(fixture.actor, DenyMode::Enforced, Vec::new());

    assert_eq!(
        evaluate_batch(&request, &fixture.facts(&[], &[])).unwrap(),
        Vec::new()
    );
}
