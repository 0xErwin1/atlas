//! The pure V2 decision procedure over caller-supplied facts.
//!
//! Decision order: validate facts, resolve physical existence, apply the
//! root exemption, compute the actor's effective actions on the target, then
//! apply denies per mode. Non-root evaluations of existing resources require
//! the target's current ancestry (a resource with no ancestors carries a
//! one-segment path), so an ancestor-scoped deny cannot be bypassed by
//! omitting the path.
//!
//! The allow side walks the resource chain from the target toward the root
//! and stops at the nearest level holding an applicable grant — the same
//! chain-walk shadowing the V1 `resolve()` applies ("first segment with
//! candidates wins"). Within that level the core [`Specificity`] precedence
//! picks the winning tier, independent of the requested action, the winning
//! tier's action sets are unioned, and the credential ceiling intersects the
//! union. There is no fallback to a weaker tier or a farther level.
//!
//! The effective actions decide the outcome: the requested action present
//! is `Allowed`, any other effective action is `Denied`, and none is
//! `NotFound`, so an existing target the actor cannot act on is
//! indistinguishable from a missing one.

use std::cmp::Reverse;
use std::collections::BTreeSet;

use crate::eval::EvalError;
use crate::eval::model::{
    ActionSet, Ceiling, DenyRule, Existence, Grant, GrantTarget, Membership, MembershipFacts,
    Subject,
};
use crate::ids::{GroupId, PrincipalId};
use atlas_core::ids::{ActionId, PathSegment, ResourcePath, ResourceRef, Specificity};

/// Deny evaluation mode. Configuration of one evaluation, never persisted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyMode {
    /// Deny facts are ignored and no would-block evidence is produced.
    Disabled,
    /// The decision ignores denies; matching denies that would change it
    /// are returned as evidence. Audit never turns a denied or hidden
    /// request into an allow.
    Audit,
    /// A deny matching an action on the target or any ancestor removes that
    /// action from the actor's effective authority. The root principal is
    /// exempt.
    Enforced,
}

/// One authorization request: one principal, one action, one target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvalRequest {
    /// The acting Custos principal. Groups and principal sets are grant and
    /// deny subjects only; they never act.
    pub actor: PrincipalId,
    /// Caller-asserted root identity, established by authentication. The
    /// evaluator never infers it, and no other principal — including
    /// platform administrators — receives the exemption.
    pub is_root: bool,
    pub action: ActionId,
    pub target: ResourceRef,
    /// The current canonical path of the target. Required for non-root
    /// evaluations of existing resources: without the ancestry, an
    /// ancestor-scoped deny could be bypassed. A resource with no ancestors
    /// carries a one-segment path. Ref grants and path/selector targets
    /// only ever match this supplied current path, never a historical one.
    pub path: Option<ResourcePath>,
    pub existence: Existence,
    pub ceiling: Ceiling,
    pub deny_mode: DenyMode,
}

/// The facts one evaluation consumes. The caller resolves grants, deny
/// rules and membership from storage; the evaluator is pure.
#[derive(Debug, Clone, Copy)]
pub struct EvaluationFacts<'a> {
    pub grants: &'a [Grant],
    pub denies: &'a [DenyRule],
    pub membership: &'a MembershipFacts,
}

/// The allow/deny/not-found outcome of one evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allowed,
    Denied { because: DenyCause },
    NotFound,
}

/// Why a visible request was denied. Negative outcomes carry no resource
/// metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DenyCause {
    /// An enforced deny rule removed the requested action on the target or
    /// an ancestor, while another effective action keeps the target
    /// visible.
    DenyRuleEnforced,
    /// The requested action is not among the effective actions at the
    /// winning tier and level after the ceiling, but another action is.
    NotGranted,
}

/// Audit-mode evidence: a deny rule that would have changed the decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyEvidence {
    pub subject: Subject,
    pub target: GrantTarget,
    pub actions: ActionSet,
}

/// The full evaluation result: the decision plus audit-mode evidence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluated {
    pub decision: Decision,
    /// Would-block evidence in [`DenyMode::Audit`]; empty in every other
    /// mode and for every `NotFound` decision.
    pub would_block: Vec<DenyEvidence>,
}

impl Evaluated {
    pub(super) fn new(decision: Decision) -> Self {
        Self {
            decision,
            would_block: Vec::new(),
        }
    }
}

/// A borrowed view of one single-target request. Single and batch
/// evaluation share it so the facts common to many targets are never cloned
/// per target.
pub(super) struct RequestView<'r> {
    pub(super) actor: PrincipalId,
    pub(super) is_root: bool,
    pub(super) action: &'r ActionId,
    pub(super) target: &'r ResourceRef,
    pub(super) path: Option<&'r ResourcePath>,
    pub(super) existence: Existence,
    pub(super) ceiling: &'r Ceiling,
    pub(super) deny_mode: DenyMode,
}

impl EvalRequest {
    fn view(&self) -> RequestView<'_> {
        RequestView {
            actor: self.actor,
            is_root: self.is_root,
            action: &self.action,
            target: &self.target,
            path: self.path.as_ref(),
            existence: self.existence,
            ceiling: &self.ceiling,
            deny_mode: self.deny_mode,
        }
    }
}

/// Evaluates one request against one set of facts. Pure and stateless.
pub fn evaluate(
    request: &EvalRequest,
    facts: &EvaluationFacts<'_>,
) -> Result<Evaluated, EvalError> {
    let view = request.view();

    validate_request(&view)?;
    validate_membership_binding(request.actor, facts.membership)?;

    evaluate_validated(&view, facts)
}

/// Evaluates a request whose target facts and membership binding were
/// already validated.
pub(super) fn evaluate_validated(
    request: &RequestView<'_>,
    facts: &EvaluationFacts<'_>,
) -> Result<Evaluated, EvalError> {
    match request.existence {
        Existence::Missing => return Ok(Evaluated::new(Decision::NotFound)),
        Existence::Unavailable(cause) => return Err(EvalError::FactsUnavailable { cause }),
        Existence::Exists => {}
    }

    if request.is_root {
        return Ok(Evaluated::new(Decision::Allowed));
    }

    let chain = chain_nodes(request)?;
    let effective = effective_actions(request, facts, &chain)?;
    let unenforced = decision_without_denies(request, &effective);

    if request.deny_mode == DenyMode::Disabled {
        return Ok(Evaluated::new(unenforced));
    }

    let enforced = enforced_outcome(request, facts, &chain, &effective)?;

    if request.deny_mode == DenyMode::Enforced {
        return Ok(Evaluated::new(enforced.decision));
    }

    let would_block = if enforced.decision == unenforced {
        Vec::new()
    } else {
        enforced
            .blocking
            .into_iter()
            .map(|rule| DenyEvidence {
                subject: rule.subject().clone(),
                target: rule.target().clone(),
                actions: rule.actions().clone(),
            })
            .collect()
    };

    Ok(Evaluated {
        decision: unenforced,
        would_block,
    })
}

/// Validates the per-target facts of a request: the action product and the
/// consistency of the supplied path with the target.
pub(super) fn validate_request(request: &RequestView<'_>) -> Result<(), EvalError> {
    if request.action.product() != request.target.product() {
        return Err(EvalError::CrossProductRequest {
            target_product: request.target.product().to_string(),
            action_product: request.action.product().to_string(),
        });
    }

    if let Some(path) = request.path {
        validate_path_consistency(path, request.target)?;
    }

    Ok(())
}

/// Rejects membership facts resolved for another principal than `actor`.
pub(super) fn validate_membership_binding(
    actor: PrincipalId,
    membership: &MembershipFacts,
) -> Result<(), EvalError> {
    if membership.actor() != actor {
        return Err(EvalError::InconsistentFacts {
            detail: "membership facts were resolved for a different principal than the actor"
                .to_string(),
        });
    }

    Ok(())
}

fn validate_path_consistency(path: &ResourcePath, target: &ResourceRef) -> Result<(), EvalError> {
    if path.product() != target.product() {
        return Err(EvalError::InconsistentFacts {
            detail: format!(
                "path product `{}` does not match target product `{}`",
                path.product(),
                target.product()
            ),
        });
    }

    if path.leaf_ref() != *target {
        return Err(EvalError::InconsistentFacts {
            detail: format!(
                "path leaf `{}` does not match target `{target}`",
                path.leaf_ref()
            ),
        });
    }

    Ok(())
}

/// One node of the canonical resource chain: index 0 is the target, higher
/// indices are its ancestors toward the root.
pub(super) struct ChainNode {
    reference: ResourceRef,
    path: Option<ResourcePath>,
}

fn chain_nodes(request: &RequestView<'_>) -> Result<Vec<ChainNode>, EvalError> {
    let Some(path) = request.path else {
        return Err(EvalError::MissingAncestry);
    };

    chain_from_path(path)
}

/// Builds the canonical chain of `path`, from its leaf toward its root.
pub(super) fn chain_from_path(path: &ResourcePath) -> Result<Vec<ChainNode>, EvalError> {
    let descendants: Vec<PathSegment> = path.segments().skip(1).cloned().collect();
    let mut chain = Vec::with_capacity(descendants.len() + 1);

    for depth in (0..=descendants.len()).rev() {
        let rest = descendants.iter().take(depth).cloned().collect();
        let prefix =
            ResourcePath::new(path.product(), path.root().clone(), rest).map_err(|_| {
                EvalError::InconsistentFacts {
                    detail: "resource path prefix failed revalidation".to_string(),
                }
            })?;

        chain.push(ChainNode {
            reference: prefix.leaf_ref(),
            path: Some(prefix),
        });
    }

    Ok(chain)
}

fn covers(target: &GrantTarget, node: &ChainNode) -> bool {
    match target {
        GrantTarget::Ref(reference) => *reference == node.reference,
        GrantTarget::Path(path) => node.path.as_ref() == Some(path),
        GrantTarget::Selector(selector) => node
            .path
            .as_ref()
            .is_some_and(|path| selector.matches(path)),
    }
}

pub(super) fn covers_chain(target: &GrantTarget, chain: &[ChainNode]) -> bool {
    chain.iter().any(|node| covers(target, node))
}

/// The chain level of the nearest node `target` covers; 0 is the target.
pub(super) fn level_of(target: &GrantTarget, chain: &[ChainNode]) -> Option<usize> {
    chain.iter().position(|node| covers(target, node))
}

/// Keeps the candidates at the nearest chain level and, within it, the
/// strongest specificity tier. There is no fallback to a weaker tier or a
/// farther level.
pub(super) fn winners<T>(candidates: Vec<(usize, Specificity, T)>) -> Vec<T> {
    let Some(best) = candidates
        .iter()
        .map(|(level, specificity, _)| (Reverse(*level), *specificity))
        .max()
    else {
        return Vec::new();
    };

    candidates
        .into_iter()
        .filter(|(level, specificity, _)| (Reverse(*level), *specificity) == best)
        .map(|(.., candidate)| candidate)
        .collect()
}

/// Whether a grant or deny subject reaches the actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Reach {
    Applies,
    DoesNotApply,
    /// A group subject whose membership fact is missing or indeterminate.
    Unknown(GroupId),
}

/// Resolves a fact subject against the actor. Group subjects without a
/// confirmed fact stay [`Reach::Unknown`] so the caller can fail closed
/// when they matter; principal sets without a confirmed fact never apply.
pub(super) fn reach(subject: &Subject, actor: PrincipalId, membership: &MembershipFacts) -> Reach {
    let applies = match subject {
        Subject::Principal(principal) => *principal == actor,
        Subject::Group(group) => match membership.group_membership(group) {
            Some(Membership::Member) => true,
            Some(Membership::NotMember) => false,
            Some(Membership::Indeterminate) | None => return Reach::Unknown(*group),
        },
        Subject::PrincipalSet(set) => membership.principal_set(set) == Membership::Member,
    };

    if applies {
        Reach::Applies
    } else {
        Reach::DoesNotApply
    }
}

/// The actor's effective actions on the target before denies: the union of
/// the winning grants at the nearest level and strongest tier, intersected
/// with the credential ceiling, keeping only the requested action and
/// actions of the target's own product and kind.
///
/// A group grant with unknown membership fails closed when it sits at or
/// above the winning level and tier, because its membership could change
/// which grants win. Shadowed unknown grants are ignored.
fn effective_actions(
    request: &RequestView<'_>,
    facts: &EvaluationFacts<'_>,
    chain: &[ChainNode],
) -> Result<BTreeSet<ActionId>, EvalError> {
    let candidates: Vec<(usize, Specificity, (&Grant, Reach))> = facts
        .grants
        .iter()
        .filter_map(|grant| {
            let grant_reach = reach(grant.subject(), request.actor, facts.membership);
            if grant_reach == Reach::DoesNotApply {
                return None;
            }

            level_of(grant.target(), chain)
                .map(|level| (level, grant.target().specificity(), (grant, grant_reach)))
        })
        .collect();

    let winners = winners(candidates);

    if let Some(group) = winners
        .iter()
        .find_map(|(_, grant_reach)| match grant_reach {
            Reach::Unknown(group) => Some(*group),
            Reach::Applies | Reach::DoesNotApply => None,
        })
    {
        return Err(EvalError::GroupMembershipUnavailable { group });
    }

    let effective = winners
        .iter()
        .flat_map(|(grant, _)| grant.actions().iter())
        .filter(|action| request.ceiling.permits(action))
        .filter(|action| discloses_target(action, request))
        .cloned()
        .collect();

    Ok(effective)
}

/// Whether holding `action` makes the target discoverable. Actions of
/// another product or of another resource kind than the target describe
/// something else and never disclose it; an ancestor grant's descendant
/// actions count because they are compared with the target's kind.
fn discloses_target(action: &ActionId, request: &RequestView<'_>) -> bool {
    action == request.action
        || (action.product() == request.target.product() && action.kind() == request.target.kind())
}

fn decision_without_denies(request: &RequestView<'_>, effective: &BTreeSet<ActionId>) -> Decision {
    if effective.contains(request.action) {
        Decision::Allowed
    } else if effective.is_empty() {
        Decision::NotFound
    } else {
        Decision::Denied {
            because: DenyCause::NotGranted,
        }
    }
}

/// The enforced decision plus the deny rules that shaped it.
struct EnforcedOutcome<'f> {
    decision: Decision,
    blocking: Vec<&'f DenyRule>,
}

/// The deny rules applying to one effective action and the first relevant
/// group rule whose membership is unknown.
struct ActionDenials<'f> {
    applying: Vec<&'f DenyRule>,
    unknown: Option<GroupId>,
}

/// Collects the deny rules relevant to `action`: they contain the action
/// and cover the target or an ancestor. Membership is consulted only for
/// relevant rules.
fn action_denials<'f>(
    action: &ActionId,
    request: &RequestView<'_>,
    facts: &EvaluationFacts<'f>,
    chain: &[ChainNode],
) -> ActionDenials<'f> {
    let mut denials = ActionDenials {
        applying: Vec::new(),
        unknown: None,
    };

    let relevant = facts
        .denies
        .iter()
        .filter(|rule| rule.actions().contains(action) && covers_chain(rule.target(), chain));

    for rule in relevant {
        match reach(rule.subject(), request.actor, facts.membership) {
            Reach::Applies => denials.applying.push(rule),
            Reach::DoesNotApply => {}
            Reach::Unknown(group) => {
                denials.unknown.get_or_insert(group);
            }
        }
    }

    denials
}

/// Applies enforced denies to the effective actions.
///
/// The requested action is checked first: every relevant group rule on it
/// needs a confirmed membership fact, and when no rule applies the request
/// is allowed without consulting any other fact. Otherwise the target stays
/// visible as soon as one other effective action is free of applying
/// denies; unknown group denies on other actions fail closed only when no
/// such action exists, because only then could they decide between
/// `Denied` and `NotFound`.
fn enforced_outcome<'f>(
    request: &RequestView<'_>,
    facts: &EvaluationFacts<'f>,
    chain: &[ChainNode],
    effective: &BTreeSet<ActionId>,
) -> Result<EnforcedOutcome<'f>, EvalError> {
    let mut blocking = Vec::new();

    let because = if effective.contains(request.action) {
        let denials = action_denials(request.action, request, facts, chain);

        if let Some(group) = denials.unknown {
            return Err(EvalError::GroupMembershipUnavailable { group });
        }

        if denials.applying.is_empty() {
            return Ok(EnforcedOutcome {
                decision: Decision::Allowed,
                blocking,
            });
        }

        blocking = denials.applying;
        DenyCause::DenyRuleEnforced
    } else {
        DenyCause::NotGranted
    };

    let mut hiding: Vec<&DenyRule> = Vec::new();
    let mut unknown = None;

    for action in effective.iter().filter(|action| *action != request.action) {
        let denials = action_denials(action, request, facts, chain);

        if denials.applying.is_empty() && denials.unknown.is_none() {
            return Ok(EnforcedOutcome {
                decision: Decision::Denied { because },
                blocking,
            });
        }

        if denials.applying.is_empty() {
            unknown = unknown.or(denials.unknown);
        }

        hiding.extend(denials.applying);
    }

    if let Some(group) = unknown {
        return Err(EvalError::GroupMembershipUnavailable { group });
    }

    for rule in hiding {
        if !blocking.iter().any(|seen| std::ptr::eq(*seen, rule)) {
            blocking.push(rule);
        }
    }

    Ok(EnforcedOutcome {
        decision: Decision::NotFound,
        blocking,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::FactFailure;
    use crate::eval::model::{CeilingActions, DenyRule, Grant};

    fn action(raw: &str) -> ActionId {
        raw.parse().unwrap()
    }

    fn actions(raw: &[&str]) -> ActionSet {
        ActionSet::new(raw.iter().map(|raw| action(raw))).unwrap()
    }

    fn ceiling(raw: &[&str]) -> Ceiling {
        Ceiling::Restricted(raw.iter().map(|raw| action(raw)).collect())
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

    /// One test fixture: the acting principal `alice`, another principal
    /// `bob`, and the membership facts bound to the actor.
    struct Fixture {
        actor: PrincipalId,
        alice: Subject,
        bob: Subject,
        membership: MembershipFacts,
    }

    impl Fixture {
        fn new() -> Self {
            let actor = PrincipalId::new();

            Self {
                actor,
                alice: Subject::Principal(actor),
                bob: Subject::Principal(PrincipalId::new()),
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

    fn read_request(actor: PrincipalId) -> EvalRequest {
        request(actor, action(READ), DOC.parse().unwrap())
    }

    fn doc_path() -> ResourcePath {
        "acta::workspace::w1/folder::f1/document::d1"
            .parse()
            .unwrap()
    }

    #[test]
    fn an_exact_ref_beats_path_and_selector_at_the_same_level() {
        let fixture = Fixture::new();
        let grants = [
            grant(fixture.alice.clone(), ref_target(DOC), &[]),
            grant(
                fixture.alice.clone(),
                path_target("acta::workspace::w1/folder::f1/document::d1"),
                &[READ],
            ),
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/**"),
                &[READ],
            ),
        ];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(UPDATE), DOC.parse().unwrap());
        req.path = Some(doc_path());

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::NotFound);

        let grants = [
            grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE]),
            grant(
                fixture.alice.clone(),
                path_target("acta::workspace::w1/folder::f1/document::d1"),
                &[READ],
            ),
        ];
        let facts = fixture.facts(&grants, &[]);

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);
    }

    #[test]
    fn selector_literal_count_outranks_open_ended_and_wildcard_ties() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(UPDATE), DOC.parse().unwrap());
        req.path = Some(doc_path());

        let nearer_more_literals = [
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/folder::f1/**"),
                &[],
            ),
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/**"),
                &[UPDATE],
            ),
        ];
        let facts = fixture.facts(&nearer_more_literals, &[]);
        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::NotFound);

        let closed_beats_open = [
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/folder::f1/*"),
                &[UPDATE],
            ),
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/folder::f1/**"),
                &[],
            ),
        ];
        let facts = fixture.facts(&closed_beats_open, &[]);
        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);

        let wildcard = selector_target("acta::workspace::w1/*/*");
        let exact_selector_beats_wildcard = [
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/folder::f1/document::d1"),
                &[],
            ),
            grant(fixture.alice.clone(), wildcard, &[UPDATE]),
        ];
        let facts = fixture.facts(&exact_selector_beats_wildcard, &[]);
        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::NotFound);
    }

    #[test]
    fn equal_specificity_grants_union_their_action_sets() {
        let fixture = Fixture::new();
        let grants = [
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/*/*"),
                &[READ],
            ),
            grant(
                fixture.alice.clone(),
                selector_target("acta::*/folder::f1/*"),
                &[UPDATE],
            ),
        ];
        let facts = fixture.facts(&grants, &[]);

        let mut read = request(fixture.actor, action(READ), DOC.parse().unwrap());
        read.path = Some(doc_path());
        let mut update = request(fixture.actor, action(UPDATE), DOC.parse().unwrap());
        update.path = Some(doc_path());

        let read = evaluate(&read, &facts).unwrap();
        let update = evaluate(&update, &facts).unwrap();

        assert_eq!(read.decision, Decision::Allowed);
        assert_eq!(update.decision, Decision::Allowed);
    }

    #[test]
    fn winning_tier_lacking_the_action_denies_without_lower_tier_fallback() {
        let fixture = Fixture::new();
        let grants = [
            grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE]),
            grant(
                fixture.alice.clone(),
                path_target("acta::workspace::w1/folder::f1/document::d1"),
                &[READ],
            ),
            grant(
                fixture.alice.clone(),
                selector_target("acta::workspace::w1/**"),
                &[READ],
            ),
        ];
        let facts = fixture.facts(&grants, &[]);

        let outcome = evaluate(
            &request(fixture.actor, action(READ), DOC.parse().unwrap()),
            &facts,
        )
        .unwrap();

        assert_eq!(
            outcome.decision,
            Decision::Denied {
                because: DenyCause::NotGranted
            }
        );
    }

    #[test]
    fn nearer_allow_level_shadows_a_farther_ancestor_level() {
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
                &[READ],
            ),
        ];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(UPDATE), DOC.parse().unwrap());
        req.path = Some(doc_path());

        assert_eq!(
            evaluate(&req, &facts).unwrap().decision,
            Decision::Denied {
                because: DenyCause::NotGranted
            }
        );
    }

    #[test]
    fn ancestor_ref_grants_inherit_to_descendants() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &[READ],
        )];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.path = Some(doc_path());

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);
    }

    #[test]
    fn open_ended_selector_grants_cover_deep_descendants() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        )];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.path = Some(doc_path());

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);
    }

    #[test]
    fn a_restricted_ceiling_narrows_the_allow_union() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ, UPDATE],
        )];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(UPDATE), DOC.parse().unwrap());
        req.ceiling = ceiling(&[READ]);

        assert_eq!(
            evaluate(&req, &facts).unwrap().decision,
            Decision::Denied {
                because: DenyCause::NotGranted
            }
        );
    }

    #[test]
    fn an_empty_ceiling_never_grants() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        )];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.ceiling = Ceiling::Restricted(CeilingActions::default());

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::NotFound);
    }

    #[test]
    fn a_ceiling_never_grants_beyond_the_allow_union() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            selector_target("acta::workspace::w1/**"),
            &[READ],
        )];
        let facts = fixture.facts(&grants, &[]);
        let mut req = request(fixture.actor, action(DELETE), DOC.parse().unwrap());
        req.ceiling = ceiling(&[READ, UPDATE, DELETE]);

        assert_eq!(
            evaluate(&req, &facts).unwrap().decision,
            Decision::Denied {
                because: DenyCause::NotGranted
            }
        );
    }

    #[test]
    fn root_allows_an_existing_valid_request_without_grants_or_ceiling() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(DELETE), DOC.parse().unwrap());
        req.is_root = true;
        req.ceiling = Ceiling::Restricted(CeilingActions::default());

        assert_eq!(
            evaluate(&req, &fixture.facts(&[], &[])).unwrap().decision,
            Decision::Allowed
        );
    }

    #[test]
    fn root_is_exempt_from_enforced_denies() {
        let fixture = Fixture::new();
        let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[DELETE])];
        let facts = fixture.facts(&[], &denies);
        let mut req = request(fixture.actor, action(DELETE), DOC.parse().unwrap());
        req.is_root = true;

        assert_eq!(evaluate(&req, &facts).unwrap().decision, Decision::Allowed);
    }

    #[test]
    fn root_on_a_missing_target_is_not_found() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.is_root = true;
        req.existence = Existence::Missing;

        assert_eq!(
            evaluate(&req, &fixture.facts(&[], &[])).unwrap().decision,
            Decision::NotFound
        );
    }

    #[test]
    fn root_with_unavailable_existence_errors_instead_of_allowing() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.is_root = true;
        req.existence = Existence::Unavailable(FactFailure::Provider);

        assert_eq!(
            evaluate(&req, &fixture.facts(&[], &[])),
            Err(EvalError::FactsUnavailable {
                cause: FactFailure::Provider
            })
        );
    }

    #[test]
    fn a_path_leaf_mismatch_fails_closed() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.path = Some(
            "acta::workspace::w1/folder::f1/document::other"
                .parse()
                .unwrap(),
        );

        assert!(matches!(
            evaluate(&req, &fixture.facts(&[], &[])),
            Err(EvalError::InconsistentFacts { .. })
        ));
    }

    #[test]
    fn a_path_product_mismatch_fails_closed() {
        let fixture = Fixture::new();
        let mut req = request(fixture.actor, action(READ), DOC.parse().unwrap());
        req.path = Some("custos::document::d1".parse().unwrap());

        assert!(matches!(
            evaluate(&req, &fixture.facts(&[], &[])),
            Err(EvalError::InconsistentFacts { .. })
        ));
    }

    #[test]
    fn grants_to_other_subjects_do_not_apply() {
        let mut fixture = Fixture::new();
        let other_group = GroupId::new();
        fixture
            .membership
            .set_group(other_group, Membership::NotMember);
        let grants = [
            grant(fixture.bob.clone(), ref_target(DOC), &[READ]),
            grant(Subject::Group(other_group), ref_target(DOC), &[READ]),
        ];
        let facts = fixture.facts(&grants, &[]);

        assert_eq!(
            evaluate(&read_request(fixture.actor), &facts)
                .unwrap()
                .decision,
            Decision::NotFound
        );
    }

    #[test]
    fn a_grant_action_of_another_kind_is_ignored_even_beside_disclosing_actions() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            ref_target("acta::workspace::w1"),
            &["acta::workspace::update", UPDATE],
        )];
        let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];
        let facts = fixture.facts(&grants, &denies);

        assert_eq!(
            evaluate(&read_request(fixture.actor), &facts)
                .unwrap()
                .decision,
            Decision::NotFound
        );
    }

    #[test]
    fn an_action_specific_deny_blocks_only_its_actions() {
        let fixture = Fixture::new();
        let grants = [grant(
            fixture.alice.clone(),
            ref_target(DOC),
            &[READ, UPDATE, DELETE],
        )];
        let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[DELETE])];
        let facts = fixture.facts(&grants, &denies);

        let read = evaluate(
            &request(fixture.actor, action(READ), DOC.parse().unwrap()),
            &facts,
        )
        .unwrap();
        let update = evaluate(
            &request(fixture.actor, action(UPDATE), DOC.parse().unwrap()),
            &facts,
        )
        .unwrap();
        let delete = evaluate(
            &request(fixture.actor, action(DELETE), DOC.parse().unwrap()),
            &facts,
        )
        .unwrap();

        assert_eq!(read.decision, Decision::Allowed);
        assert_eq!(update.decision, Decision::Allowed);
        assert_eq!(
            delete.decision,
            Decision::Denied {
                because: DenyCause::DenyRuleEnforced
            }
        );
    }

    #[test]
    fn audit_mode_does_not_upgrade_a_denied_request() {
        let fixture = Fixture::new();
        let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[UPDATE])];
        let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[READ])];
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
        assert!(outcome.would_block.is_empty());
    }

    #[test]
    fn disabled_mode_ignores_deny_facts_and_yields_no_evidence() {
        let fixture = Fixture::new();
        let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
        let denies = [deny(fixture.alice.clone(), ref_target(DOC), &[READ])];
        let facts = fixture.facts(&grants, &denies);
        let mut req = read_request(fixture.actor);
        req.deny_mode = DenyMode::Disabled;

        let outcome = evaluate(&req, &facts).unwrap();

        assert_eq!(outcome.decision, Decision::Allowed);
        assert!(outcome.would_block.is_empty());
    }

    #[test]
    fn denies_from_other_subjects_do_not_apply() {
        let fixture = Fixture::new();
        let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
        let denies = [deny(fixture.bob.clone(), ref_target(DOC), &[READ])];
        let facts = fixture.facts(&grants, &denies);

        assert_eq!(
            evaluate(&read_request(fixture.actor), &facts)
                .unwrap()
                .decision,
            Decision::Allowed
        );
    }

    #[test]
    fn group_denies_apply_through_confirmed_membership() {
        let mut fixture = Fixture::new();
        let banned = GroupId::new();
        fixture.membership.set_group(banned, Membership::Member);
        let grants = [grant(fixture.alice.clone(), ref_target(DOC), &[READ])];
        let denies = [deny(Subject::Group(banned), ref_target(DOC), &[READ])];
        let facts = fixture.facts(&grants, &denies);

        assert_eq!(
            evaluate(&read_request(fixture.actor), &facts)
                .unwrap()
                .decision,
            Decision::NotFound
        );
    }
}
