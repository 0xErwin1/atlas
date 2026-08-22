use std::fmt;
use std::str::FromStr;

use super::impl_string_conversions;
use super::resource_ref::ResourceRef;
use super::segment::{SegmentError, split_element, validate_segment};

/// One `<kind>::<id>` element of a `ResourcePath`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PathSegment {
    kind: String,
    id: String,
}

impl PathSegment {
    pub(crate) fn new(kind: &str, id: &str) -> Result<Self, (&'static str, SegmentError)> {
        validate_segment(kind).map_err(|source| ("kind", source))?;
        validate_segment(id).map_err(|source| ("id", source))?;

        Ok(Self {
            kind: kind.to_string(),
            id: id.to_string(),
        })
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    fn from_validated_parts(kind: String, id: String) -> Self {
        Self { kind, id }
    }
}

impl fmt::Display for PathSegment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.kind, self.id)
    }
}

/// A concrete path to a resource within a product's hierarchy, shaped as
/// `<product>::<kind>::<id>[/<kind>::<id>]*`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ResourcePath {
    product: String,
    root: PathSegment,
    rest: Vec<PathSegment>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourcePathParseError {
    #[error("resource path is empty")]
    Empty,
    #[error("resource path must start with `<product>::`")]
    MissingProduct,
    #[error("element {index} must be `<kind>::<id>`")]
    ElementShape { index: usize },
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

impl ResourcePath {
    pub fn new(
        product: &str,
        root: PathSegment,
        rest: Vec<PathSegment>,
    ) -> Result<Self, ResourcePathParseError> {
        validate_segment(product).map_err(|source| ResourcePathParseError::Product { source })?;

        Ok(Self {
            product: product.to_string(),
            root,
            rest,
        })
    }

    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn root(&self) -> &PathSegment {
        &self.root
    }

    /// The deepest element of the path, falling back to `root` when the
    /// path has no descendants.
    pub fn leaf(&self) -> &PathSegment {
        self.rest.last().unwrap_or(&self.root)
    }

    pub fn segments(&self) -> impl Iterator<Item = &PathSegment> {
        std::iter::once(&self.root).chain(self.rest.iter())
    }

    /// Collapses the path to its leaf element as a `ResourceRef`. Lossy:
    /// every ancestor element between `product` and the leaf is discarded.
    pub fn leaf_ref(&self) -> ResourceRef {
        let leaf = self.leaf();
        ResourceRef::from_validated_parts(self.product.clone(), leaf.kind.clone(), leaf.id.clone())
    }
}

impl From<ResourceRef> for ResourcePath {
    fn from(resource: ResourceRef) -> Self {
        Self {
            product: resource.product().to_string(),
            root: PathSegment::from_validated_parts(
                resource.kind().to_string(),
                resource.id().to_string(),
            ),
            rest: Vec::new(),
        }
    }
}

fn parse_path_element(element: &str, index: usize) -> Result<PathSegment, ResourcePathParseError> {
    let (kind, id) =
        split_element(element).ok_or(ResourcePathParseError::ElementShape { index })?;

    PathSegment::new(kind, id).map_err(|(field, source)| ResourcePathParseError::Segment {
        index,
        field,
        source,
    })
}

impl FromStr for ResourcePath {
    type Err = ResourcePathParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            return Err(ResourcePathParseError::Empty);
        }

        let (product, rest) = s
            .split_once("::")
            .ok_or(ResourcePathParseError::MissingProduct)?;
        validate_segment(product).map_err(|source| ResourcePathParseError::Product { source })?;

        let mut elements = rest.split('/');
        let root = parse_path_element(elements.next().unwrap_or_default(), 0)?;

        let mut path_rest = Vec::new();
        for (offset, element) in elements.enumerate() {
            path_rest.push(parse_path_element(element, offset + 1)?);
        }

        Ok(Self {
            product: product.to_string(),
            root,
            rest: path_rest,
        })
    }
}

impl fmt::Display for ResourcePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.product, self.root)?;

        for segment in &self.rest {
            write!(f, "/{segment}")?;
        }

        Ok(())
    }
}

impl_string_conversions!(ResourcePath, ResourcePathParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_validates_both_halves_exposes_accessors_and_displays_canonical_shape() {
        let segment = PathSegment::new("workspace", "w_01").expect("valid segment");
        assert_eq!(segment.kind(), "workspace");
        assert_eq!(segment.id(), "w_01");
        assert_eq!(segment.to_string(), "workspace::w_01");
    }

    fn segment(kind: &str, id: &str) -> PathSegment {
        PathSegment::new(kind, id).expect("valid segment")
    }

    #[test]
    fn new_builds_single_and_multi_element_paths() {
        let root = segment("workspace", "w_01");
        let single = ResourcePath::new("acta", root.clone(), Vec::new()).expect("valid path");
        assert_eq!(single.product(), "acta");
        assert_eq!(single.root(), &root);
        assert_eq!(single.leaf(), &root);

        let child = segment("project", "p_01");
        let multi =
            ResourcePath::new("acta", root.clone(), vec![child.clone()]).expect("valid path");
        assert_eq!(multi.leaf(), &child);
        assert_eq!(multi.segments().collect::<Vec<_>>(), vec![&root, &child]);
    }

    #[test]
    fn round_trips_through_display_parse_and_serde() {
        let raw = "acta::workspace::w_01/project::p_01/folder::f_01/document::d_01";
        let path: ResourcePath = raw.parse().expect("valid path");
        assert_eq!(path.to_string(), raw);
        assert_eq!(raw.parse::<ResourcePath>().expect("round trip parse"), path);

        let json = serde_json::to_string(&path).expect("serialize");
        assert_eq!(json, format!("\"{raw}\""));
        assert_eq!(
            serde_json::from_str::<ResourcePath>(&json).expect("deserialize"),
            path
        );
    }

    #[test]
    fn leaf_ref_collapses_to_the_deepest_element() {
        let raw = "acta::workspace::w_01/project::p_01/folder::f_01";
        let path: ResourcePath = raw.parse().expect("valid path");
        let leaf_ref = path.leaf_ref();
        assert_eq!(leaf_ref.product(), "acta");
        assert_eq!(leaf_ref.kind(), "folder");
        assert_eq!(leaf_ref.id(), "f_01");
    }

    #[test]
    fn from_resource_ref_builds_a_single_element_path_that_round_trips() {
        let resource = ResourceRef::new("acta", "document", "42").expect("valid resource ref");
        let path = ResourcePath::from(resource.clone());
        assert_eq!(path.product(), "acta");
        assert_eq!(path.leaf().kind(), "document");
        assert_eq!(path.leaf().id(), "42");
        assert_eq!(path.leaf_ref(), resource);
    }

    #[test]
    fn rejects_malformed_inputs() {
        let segment = |index, field, source| ResourcePathParseError::Segment {
            index,
            field,
            source,
        };
        let cases = [
            ("", ResourcePathParseError::Empty),
            ("no-product-here", ResourcePathParseError::MissingProduct),
            (
                "acta::workspace::w_01/project",
                ResourcePathParseError::ElementShape { index: 1 },
            ),
            (
                "acta::workspace::w_01/",
                ResourcePathParseError::ElementShape { index: 1 },
            ),
            (
                "acta::workspace::w_01//project::p_01",
                ResourcePathParseError::ElementShape { index: 1 },
            ),
            (
                "::workspace::w_01",
                ResourcePathParseError::Product {
                    source: SegmentError::Empty,
                },
            ),
            (
                "acta::doc*ument::w_01",
                segment(0, "kind", SegmentError::Reserved { ch: '*' }),
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse::<ResourcePath>().unwrap_err(), expected);
        }
    }
}
