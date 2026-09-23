//! The list visibility predicate (EVAL-5, PROV-3).
//!
//! A list query cannot evaluate every candidate row before `LIMIT`, so the
//! actor's authority for one action on one resource kind is compiled into a
//! predicate over grant targets that a storage adapter can translate. The
//! predicate is expressed in grant-target terms only, never SQL, and agrees
//! exactly with [`evaluate`](crate::eval::evaluate): it permits a resource
//! path precisely when evaluating the same action on the path's leaf, with
//! confirmed existence, returns `Allowed`.

use crate::eval::EvalError;
use crate::eval::evaluator::{
    DenyMode, EvaluationFacts, Lane, Reach, chain_from_path, covers_chain, level_of, reach,
    validate_membership_binding, winners,
};
use crate::eval::model::{Ceiling, GrantTarget, is_delegation_action};
use crate::ids::{GroupId, PrincipalId};
use atlas_core::ids::{ActionId, ResourcePath};

/// Which resources of one kind the actor may act on with one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibilityPredicate {
    /// Every resource: the root principal.
    All,
    /// No resource: the ceiling lacks the action or no grant allows it.
    Nothing,
    /// A resource is visible when no `denies` target covers it or one of its
    /// ancestors, and the `grants` rules covering its chain that sit at the
    /// nearest level and strongest tier include at least one
    /// [`RuleEffect::Allow`].
    Rules {
        /// One rule per distinct grant target of the actor, sorted by
        /// target specificity, strongest first. The order is not
        /// precedence: which rule wins depends on the chain level a target
        /// covers, which exists only per resource, so level cannot be part
        /// of a resource-independent sort. A translation must decide as
        /// [`VisibilityPredicate::permits`] does: among the rules covering
        /// the chain keep the nearest level, then the strongest tier within
        /// it, and allow when any of those rules is an allow.
        grants: Vec<VisibilityRule>,
        /// Enforced deny targets; each blocks the resources it covers and
        /// their descendants regardless of grant precedence.
        denies: Vec<GrantTarget>,
    },
}

/// One grant target of the actor and whether it confers the action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityRule {
    pub target: GrantTarget,
    pub effect: RuleEffect,
}

/// The effect of a grant rule when it wins the precedence of a chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleEffect {
    /// A grant on this target includes the action.
    Allow,
    /// Grants on this target exist but none includes the action, so it
    /// shadows weaker or farther grants without allowing.
    Block,
}

impl VisibilityPredicate {
    /// Reference matcher for a resource path of the predicate's product and
    /// kind: the semantics a storage translation must reproduce.
    pub fn permits(&self, path: &ResourcePath) -> bool {
        let (grants, denies) = match self {
            Self::All => return true,
            Self::Nothing => return false,
            Self::Rules { grants, denies } => (grants, denies),
        };

        let Ok(chain) = chain_from_path(path) else {
            return false;
        };

        if denies.iter().any(|target| covers_chain(target, &chain)) {
            return false;
        }

        let candidates = grants
            .iter()
            .filter_map(|rule| {
                level_of(&rule.target, &chain)
                    .map(|level| (level, rule.target.specificity(), rule.effect))
            })
            .collect();

        winners(candidates).contains(&RuleEffect::Allow)
    }
}

/// Compiles the actor's authority for `action` on resources of `kind` into
/// a [`VisibilityPredicate`].
///
/// Root sees everything; a ceiling without the action or grants that never
/// include it yield [`VisibilityPredicate::Nothing`]. Grants compete in the
/// action's precedence lane only: grants able to carry the action's product
/// for a product action, grants carrying delegation actions for a
/// delegation action. Otherwise unknown facts that could decide some
/// resource fail closed: any group grant of the lane with unknown
/// membership (it could win or shadow a tier), and in enforced or audit
/// mode any group deny with unknown membership on the action. Audit mode
/// additionally rejects, when a confirmed deny on the action exists,
/// unknown group denies on other actions of `kind` and, for a delegation
/// action, unknown product-lane group grants, because the single-target
/// evaluator needs them to tell a denied resource from a hidden one. For a
/// product action, deny facts of other products are ignored; a delegation
/// action applies to targets of every product.
pub fn visibility_filter(
    actor: PrincipalId,
    is_root: bool,
    kind: &str,
    action: &ActionId,
    ceiling: &Ceiling,
    deny_mode: DenyMode,
    facts: &EvaluationFacts<'_>,
) -> Result<VisibilityPredicate, EvalError> {
    validate_membership_binding(actor, facts.membership)?;

    if is_root {
        return Ok(VisibilityPredicate::All);
    }

    if !ceiling.permits(action) {
        return Ok(VisibilityPredicate::Nothing);
    }

    let grants = grant_rules(actor, action, facts)?;

    if !grants.iter().any(|rule| rule.effect == RuleEffect::Allow) {
        return Ok(VisibilityPredicate::Nothing);
    }

    let denies = deny_targets(actor, kind, action, deny_mode, facts)?;

    Ok(VisibilityPredicate::Rules { grants, denies })
}

/// The actor's grant rules for `action`, merged per target. Fails closed on
/// a group grant with unknown membership unless no grant, known or not,
/// includes the action.
fn grant_rules(
    actor: PrincipalId,
    action: &ActionId,
    facts: &EvaluationFacts<'_>,
) -> Result<Vec<VisibilityRule>, EvalError> {
    let mut rules: Vec<VisibilityRule> = Vec::new();
    let mut unknown: Option<GroupId> = None;
    let mut unknown_allows = false;

    let lane = Lane::of(action);
    let relevant = facts
        .grants
        .iter()
        .filter(|grant| lane.admits(grant, action.product()));

    for grant in relevant {
        let allows = grant.actions().contains(action);

        match reach(grant.subject(), actor, facts.membership) {
            Reach::Applies => merge_rule(&mut rules, grant.target(), allows),
            Reach::DoesNotApply => {}
            Reach::Unknown(group) => {
                unknown.get_or_insert(group);
                unknown_allows |= allows;
            }
        }
    }

    let any_allows = unknown_allows || rules.iter().any(|rule| rule.effect == RuleEffect::Allow);

    if let Some(group) = unknown
        && any_allows
    {
        return Err(EvalError::GroupMembershipUnavailable { group });
    }

    rules.sort_by_key(|rule| std::cmp::Reverse(rule.target.specificity()));

    Ok(rules)
}

/// Whether facts on `target` can decide `action`. A product action only
/// applies to targets of its product; a delegation action applies to
/// targets of any product, so no fact is excluded by product.
fn in_scope(target: &GrantTarget, action: &ActionId) -> bool {
    is_delegation_action(action) || target.product() == action.product()
}

fn merge_rule(rules: &mut Vec<VisibilityRule>, target: &GrantTarget, allows: bool) {
    let effect = if allows {
        RuleEffect::Allow
    } else {
        RuleEffect::Block
    };

    match rules.iter_mut().find(|rule| rule.target == *target) {
        Some(existing) if allows => existing.effect = RuleEffect::Allow,
        Some(_) => {}
        None => rules.push(VisibilityRule {
            target: target.clone(),
            effect,
        }),
    }
}

/// The enforced deny targets for `action`, after checking that no unknown
/// group deny could change a decision. Disabled mode ignores denies and
/// audit mode never blocks.
fn deny_targets(
    actor: PrincipalId,
    kind: &str,
    action: &ActionId,
    deny_mode: DenyMode,
    facts: &EvaluationFacts<'_>,
) -> Result<Vec<GrantTarget>, EvalError> {
    if deny_mode == DenyMode::Disabled {
        return Ok(Vec::new());
    }

    let mut targets: Vec<GrantTarget> = Vec::new();
    let mut unknown_on_action: Option<GroupId> = None;
    let mut unknown_on_kind: Option<GroupId> = None;

    let relevant = facts
        .denies
        .iter()
        .filter(|rule| in_scope(rule.target(), action));

    for rule in relevant {
        let on_action = rule.actions().contains(action);
        let on_kind = rule.actions().iter().any(|denied| denied.kind() == kind);

        match reach(rule.subject(), actor, facts.membership) {
            Reach::Applies if on_action && !targets.contains(rule.target()) => {
                targets.push(rule.target().clone());
            }
            Reach::Applies | Reach::DoesNotApply => {}
            Reach::Unknown(group) if on_action => {
                unknown_on_action.get_or_insert(group);
            }
            Reach::Unknown(group) if on_kind => {
                unknown_on_kind.get_or_insert(group);
            }
            Reach::Unknown(_) => {}
        }
    }

    if let Some(group) = unknown_on_action {
        return Err(EvalError::GroupMembershipUnavailable { group });
    }

    if deny_mode == DenyMode::Enforced {
        return Ok(targets);
    }

    if targets.is_empty() {
        return Ok(Vec::new());
    }

    let unknown_discovery = if is_delegation_action(action) {
        unknown_discovery_grant(actor, facts)
    } else {
        None
    };

    match unknown_on_kind.or(unknown_discovery) {
        Some(group) => Err(EvalError::GroupMembershipUnavailable { group }),
        None => Ok(Vec::new()),
    }
}

/// The first product-lane grant with unknown group membership. An audited
/// delegation request that a confirmed deny would block falls back to
/// product discovery, which needs these memberships.
fn unknown_discovery_grant(actor: PrincipalId, facts: &EvaluationFacts<'_>) -> Option<GroupId> {
    facts
        .grants
        .iter()
        .filter(|grant| !grant.actions().is_delegation_only())
        .find_map(
            |grant| match reach(grant.subject(), actor, facts.membership) {
                Reach::Unknown(group) => Some(group),
                Reach::Applies | Reach::DoesNotApply => None,
            },
        )
}
