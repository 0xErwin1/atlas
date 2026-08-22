use std::fmt;
use std::str::FromStr;

use super::impl_string_conversions;
use super::segment::{SegmentError, validate_segment};

/// A fully qualified identifier for a resource, shaped as
/// `<product>::<kind>::<id>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ResourceRef {
    product: String,
    kind: String,
    id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResourceRefParseError {
    #[error("resource ref must be `<product>::<kind>::<id>`")]
    Shape,
    #[error("invalid {field} in resource ref: {source}")]
    Segment {
        field: &'static str,
        #[source]
        source: SegmentError,
    },
}

impl ResourceRef {
    pub fn new(product: &str, kind: &str, id: &str) -> Result<Self, ResourceRefParseError> {
        validate_segment(product).map_err(|source| ResourceRefParseError::Segment {
            field: "product",
            source,
        })?;
        validate_segment(kind).map_err(|source| ResourceRefParseError::Segment {
            field: "kind",
            source,
        })?;
        validate_segment(id).map_err(|source| ResourceRefParseError::Segment {
            field: "id",
            source,
        })?;

        Ok(Self {
            product: product.to_string(),
            kind: kind.to_string(),
            id: id.to_string(),
        })
    }

    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    /// Builds a `ResourceRef` from already-validated parts, skipping
    /// re-validation. Used by conversions from types that validate their
    /// own segments (for example `ResourcePath::leaf_ref`).
    pub(crate) fn from_validated_parts(product: String, kind: String, id: String) -> Self {
        Self { product, kind, id }
    }
}

impl FromStr for ResourceRef {
    type Err = ResourceRefParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split("::");
        let (Some(product), Some(kind), Some(id), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ResourceRefParseError::Shape);
        };

        Self::new(product, kind, id)
    }
}

impl fmt::Display for ResourceRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}::{}", self.product, self.kind, self.id)
    }
}

impl_string_conversions!(ResourceRef, ResourceRefParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_resource_ref_with_accessors() {
        let id: ResourceRef = "acta::document::42".parse().expect("valid resource ref");
        assert_eq!(id.product(), "acta");
        assert_eq!(id.kind(), "document");
        assert_eq!(id.id(), "42");
    }

    #[test]
    fn rejects_invalid_inputs() {
        let segment = |field, source| ResourceRefParseError::Segment { field, source };
        let cases = [
            ("acta::document", ResourceRefParseError::Shape),
            ("acta::document::42::extra", ResourceRefParseError::Shape),
            ("::document::42", segment("product", SegmentError::Empty)),
            ("acta::::42", segment("kind", SegmentError::Empty)),
            ("acta::document::", segment("id", SegmentError::Empty)),
            (
                "acta::document::d/1",
                segment("id", SegmentError::Reserved { ch: '/' }),
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse::<ResourceRef>().unwrap_err(), expected);
        }
    }

    #[test]
    fn display_matches_canonical_shape_and_round_trips_through_parse() {
        let id = ResourceRef::new("acta", "document", "42").expect("valid resource ref");
        assert_eq!(id.to_string(), "acta::document::42");

        let parsed: ResourceRef = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ordering_is_stable() {
        let a = ResourceRef::new("acta", "document", "1").expect("valid resource ref");
        let b = ResourceRef::new("acta", "document", "2").expect("valid resource ref");
        assert!(a < b);
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = ResourceRef::new("acta", "document", "42").expect("valid resource ref");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"acta::document::42\"");

        let parsed: ResourceRef = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<ResourceRef, _> = serde_json::from_str("\"not a resource\"");
        assert!(result.is_err());
    }
}
