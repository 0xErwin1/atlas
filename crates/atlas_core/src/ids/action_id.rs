use std::fmt;
use std::str::FromStr;

use super::impl_string_conversions;
use super::segment::{SegmentError, validate_segment};

/// A fully qualified identifier for an action, shaped as
/// `<product>::<kind>::<action>`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ActionId {
    product: String,
    kind: String,
    action: String,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ActionIdParseError {
    #[error("action id must be `<product>::<kind>::<action>`")]
    Shape,
    #[error("invalid {field} in action id: {source}")]
    Segment {
        field: &'static str,
        #[source]
        source: SegmentError,
    },
}

impl ActionId {
    pub fn new(product: &str, kind: &str, action: &str) -> Result<Self, ActionIdParseError> {
        validate_segment(product).map_err(|source| ActionIdParseError::Segment {
            field: "product",
            source,
        })?;
        validate_segment(kind).map_err(|source| ActionIdParseError::Segment {
            field: "kind",
            source,
        })?;
        validate_segment(action).map_err(|source| ActionIdParseError::Segment {
            field: "action",
            source,
        })?;

        Ok(Self {
            product: product.to_string(),
            kind: kind.to_string(),
            action: action.to_string(),
        })
    }

    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn kind(&self) -> &str {
        &self.kind
    }

    pub fn action(&self) -> &str {
        &self.action
    }
}

impl FromStr for ActionId {
    type Err = ActionIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split("::");
        let (Some(product), Some(kind), Some(action), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(ActionIdParseError::Shape);
        };

        Self::new(product, kind, action)
    }
}

impl fmt::Display for ActionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}::{}", self.product, self.kind, self.action)
    }
}

impl_string_conversions!(ActionId, ActionIdParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_action_id_with_accessors() {
        let id: ActionId = "acta::task::create".parse().expect("valid action id");
        assert_eq!(id.product(), "acta");
        assert_eq!(id.kind(), "task");
        assert_eq!(id.action(), "create");
    }

    #[test]
    fn rejects_invalid_inputs() {
        let segment = |field, source| ActionIdParseError::Segment { field, source };
        let cases = [
            ("acta::task", ActionIdParseError::Shape),
            ("acta::task::create::extra", ActionIdParseError::Shape),
            ("::task::create", segment("product", SegmentError::Empty)),
            ("acta::::create", segment("kind", SegmentError::Empty)),
            ("acta::task::", segment("action", SegmentError::Empty)),
            (
                "acta::doc*ument::read",
                segment("kind", SegmentError::Reserved { ch: '*' }),
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse::<ActionId>().unwrap_err(), expected);
        }
    }

    #[test]
    fn display_matches_canonical_shape_and_round_trips_through_parse() {
        let id = ActionId::new("acta", "task", "create").expect("valid action id");
        assert_eq!(id.to_string(), "acta::task::create");

        let parsed: ActionId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn ordering_is_stable() {
        let a = ActionId::new("acta", "task", "create").expect("valid action id");
        let b = ActionId::new("acta", "task", "delete").expect("valid action id");
        assert!(a < b);
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = ActionId::new("acta", "task", "create").expect("valid action id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"acta::task::create\"");

        let parsed: ActionId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<ActionId, _> = serde_json::from_str("\"not an action\"");
        assert!(result.is_err());
    }
}
