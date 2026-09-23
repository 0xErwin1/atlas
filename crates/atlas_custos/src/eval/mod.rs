//! Pure V2 authorization evaluation (E5).
//!
//! The evaluator decides one action of one principal on one resource from
//! caller-supplied facts only: no I/O, no provider calls, no state across
//! requests. It is additive to the V1 role-resolution in
//! `atlas_server::authz::policy::resolve()`, which stays unchanged until the
//! coordinated E7 integration.
//!
//! Built on the same decision procedure: [`evaluate_batch`] decides one
//! action over many targets, [`visibility_filter`] compiles a list predicate
//! that agrees with [`evaluate`], and [`Catalog`] validates grant targets,
//! custom roles and grant specs against the declared product vocabulary.
//!
//! Facts boundary (the contract the caller must honor):
//!
//! - The actor is always one Custos principal. `EvalRequest::is_root` is
//!   caller-asserted and must be derived from authentication (the root
//!   user), never from grants or request input. The evaluator never infers
//!   it, and platform administrators receive no bypass: they evaluate like
//!   any other principal. Membership facts are bound to the actor; facts
//!   resolved for another principal are rejected.
//! - `Existence` is physical only. The evaluator derives discoverability
//!   itself: an existing target on which the actor holds no effective action
//!   after the credential ceiling and enforced denies is `Decision::NotFound`,
//!   indistinguishable from a missing one. Hidden outcomes carry no grant,
//!   deny or reason metadata.
//! - Grant and deny actions are trusted to be valid catalog actions: the
//!   evaluator checks products but does not consult a catalog. Only the
//!   requested action and actions of the target's own product and kind
//!   make a target discoverable; descendant actions attached to an ancestor
//!   grant count for the descendant target.
//! - Group and principal-set grants apply only through confirmed membership
//!   facts. A group grant or a relevant group deny whose membership is
//!   missing or indeterminate fails closed with a technical error when it
//!   could decide the outcome (it sits at the winning level and tier, or it
//!   denies an action the decision depends on); shadowed or unrelated facts
//!   need no membership. Missing or indeterminate principal-set membership
//!   stays nonmember.
//! - A non-root evaluation of an existing resource requires the resource's
//!   current ancestry, so ancestor-scoped denies cannot be bypassed by
//!   omitting the path; a resource with no ancestors carries a one-segment
//!   path.
//! - Facts that are unavailable or mutually inconsistent produce a technical
//!   error and never an allow; the outer orchestration maps that to 503.
//!
//! Deny mode is evaluation configuration, not persisted state: `Disabled`
//! ignores deny facts, `Audit` decides without denies and returns evidence of
//! the denies that would change the decision, and `Enforced` removes every
//! action a deny matches on the target or any ancestor from the actor's
//! effective authority, beating any allow (the root principal is exempt).

mod batch;
mod catalog;
mod evaluator;
mod model;
mod visibility;

pub use batch::{BatchRequest, BatchTarget, evaluate_batch};
pub use catalog::{
    BuiltinRole, Catalog, CatalogError, CustomRole, GrantSpec, ProductSpec, RoleRef, RoleSpec,
};
pub use evaluator::{
    Decision, DenyCause, DenyEvidence, DenyMode, EvalRequest, Evaluated, EvaluationFacts, evaluate,
};
pub use model::{
    ActionSet, Ceiling, CeilingActions, DenyRule, Existence, Grant, GrantTarget, Membership,
    MembershipFacts, Subject,
};
pub use visibility::{RuleEffect, VisibilityPredicate, VisibilityRule, visibility_filter};

use crate::ids::GroupId;
use std::fmt;

/// Why a fact could not be established. Deliberately carries no provider
/// payload, credential or raw error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FactFailure {
    /// The fact source did not answer in time.
    Timeout,
    /// The fact source reported a failure.
    Provider,
    /// The caller failed while resolving the fact.
    Internal,
    /// The fact source answered without a definite result.
    Indeterminate,
}

impl fmt::Display for FactFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let cause = match self {
            Self::Timeout => "timeout",
            Self::Provider => "provider failure",
            Self::Internal => "internal failure",
            Self::Indeterminate => "indeterminate result",
        };

        f.write_str(cause)
    }
}

/// Technical failure of the pure evaluation. Never carries an allow: every
/// variant is fail-closed and maps to a 5xx-class outcome in the outer
/// orchestration, not to a `NotFound` decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvalError {
    /// The requested action belongs to a different product than the target.
    CrossProductRequest {
        target_product: String,
        action_product: String,
    },
    /// A grant or deny rule mixes action products, or pairs actions of one
    /// product with a target of another.
    CrossProductFact {
        target_product: String,
        action_product: String,
    },
    /// An action set mixes actions from different products.
    CrossProductActions { first: String, second: String },
    /// Supplied facts contradict each other (path/target/product/ancestry).
    InconsistentFacts { detail: String },
    /// An existing non-root resource was evaluated without its current
    /// ancestry, so ancestor-scoped deny rules cannot be checked.
    MissingAncestry,
    /// A group grant or deny rule could decide the outcome, but the acting
    /// principal's membership in that group has no confirmed fact.
    GroupMembershipUnavailable { group: GroupId },
    /// A fact could not be established (for example the existence lookup
    /// failed). The caller must not flatten this into `Missing`.
    FactsUnavailable { cause: FactFailure },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CrossProductRequest {
                target_product,
                action_product,
            } => write!(
                f,
                "requested action product `{action_product}` does not match target product `{target_product}`"
            ),
            Self::CrossProductFact {
                target_product,
                action_product,
            } => write!(
                f,
                "fact mixes action product `{action_product}` with target product `{target_product}`"
            ),
            Self::CrossProductActions { first, second } => {
                write!(f, "action set mixes products `{first}` and `{second}`")
            }
            Self::InconsistentFacts { detail } => {
                write!(f, "inconsistent authorization facts: {detail}")
            }
            Self::MissingAncestry => write!(
                f,
                "the current ancestry of an existing non-root resource was not supplied"
            ),
            Self::GroupMembershipUnavailable { group } => write!(
                f,
                "membership in group `{group}` is unavailable for a deciding grant or deny rule"
            ),
            Self::FactsUnavailable { cause } => {
                write!(f, "authorization facts are unavailable: {cause}")
            }
        }
    }
}

impl std::error::Error for EvalError {}
