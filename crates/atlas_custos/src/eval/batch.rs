//! Batch evaluation (EVAL-4): one actor and one action over many targets.

use crate::eval::EvalError;
use crate::eval::evaluator::{
    Decision, DenyMode, Evaluated, EvaluationFacts, RequestView, evaluate_validated,
    validate_membership_binding, validate_request,
};
use crate::eval::model::{Ceiling, Existence};
use crate::ids::PrincipalId;
use atlas_core::ids::{ActionId, ResourcePath, ResourceRef};

/// One batch authorization request: the facts shared by every target plus
/// the per-target facts, in the order results are returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchRequest {
    pub actor: PrincipalId,
    pub is_root: bool,
    pub action: ActionId,
    pub ceiling: Ceiling,
    pub deny_mode: DenyMode,
    pub targets: Vec<BatchTarget>,
}

/// The facts that differ per target of a batch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchTarget {
    pub target: ResourceRef,
    pub path: Option<ResourcePath>,
    pub existence: Existence,
}

/// Evaluates one action for one actor over many targets, returning one
/// result per target in input order.
///
/// The shared facts are validated once: membership facts bound to another
/// principal fail the whole batch. Each target then follows the single
/// [`evaluate`](crate::eval::evaluate) contract on its own, so a per-target
/// technical error is reported for that target only, except that a target
/// whose existence is unavailable is `NotFound` without evidence instead of
/// failing.
pub fn evaluate_batch(
    request: &BatchRequest,
    facts: &EvaluationFacts<'_>,
) -> Result<Vec<Result<Evaluated, EvalError>>, EvalError> {
    validate_membership_binding(request.actor, facts.membership)?;

    let results = request
        .targets
        .iter()
        .map(|entry| {
            let view = RequestView {
                actor: request.actor,
                is_root: request.is_root,
                action: &request.action,
                target: &entry.target,
                path: entry.path.as_ref(),
                existence: entry.existence,
                ceiling: &request.ceiling,
                deny_mode: request.deny_mode,
            };

            evaluate_target(&view, facts)
        })
        .collect();

    Ok(results)
}

fn evaluate_target(
    request: &RequestView<'_>,
    facts: &EvaluationFacts<'_>,
) -> Result<Evaluated, EvalError> {
    validate_request(request)?;

    if let Existence::Unavailable(_) = request.existence {
        return Ok(Evaluated::new(Decision::NotFound));
    }

    evaluate_validated(request, facts)
}
