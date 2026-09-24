//! A storage-neutral form of the V2 list visibility predicate (EVAL-5,
//! PROV-3), built only from core ids so a product's storage adapter can
//! translate it without depending on the evaluator that produced it.
//!
//! The semantics are the evaluator's: a resource is visible when no deny
//! target covers it or an ancestor, and among the grants covering its chain
//! those at the nearest level and, within it, the strongest target
//! specificity include at least one that allows. Grant order carries no
//! precedence.

use crate::ids::{ResourcePath, ResourceRef, ResourceSelector, Specificity};

/// Which resources of one kind an actor may act on with one action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListVisibility {
    /// Every resource.
    All,
    /// No resource.
    Nothing,
    /// A resource is visible when no `denies` target covers it or one of its
    /// ancestors, and the `grants` rules covering its chain that sit at the
    /// nearest level and strongest tier include at least one allow.
    Rules {
        /// One rule per distinct grant target of the actor. The order is not
        /// precedence: which rule wins depends on the chain level a target
        /// covers, which exists only per resource. A translation must decide
        /// as the evaluator does: among the rules covering the chain keep the
        /// nearest level, then the strongest tier within it, and allow when
        /// any of those rules is an allow.
        grants: Vec<VisibilityGrant>,
        /// Enforced deny targets; each blocks the resources it covers and
        /// their descendants regardless of grant precedence.
        denies: Vec<VisibilityTarget>,
    },
}

/// One grant target of the actor and whether it confers the action when it
/// wins a resource's precedence. A losing-effect grant still shadows
/// farther or weaker ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VisibilityGrant {
    pub target: VisibilityTarget,
    pub allow: bool,
}

/// The resource side of a grant or deny: an exact reference that follows
/// the object, an exact current path, or a selector over current paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibilityTarget {
    Ref(ResourceRef),
    Path(ResourcePath),
    Selector(ResourceSelector),
}

impl VisibilityTarget {
    pub fn product(&self) -> &str {
        match self {
            Self::Ref(reference) => reference.product(),
            Self::Path(path) => path.product(),
            Self::Selector(selector) => selector.product(),
        }
    }

    /// The precedence tier of the target within one chain level.
    pub fn specificity(&self) -> Specificity {
        match self {
            Self::Ref(reference) => reference.specificity(),
            Self::Path(path) => path.specificity(),
            Self::Selector(selector) => selector.specificity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_target_reports_the_product_and_tier_of_its_core_id() {
        let reference: ResourceRef = "acta::document::d1".parse().unwrap();
        let path: ResourcePath = "acta::workspace::w1/document::d1".parse().unwrap();
        let selector: ResourceSelector = "custos::*".parse().unwrap();

        let targets = [
            VisibilityTarget::Ref(reference.clone()),
            VisibilityTarget::Path(path.clone()),
            VisibilityTarget::Selector(selector.clone()),
        ];

        assert_eq!(
            targets
                .iter()
                .map(VisibilityTarget::product)
                .collect::<Vec<_>>(),
            ["acta", "acta", "custos"]
        );
        assert_eq!(targets[0].specificity(), reference.specificity());
        assert_eq!(targets[1].specificity(), path.specificity());
        assert_eq!(targets[2].specificity(), selector.specificity());
    }
}
