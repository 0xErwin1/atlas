//! The pure V2 evaluation fact vocabulary: subjects, action sets, grant and
//! deny targets, membership facts, credential ceilings and existence facts.
//!
//! Every type is plain data validated at construction so that malformed or
//! cross-product facts cannot be represented and reach the evaluator.

use std::collections::{HashMap, HashSet};

use crate::eval::{EvalError, FactFailure};
use crate::ids::{GroupId, PrincipalId};
use atlas_core::ids::{
    ActionId, PrincipalSetId, ResourcePath, ResourceRef, ResourceSelector, Specificity,
};

/// The subject a grant or deny rule is addressed to. No re-keying:
/// principals keep their Custos identity. The acting side of a request is
/// always a single principal, never a group or principal set.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Subject {
    Principal(PrincipalId),
    Group(GroupId),
    PrincipalSet(PrincipalSetId),
}

/// Whether the acting principal's membership in a group or principal set is
/// established by the caller-supplied facts. Unknown membership never
/// grants: an absent or indeterminate group fact that could decide the
/// outcome fails the evaluation, and an absent or indeterminate principal
/// set fact reads as nonmembership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Membership {
    Member,
    NotMember,
    Indeterminate,
}

/// Evaluated membership facts bound to one acting principal, grouped by the
/// subject identity they resolve. The evaluator rejects facts bound to a
/// different principal than the request actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipFacts {
    actor: PrincipalId,
    groups: HashMap<GroupId, Membership>,
    principal_sets: HashMap<PrincipalSetId, Membership>,
}

impl MembershipFacts {
    /// Empty membership facts for `actor`.
    pub fn new(actor: PrincipalId) -> Self {
        Self {
            actor,
            groups: HashMap::new(),
            principal_sets: HashMap::new(),
        }
    }

    /// The principal these facts were resolved for.
    pub fn actor(&self) -> PrincipalId {
        self.actor
    }

    pub fn set_group(&mut self, group: GroupId, membership: Membership) {
        self.groups.insert(group, membership);
    }

    pub fn set_principal_set(&mut self, set: PrincipalSetId, membership: Membership) {
        self.principal_sets.insert(set, membership);
    }

    /// The recorded membership for `group`, keeping an absent fact distinct
    /// from a confirmed one. The evaluator fails closed when a group grant
    /// or deny that could decide the outcome has no recorded membership or
    /// an indeterminate one.
    pub fn group_membership(&self, group: &GroupId) -> Option<Membership> {
        self.groups.get(group).copied()
    }

    /// The recorded membership for `set`; absent entries read as
    /// [`Membership::NotMember`].
    pub fn principal_set(&self, set: &PrincipalSetId) -> Membership {
        self.principal_sets
            .get(set)
            .copied()
            .unwrap_or(Membership::NotMember)
    }
}

/// The product whose actions administer authorization itself.
pub(crate) const CUSTOS_PRODUCT: &str = "custos";

/// The Custos delegation actions as `(kind, action)` pairs under the
/// `custos` product: `custos::grant::create`, `custos::grant::delete` and
/// `custos::group::create|update|delete|add_member|remove_member`.
const DELEGATION_ACTIONS: [(&str, &str); 7] = [
    ("grant", "create"),
    ("grant", "delete"),
    ("group", "create"),
    ("group", "update"),
    ("group", "delete"),
    ("group", "add_member"),
    ("group", "remove_member"),
];

/// Whether `action` is one of the fixed Custos delegation actions. They are
/// the only Custos actions an action set may carry next to another
/// product's actions, and they are evaluated against the grant's own target
/// like any other action.
pub fn is_delegation_action(action: &ActionId) -> bool {
    action.product() == CUSTOS_PRODUCT
        && DELEGATION_ACTIONS
            .iter()
            .any(|(kind, verb)| action.kind() == *kind && action.action() == *verb)
}

/// The resolved action set of a grant or deny rule: the actions of one
/// product plus any Custos delegation actions. A set of Custos actions only
/// is valid too. An empty set grants and denies nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ActionSet {
    product: Option<String>,
    actions: HashSet<ActionId>,
    delegation_only: bool,
}

impl ActionSet {
    /// Builds an action set from resolved actions, rejecting a set whose
    /// non-delegation actions span more than one product.
    pub fn new(actions: impl IntoIterator<Item = ActionId>) -> Result<Self, EvalError> {
        let mut anchor: Option<String> = None;
        let mut set = HashSet::new();

        for action in actions {
            if !is_delegation_action(&action) {
                match &anchor {
                    Some(existing) if existing != action.product() => {
                        return Err(EvalError::CrossProductActions {
                            first: existing.clone(),
                            second: action.product().to_string(),
                        });
                    }
                    Some(_) => {}
                    None => anchor = Some(action.product().to_string()),
                }
            }

            set.insert(action);
        }

        let delegation_only = anchor.is_none() && !set.is_empty();
        let product = anchor.or_else(|| delegation_only.then(|| CUSTOS_PRODUCT.to_string()));

        Ok(Self {
            product,
            actions: set,
            delegation_only,
        })
    }

    /// The product of the set's non-delegation actions, `custos` for a set
    /// of delegation actions only, or `None` when the set is empty.
    pub fn product(&self) -> Option<&str> {
        self.product.as_deref()
    }

    /// Whether the set is non-empty and holds delegation actions only.
    pub fn is_delegation_only(&self) -> bool {
        self.delegation_only
    }

    pub fn contains(&self, action: &ActionId) -> bool {
        self.actions.contains(action)
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ActionId> {
        self.actions.iter()
    }
}

/// The resource side of a grant or deny rule, using the core target
/// vocabulary. Ref grants follow a stable object across moves; path and
/// selector targets match only the caller-supplied current path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantTarget {
    Ref(ResourceRef),
    Path(ResourcePath),
    Selector(ResourceSelector),
}

impl GrantTarget {
    pub fn product(&self) -> &str {
        match self {
            Self::Ref(reference) => reference.product(),
            Self::Path(path) => path.product(),
            Self::Selector(selector) => selector.product(),
        }
    }

    /// The core precedence key of the target: exact ref outranks exact path
    /// outranks selector, with the selector tie breakers decided by literal
    /// segment count, then closedness, then fewer wildcards.
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Ref(reference) => reference.specificity(),
            Self::Path(path) => path.specificity(),
            Self::Selector(selector) => selector.specificity(),
        }
    }
}

/// Requires the action set's product to be the target's product. A
/// delegation-only set is valid on any target: delegation is evaluated
/// against the resource being delegated, whatever its product.
fn require_matching_product(target: &GrantTarget, actions: &ActionSet) -> Result<(), EvalError> {
    if actions.is_delegation_only() {
        return Ok(());
    }

    if let Some(action_product) = actions.product()
        && action_product != target.product()
    {
        return Err(EvalError::CrossProductFact {
            target_product: target.product().to_string(),
            action_product: action_product.to_string(),
        });
    }

    Ok(())
}

/// An allow fact: one subject, one target, one resolved action set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    subject: Subject,
    target: GrantTarget,
    actions: ActionSet,
}

impl Grant {
    pub fn new(
        subject: Subject,
        target: GrantTarget,
        actions: ActionSet,
    ) -> Result<Self, EvalError> {
        require_matching_product(&target, &actions)?;

        Ok(Self {
            subject,
            target,
            actions,
        })
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn target(&self) -> &GrantTarget {
        &self.target
    }

    pub fn actions(&self) -> &ActionSet {
        &self.actions
    }
}

/// A deny fact: one subject, one target, one resolved action set. Denies are
/// action-specific; the root principal is exempt from them entirely.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyRule {
    subject: Subject,
    target: GrantTarget,
    actions: ActionSet,
}

impl DenyRule {
    pub fn new(
        subject: Subject,
        target: GrantTarget,
        actions: ActionSet,
    ) -> Result<Self, EvalError> {
        require_matching_product(&target, &actions)?;

        Ok(Self {
            subject,
            target,
            actions,
        })
    }

    pub fn subject(&self) -> &Subject {
        &self.subject
    }

    pub fn target(&self) -> &GrantTarget {
        &self.target
    }

    pub fn actions(&self) -> &ActionSet {
        &self.actions
    }
}

/// The actions a credential may exercise. Unlike [`ActionSet`], a
/// credential ceiling may span several products; an empty set permits
/// nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CeilingActions {
    actions: HashSet<ActionId>,
}

impl CeilingActions {
    pub fn contains(&self, action: &ActionId) -> bool {
        self.actions.contains(action)
    }

    pub fn is_empty(&self) -> bool {
        self.actions.is_empty()
    }
}

impl FromIterator<ActionId> for CeilingActions {
    fn from_iter<I: IntoIterator<Item = ActionId>>(actions: I) -> Self {
        Self {
            actions: actions.into_iter().collect(),
        }
    }
}

/// The credential ceiling of the acting principal. A ceiling can only
/// narrow what grants allow; it never grants by itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ceiling {
    Unrestricted,
    Restricted(CeilingActions),
}

impl Ceiling {
    pub fn permits(&self, action: &ActionId) -> bool {
        match self {
            Self::Unrestricted => true,
            Self::Restricted(actions) => actions.contains(action),
        }
    }
}

/// Whether the target resource physically exists. Discoverability is not an
/// existence fact: the evaluator derives it from the actor's effective
/// authority. `Unavailable` means the existence fact could not be
/// established, with a typed cause.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Existence {
    Exists,
    Missing,
    Unavailable(FactFailure),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn action(raw: &str) -> ActionId {
        raw.parse().unwrap()
    }

    #[test]
    fn an_action_set_records_its_single_product() {
        let set = ActionSet::new([
            action("acta::document::read"),
            action("acta::document::update"),
        ])
        .unwrap();

        assert_eq!(set.product(), Some("acta"));
        assert!(set.contains(&action("acta::document::read")));
        assert!(!set.contains(&action("acta::document::delete")));
    }

    #[test]
    fn a_cross_product_action_set_is_rejected() {
        let mixed = ActionSet::new([
            action("acta::document::read"),
            action("custos::grants::read"),
        ]);

        assert_eq!(
            mixed.unwrap_err(),
            EvalError::CrossProductActions {
                first: "acta".to_string(),
                second: "custos".to_string(),
            }
        );
    }

    #[test]
    fn an_empty_action_set_has_no_product_and_grants_nothing() {
        let set = ActionSet::new([]).unwrap();

        assert_eq!(set.product(), None);
        assert!(set.is_empty());
        assert!(!set.contains(&action("acta::document::read")));
    }

    #[test]
    fn a_grant_rejects_actions_from_another_product_than_its_target() {
        let target = GrantTarget::Ref("acta::document::d1".parse().unwrap());
        let actions = ActionSet::new([action("custos::grants::read")]).unwrap();

        let grant = Grant::new(Subject::Principal(PrincipalId::new()), target, actions);

        assert!(matches!(
            grant.unwrap_err(),
            EvalError::CrossProductFact { .. }
        ));
    }

    #[test]
    fn a_deny_rule_rejects_actions_from_another_product_than_its_target() {
        let target = GrantTarget::Ref("acta::document::d1".parse().unwrap());
        let actions = ActionSet::new([action("custos::grants::read")]).unwrap();

        let rule = DenyRule::new(Subject::Principal(PrincipalId::new()), target, actions);

        assert!(matches!(
            rule.unwrap_err(),
            EvalError::CrossProductFact { .. }
        ));
    }

    #[test]
    fn missing_membership_entries_stay_unknown_for_groups_and_nonmember_for_sets() {
        let actor = PrincipalId::new();
        let facts = MembershipFacts::new(actor);
        let group = GroupId::new();
        let set: PrincipalSetId = "acta::workspace::w1::reviewers".parse().unwrap();

        assert_eq!(facts.actor(), actor);
        assert_eq!(facts.group_membership(&group), None);
        assert_eq!(facts.principal_set(&set), Membership::NotMember);
    }

    #[test]
    fn group_membership_keeps_a_recorded_fact_distinct_from_an_absent_one() {
        let mut facts = MembershipFacts::new(PrincipalId::new());
        let group = GroupId::new();
        facts.set_group(group, Membership::Indeterminate);

        assert_eq!(
            facts.group_membership(&group),
            Some(Membership::Indeterminate)
        );
    }

    #[test]
    fn restricted_ceiling_permits_only_its_actions() {
        let ceiling = Ceiling::Restricted([action("acta::document::read")].into_iter().collect());

        assert!(ceiling.permits(&action("acta::document::read")));
        assert!(!ceiling.permits(&action("acta::document::update")));
        assert!(Ceiling::Unrestricted.permits(&action("acta::document::update")));
    }

    #[test]
    fn a_ceiling_may_span_several_products() {
        let ceiling = Ceiling::Restricted(
            [
                action("acta::document::read"),
                action("custos::grants::read"),
            ]
            .into_iter()
            .collect(),
        );

        assert!(ceiling.permits(&action("acta::document::read")));
        assert!(ceiling.permits(&action("custos::grants::read")));
        assert!(!ceiling.permits(&action("custos::grants::update")));
    }

    #[test]
    fn an_empty_ceiling_permits_nothing() {
        let ceiling = Ceiling::Restricted(CeilingActions::default());

        assert!(!ceiling.permits(&action("acta::document::read")));
    }
}
