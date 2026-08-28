use std::fmt;
use std::str::FromStr;

use super::impl_string_conversions;
use super::resource_path::{PathSegment, ResourcePath};
use super::segment::{SegmentError, split_element, validate_segment};

/// One element of a `ResourceSelector`: either a literal `<kind>::<id>` or
/// the single-element wildcard `*`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SelectorSegment {
    Literal(PathSegment),
    Any,
}

impl fmt::Display for SelectorSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Literal(segment) => write!(f, "{segment}"),
            Self::Any => write!(f, "*"),
        }
    }
}

/// A pattern over `ResourcePath` values, shaped as
/// `<product>::<element>[/<element>]*` where an element is a literal
/// `<kind>::<id>`, the single-element wildcard `*`, or a trailing `**`
/// that matches zero or more descendant elements.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourceSelector {
    product: String,
    segments: Vec<SelectorSegment>,
    descendants: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceSelectorParseError {
    #[error("resource selector is empty")]
    Empty,
    #[error("resource selector must start with `<product>::`")]
    MissingProduct,
    #[error("element {index} must be `<kind>::<id>`, `*`, or a trailing `**`")]
    ElementShape { index: usize },
    #[error("`**` is only allowed as the final element")]
    MisplacedDescendants { index: usize },
    #[error("invalid {field} in element {index}: {source}")]
    Segment {
        index: usize,
        field: &'static str,
        #[source]
        source: SegmentError,
    },
    #[error("invalid product: {source}")]
    Product {
        #[source]
        source: SegmentError,
    },
}

impl ResourceSelector {
    pub fn new(
        product: &str,
        segments: Vec<SelectorSegment>,
        descendants: bool,
    ) -> Result<Self, ResourceSelectorParseError> {
        validate_segment(product)
            .map_err(|source| ResourceSelectorParseError::Product { source })?;

        if segments.is_empty() && !descendants {
            return Err(ResourceSelectorParseError::Empty);
        }

        Ok(Self {
            product: product.to_string(),
            segments,
            descendants,
        })
    }

    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn segments(&self) -> &[SelectorSegment] {
        &self.segments
    }

    /// Whether the selector accepts zero or more trailing descendant
    /// elements past its explicit segments (a trailing `**`).
    pub fn is_open_ended(&self) -> bool {
        self.descendants
    }

    /// Reports whether `path` is matched by this selector: same product,
    /// every explicit segment consumed in order against the path's
    /// segments, and no leftover path elements unless the selector is
    /// open-ended.
    pub fn matches(&self, path: &ResourcePath) -> bool {
        if self.product != path.product() {
            return false;
        }

        let mut remaining = path.segments();

        for segment in &self.segments {
            let Some(actual) = remaining.next() else {
                return false;
            };

            match segment {
                SelectorSegment::Any => {}
                SelectorSegment::Literal(expected) if expected == actual => {}
                SelectorSegment::Literal(_) => return false,
            }
        }

        self.descendants || remaining.next().is_none()
    }
}

fn parse_selector_element(
    element: &str,
    index: usize,
) -> Result<SelectorSegment, ResourceSelectorParseError> {
    if element == "*" {
        return Ok(SelectorSegment::Any);
    }

    let (kind, id) =
        split_element(element).ok_or(ResourceSelectorParseError::ElementShape { index })?;

    PathSegment::new(kind, id)
        .map(SelectorSegment::Literal)
        .map_err(|(field, source)| ResourceSelectorParseError::Segment {
            index,
            field,
            source,
        })
}

impl FromStr for ResourceSelector {
    type Err = ResourceSelectorParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ResourceSelectorParseError::Empty);
        }

        let (product, rest) = s
            .split_once("::")
            .ok_or(ResourceSelectorParseError::MissingProduct)?;
        validate_segment(product)
            .map_err(|source| ResourceSelectorParseError::Product { source })?;

        let elements: Vec<&str> = rest.split('/').collect();
        let last_index = elements.len().saturating_sub(1);
        let mut segments = Vec::new();
        let mut descendants = false;

        for (index, element) in elements.into_iter().enumerate() {
            if element == "**" {
                if index != last_index {
                    return Err(ResourceSelectorParseError::MisplacedDescendants { index });
                }
                descendants = true;
                continue;
            }

            segments.push(parse_selector_element(element, index)?);
        }

        Self::new(product, segments, descendants)
    }
}

impl fmt::Display for ResourceSelector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.product)?;

        let mut wrote_element = false;
        let mut segments = self.segments.iter();
        if let Some(first) = segments.next() {
            write!(f, "::{first}")?;
            wrote_element = true;
        }

        for segment in segments {
            write!(f, "/{segment}")?;
        }

        if self.descendants {
            write!(f, "{}**", if wrote_element { "/" } else { "::" })?;
        }

        Ok(())
    }
}

impl_string_conversions!(ResourceSelector, ResourceSelectorParseError);

#[cfg(test)]
mod tests {
    use super::*;

    fn literal(kind: &str, id: &str) -> SelectorSegment {
        SelectorSegment::Literal(PathSegment::new(kind, id).expect("valid segment"))
    }

    fn path(raw: &str) -> ResourcePath {
        raw.parse().expect("valid path")
    }

    #[test]
    fn display_matches_literal_or_wildcard_shape() {
        let literal = literal("workspace", "w_01");
        assert_eq!(literal.to_string(), "workspace::w_01");
        assert_eq!(SelectorSegment::Any.to_string(), "*");
    }

    #[test]
    fn new_builds_selectors_across_literal_wildcard_and_descendant_shapes() {
        let root = literal("workspace", "w_01");

        let middle_any =
            ResourceSelector::new("acta", vec![root.clone(), SelectorSegment::Any], false)
                .expect("valid selector");
        assert_eq!(middle_any.segments().len(), 2);
        assert!(!middle_any.is_open_ended());

        let terminal_descendants =
            ResourceSelector::new("acta", vec![root], true).expect("valid selector");
        assert!(terminal_descendants.is_open_ended());

        let bare_descendants =
            ResourceSelector::new("acta", Vec::new(), true).expect("valid selector");
        assert!(bare_descendants.segments().is_empty());
        assert!(bare_descendants.is_open_ended());
    }

    #[test]
    fn round_trips_through_display_parse_and_serde() {
        let cases = [
            "acta::workspace::w_01/project::p_01/*",
            "acta::workspace::w_01/project::p_01/folder::f_01/**",
        ];

        for raw in cases {
            let selector: ResourceSelector = raw.parse().expect("valid selector");
            assert_eq!(selector.to_string(), raw);
            assert_eq!(
                raw.parse::<ResourceSelector>().expect("round trip parse"),
                selector
            );

            let json = serde_json::to_string(&selector).expect("serialize");
            assert_eq!(json, format!("\"{raw}\""));
            assert_eq!(
                serde_json::from_str::<ResourceSelector>(&json).expect("deserialize"),
                selector
            );
        }
    }

    #[test]
    fn rejects_malformed_inputs() {
        let cases = [
            ("", ResourceSelectorParseError::Empty),
            (
                "no-product-here",
                ResourceSelectorParseError::MissingProduct,
            ),
            (
                "acta::workspace::w_01/**/project::p_01",
                ResourceSelectorParseError::MisplacedDescendants { index: 1 },
            ),
            (
                "acta::doc*ument::w_01",
                ResourceSelectorParseError::Segment {
                    index: 0,
                    field: "kind",
                    source: SegmentError::Reserved { ch: '*' },
                },
            ),
            (
                "acta::***",
                ResourceSelectorParseError::ElementShape { index: 0 },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse::<ResourceSelector>().unwrap_err(), expected);
        }
    }

    #[test]
    fn matches_the_expected_paths() {
        let ws_project = "acta::workspace::w_01/project::p_01";
        let ws_project_folder = "acta::workspace::w_01/project::p_01/folder::f_01";
        let cases = [
            (ws_project, ws_project, true),
            ("acta::workspace::w_01/*", ws_project, true),
            ("acta::workspace::w_01/*", ws_project_folder, false),
            ("acta::workspace::w_01/**", "acta::workspace::w_01", true),
            ("acta::workspace::w_01/**", ws_project_folder, true),
            (ws_project, "custos::workspace::w_01/project::p_01", false),
            (ws_project, "acta::workspace::w_01", false),
            (ws_project, ws_project_folder, false),
            ("acta::workspace::w_01", "acta::workspace::W_01", false),
        ];

        for (selector, candidate, expected) in cases {
            let selector: ResourceSelector = selector.parse().expect("valid selector");
            assert_eq!(selector.matches(&path(candidate)), expected);
        }
    }
}
