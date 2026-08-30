#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod attribution;
pub mod capabilities;
pub mod config;
pub mod error;
pub mod ids;
pub mod position;
pub mod principal;
pub mod registry;
pub mod slug;

pub use attribution::Attribution;
pub use ids::{
    ActionId, ActionIdParseError, PathSegment, PrincipalId, PrincipalIdParseError, PrincipalSetId,
    PrincipalSetIdParseError, ResourcePath, ResourcePathParseError, ResourceRef,
    ResourceRefParseError, ResourceSelector, ResourceSelectorParseError, SegmentError,
    SelectorSegment, Specificity,
};
pub use principal::Principal;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_root_paths_resolve() {
        let action: ActionId = "acta::task::create".parse().expect("valid action id");
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");
        let principal: PrincipalId = "u_42".parse().expect("valid principal id");
        let principal_set: PrincipalSetId = "acta::workspace::w_01::members"
            .parse()
            .expect("valid principal set id");

        assert_eq!(action.to_string(), "acta::task::create");
        assert_eq!(resource.to_string(), "acta::document::42");
        assert_eq!(principal.to_string(), "u_42");
        assert_eq!(principal_set.to_string(), "acta::workspace::w_01::members");
    }

    #[test]
    fn registry_module_path_resolves() {
        let id: crate::registry::ComponentId =
            "storage.filesystem".parse().expect("valid component id");

        assert_eq!(id.to_string(), "storage.filesystem");
    }
}
