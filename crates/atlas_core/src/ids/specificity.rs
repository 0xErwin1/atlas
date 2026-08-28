use std::cmp::Reverse;

use super::resource_path::ResourcePath;
use super::resource_ref::ResourceRef;
use super::selector::{ResourceSelector, SelectorSegment};

/// Precedence key for grant matching. Field declaration order is the
/// precedence table: exactness first, then literal_segments, then
/// open_ended (a closed selector beats a `**` one), then wildcards (fewer
/// `*` beats more). The derived lexicographic `Ord` compares fields in
/// that order; greater means more specific.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Specificity {
    exactness: u8,
    literal_segments: usize,
    open_ended: Reverse<bool>,
    wildcards: Reverse<usize>,
}

impl Specificity {
    pub(crate) fn new(
        exactness: u8,
        literal_segments: usize,
        wildcards: usize,
        open_ended: bool,
    ) -> Self {
        Self {
            exactness,
            literal_segments,
            open_ended: Reverse(open_ended),
            wildcards: Reverse(wildcards),
        }
    }
}

impl ResourceRef {
    /// Precedence key for an exact resource reference: outranks any
    /// `ResourcePath` or `ResourceSelector` match.
    pub fn specificity(&self) -> Specificity {
        Specificity::new(2, 1, 0, false)
    }
}

impl ResourcePath {
    /// Precedence key for an exact resource path: outranks every
    /// `ResourceSelector` match, ranked among other paths by segment count.
    pub fn specificity(&self) -> Specificity {
        Specificity::new(1, self.segments().count(), 0, false)
    }
}

impl ResourceSelector {
    /// Precedence key for a selector match: ranked by literal segment
    /// count, then by not being open-ended, then by fewer wildcards.
    pub fn specificity(&self) -> Specificity {
        let literal_segments = self
            .segments()
            .iter()
            .filter(|segment| matches!(segment, SelectorSegment::Literal(_)))
            .count();
        let wildcards = self
            .segments()
            .iter()
            .filter(|segment| matches!(segment, SelectorSegment::Any))
            .count();

        Specificity::new(0, literal_segments, wildcards, self.is_open_ended())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{ResourcePath, ResourceRef, ResourceSelector, SelectorSegment};

    #[test]
    fn higher_exactness_wins_regardless_of_other_fields() {
        let low = Specificity::new(0, 100, 0, false);
        let high = Specificity::new(1, 0, 100, true);
        assert!(high > low);
    }

    #[test]
    fn equal_exactness_more_literal_segments_wins() {
        let fewer = Specificity::new(0, 1, 0, false);
        let more = Specificity::new(0, 2, 0, false);
        assert!(more > fewer);
    }

    #[test]
    fn equal_exactness_and_literal_segments_fewer_wildcards_wins() {
        let more_wildcards = Specificity::new(0, 1, 2, false);
        let fewer_wildcards = Specificity::new(0, 1, 1, false);
        assert!(fewer_wildcards > more_wildcards);
    }

    #[test]
    fn equal_exactness_literal_segments_and_wildcards_non_open_ended_wins() {
        let open_ended = Specificity::new(0, 1, 0, true);
        let closed = Specificity::new(0, 1, 0, false);
        assert!(closed > open_ended);
    }

    #[test]
    fn closed_selector_with_more_wildcards_still_outranks_open_ended_one() {
        let open_ended = Specificity::new(0, 1, 0, true);
        let closed_with_wildcards = Specificity::new(0, 1, 3, false);
        assert!(closed_with_wildcards > open_ended);
    }

    #[test]
    fn all_fields_equal_compares_equal() {
        let a = Specificity::new(1, 2, 3, false);
        let b = Specificity::new(1, 2, 3, false);
        assert_eq!(a.cmp(&b), std::cmp::Ordering::Equal);
    }

    #[test]
    fn ord_and_eq_are_consistent() {
        let cases = [
            (
                Specificity::new(0, 0, 0, false),
                Specificity::new(0, 0, 0, false),
            ),
            (
                Specificity::new(1, 0, 0, false),
                Specificity::new(0, 0, 0, false),
            ),
            (
                Specificity::new(0, 1, 0, false),
                Specificity::new(0, 0, 0, false),
            ),
            (
                Specificity::new(0, 0, 1, false),
                Specificity::new(0, 0, 0, false),
            ),
            (
                Specificity::new(0, 0, 0, true),
                Specificity::new(0, 0, 0, false),
            ),
        ];

        for (a, b) in cases {
            assert_eq!(a.cmp(&b) == std::cmp::Ordering::Equal, a == b);
        }
    }

    #[test]
    fn exact_ref_outranks_exact_path() {
        let by_ref: ResourceRef = "acta::document::d1".parse().expect("valid resource ref");
        let by_path: ResourcePath = "acta::document::d1".parse().expect("valid resource path");
        assert!(by_ref.specificity() > by_path.specificity());
    }

    #[test]
    fn exact_path_outranks_any_selector_regardless_of_literal_count() {
        let by_path: ResourcePath = "acta::workspace::w1".parse().expect("valid resource path");
        let by_selector: ResourceSelector = "acta::workspace::w1/project::p1/folder::f1"
            .parse()
            .expect("valid resource selector");
        assert!(by_path.specificity() > by_selector.specificity());
    }

    #[test]
    fn more_literal_segments_outranks_fewer_among_selectors() {
        let fewer: ResourceSelector = "acta::workspace::w1/*"
            .parse()
            .expect("valid resource selector");
        let more: ResourceSelector = "acta::workspace::w1/project::p1"
            .parse()
            .expect("valid resource selector");
        assert!(more.specificity() > fewer.specificity());
    }

    #[test]
    fn fewer_wildcards_outranks_more_among_equal_literal_selectors() {
        let more_wildcards = ResourceSelector::new(
            "acta",
            vec![
                literal("workspace", "w1"),
                SelectorSegment::Any,
                SelectorSegment::Any,
            ],
            false,
        )
        .expect("valid resource selector");
        let fewer_wildcards = ResourceSelector::new(
            "acta",
            vec![literal("workspace", "w1"), SelectorSegment::Any],
            false,
        )
        .expect("valid resource selector");
        assert!(fewer_wildcards.specificity() > more_wildcards.specificity());
    }

    #[test]
    fn non_descendant_outranks_descendant_among_otherwise_equal_selectors() {
        let descendant = ResourceSelector::new(
            "acta",
            vec![literal("workspace", "w1"), SelectorSegment::Any],
            true,
        )
        .expect("valid resource selector");
        let non_descendant = ResourceSelector::new(
            "acta",
            vec![literal("workspace", "w1"), SelectorSegment::Any],
            false,
        )
        .expect("valid resource selector");
        assert!(non_descendant.specificity() > descendant.specificity());
    }

    #[test]
    fn single_wildcard_selector_outranks_open_ended_one_at_equal_literals() {
        let open_ended: ResourceSelector = "acta::workspace::w1/**"
            .parse()
            .expect("valid resource selector");
        let single_wildcard: ResourceSelector = "acta::workspace::w1/*"
            .parse()
            .expect("valid resource selector");
        assert!(single_wildcard.specificity() > open_ended.specificity());
    }

    #[test]
    fn equal_specificity_selectors_compare_equal() {
        let a: ResourceSelector = "acta::workspace::w1/project::p1"
            .parse()
            .expect("valid resource selector");
        let b: ResourceSelector = "acta::workspace::w2/project::p2"
            .parse()
            .expect("valid resource selector");
        assert_eq!(a.specificity(), b.specificity());
    }

    #[test]
    fn max_by_key_over_mixed_candidates_picks_the_most_specific() {
        let by_ref: ResourceRef = "acta::document::d1".parse().expect("valid resource ref");
        let by_path: ResourcePath = "acta::document::d1".parse().expect("valid resource path");
        let by_selector: ResourceSelector = "acta::**".parse().expect("valid resource selector");

        let candidates = [
            by_ref.specificity(),
            by_path.specificity(),
            by_selector.specificity(),
        ];
        let winner = candidates
            .into_iter()
            .max_by_key(|specificity| *specificity)
            .expect("non-empty candidates");

        assert_eq!(winner, by_ref.specificity());
    }

    fn literal(kind: &str, id: &str) -> SelectorSegment {
        SelectorSegment::Literal(
            crate::ids::PathSegment::new(kind, id).expect("valid path segment"),
        )
    }
}
