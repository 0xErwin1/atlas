//! The pure delegation check (GRANT-3): an actor may only grant what it
//! holds, and only where it holds `custos::grant::create`.

use std::collections::BTreeSet;
use std::fmt;

use crate::eval::EvalError;
use crate::eval::evaluator::{
    Decision, DenyMode, EvaluationFacts, Lane, RequestView, chain_from_path, decide, lane_actions,
    validate_membership_binding, validate_path_consistency,
};
use crate::eval::model::{ActionSet, CUSTOS_PRODUCT, Ceiling, Existence};
use crate::ids::PrincipalId;
use atlas_core::ids::{ActionId, ResourcePath, ResourceRef};

/// One actor on one target, without a requested action: the input of
/// [`effective_actions`]. The fields mean what they mean on
/// [`EvalRequest`](crate::eval::EvalRequest).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveRequest {
    pub actor: PrincipalId,
    pub is_root: bool,
    pub target: ResourceRef,
    pub path: Option<ResourcePath>,
    pub existence: Existence,
    pub ceiling: Ceiling,
    pub deny_mode: DenyMode,
}

/// The actions an actor may exercise on one target: exactly the actions
/// [`evaluate`](crate::eval::evaluate) would allow there. Only
/// [`effective_actions`] builds it, which makes it the only sanctioned input
/// of [`can_delegate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveActions {
    all: bool,
    actions: BTreeSet<ActionId>,
}

impl EffectiveActions {
    fn held(actions: BTreeSet<ActionId>) -> Self {
        Self {
            all: false,
            actions,
        }
    }

    /// Whether the actor may exercise `action` on the target.
    pub fn contains(&self, action: &ActionId) -> bool {
        self.all || self.actions.contains(action)
    }

    /// Whether the actor is root and so holds every action.
    pub fn is_all(&self) -> bool {
        self.all
    }

    /// The explicitly held actions in ascending order. Empty for root,
    /// which holds every action without listing them.
    pub fn iter(&self) -> impl Iterator<Item = &ActionId> {
        self.actions.iter()
    }
}

/// Computes the actor's effective actions on one target once: every action
/// of the winning grants in the product and delegation lanes, after the
/// credential ceiling and enforced denies, kept only where
/// [`evaluate`](crate::eval::evaluate) would return `Allowed` for it. Root
/// holds every action on an existing target; a missing target holds none.
///
/// This is the only sanctioned input of [`can_delegate`]: routes must not
/// rebuild it from raw grants or by evaluating actions one at a time.
/// Facts `evaluate` would reject for any held candidate action fail the
/// whole computation with the same technical error.
pub fn effective_actions(
    request: &EffectiveRequest,
    facts: &EvaluationFacts<'_>,
) -> Result<EffectiveActions, EvalError> {
    validate_membership_binding(request.actor, facts.membership)?;

    if let Some(path) = &request.path {
        validate_path_consistency(path, &request.target)?;
    }

    match request.existence {
        Existence::Missing => return Ok(EffectiveActions::held(BTreeSet::new())),
        Existence::Unavailable(cause) => return Err(EvalError::FactsUnavailable { cause }),
        Existence::Exists => {}
    }

    if request.is_root {
        return Ok(EffectiveActions {
            all: true,
            actions: BTreeSet::new(),
        });
    }

    let Some(path) = &request.path else {
        return Err(EvalError::MissingAncestry);
    };

    let chain = chain_from_path(path)?;
    let product = request.target.product();
    let product_lane = lane_actions(
        request.actor,
        &request.ceiling,
        product,
        Lane::Product,
        facts,
        &chain,
    )?;
    let delegation_lane = lane_actions(
        request.actor,
        &request.ceiling,
        product,
        Lane::Delegation,
        facts,
        &chain,
    )?;
    let lanes = |lane| {
        Ok(match lane {
            Lane::Product => product_lane.clone(),
            Lane::Delegation => delegation_lane.clone(),
        })
    };

    let mut held = BTreeSet::new();

    for action in product_lane.iter().chain(&delegation_lane) {
        let view = RequestView {
            actor: request.actor,
            is_root: false,
            action,
            target: &request.target,
            path: Some(path),
            existence: Existence::Exists,
            ceiling: &request.ceiling,
            deny_mode: request.deny_mode,
        };

        if decide(&view, facts, &chain, &lanes)?.decision == Decision::Allowed {
            held.insert(action.clone());
        }
    }

    Ok(EffectiveActions::held(held))
}

/// Why an actor may not create a grant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DelegationRefused {
    /// The actor's effective actions on the target lack
    /// `custos::grant::create`.
    MissingGrantCreate,
    /// The grant confers actions the actor does not hold on the target,
    /// listed in ascending order.
    BeyondAuthority { actions: Vec<ActionId> },
}

impl fmt::Display for DelegationRefused {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingGrantCreate => {
                write!(f, "the actor may not create grants on this target")
            }
            Self::BeyondAuthority { actions } => {
                let listed: Vec<String> = actions.iter().map(ToString::to_string).collect();
                write!(
                    f,
                    "the grant confers actions beyond the actor's authority: {}",
                    listed.join(", ")
                )
            }
        }
    }
}

impl std::error::Error for DelegationRefused {}

/// Decides whether an actor may grant `granted` on a target where its
/// effective actions, from [`effective_actions`], are `actor_effective`.
/// Precedence, the credential ceiling and enforced denies are already
/// applied there (GRANT-4), so this check only compares sets: the actor
/// needs `custos::grant::create` and every granted action. Root may grant
/// anything.
pub fn can_delegate(
    actor_effective: &EffectiveActions,
    granted: &ActionSet,
) -> Result<(), DelegationRefused> {
    if !actor_effective.is_all() && !actor_effective.iter().any(is_grant_create) {
        return Err(DelegationRefused::MissingGrantCreate);
    }

    let beyond: BTreeSet<&ActionId> = granted
        .iter()
        .filter(|action| !actor_effective.contains(action))
        .collect();

    if beyond.is_empty() {
        Ok(())
    } else {
        Err(DelegationRefused::BeyondAuthority {
            actions: beyond.into_iter().cloned().collect(),
        })
    }
}

fn is_grant_create(action: &ActionId) -> bool {
    action.product() == CUSTOS_PRODUCT && action.kind() == "grant" && action.action() == "create"
}
