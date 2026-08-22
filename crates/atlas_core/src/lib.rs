#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod ids;

pub use ids::{
    ActionId, ActionIdParseError, PathSegment, ResourcePath, ResourcePathParseError, ResourceRef,
    ResourceRefParseError, ResourceSelector, ResourceSelectorParseError, SegmentError,
    SelectorSegment, Specificity,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crate_root_paths_resolve() {
        let action: ActionId = "acta::task::create".parse().expect("valid action id");
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");

        assert_eq!(action.to_string(), "acta::task::create");
        assert_eq!(resource.to_string(), "acta::document::42");
    }
}
