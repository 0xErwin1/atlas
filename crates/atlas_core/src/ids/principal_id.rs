use std::fmt;
use std::str::FromStr;

use super::impl_string_conversions;
use super::segment::{SegmentError, validate_segment};

/// An opaque, kind-erased identifier for a single principal (a user, a
/// service account, or any other actor a provider can name).
///
/// `atlas_core` never interprets this value; it is a foreign key owned by
/// whichever provider issued it. It is intentionally not a `ResourceRef` and
/// has no conversion to or from one.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrincipalId(String);

/// The error returned when a string is not a valid `PrincipalId`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid principal id: {source}")]
pub struct PrincipalIdParseError {
    #[source]
    source: SegmentError,
}

impl PrincipalId {
    /// Validates and wraps `value` as a `PrincipalId`.
    pub fn new(value: &str) -> Result<Self, PrincipalIdParseError> {
        validate_segment(value).map_err(|source| PrincipalIdParseError { source })?;
        Ok(Self(value.to_string()))
    }

    /// Returns the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for PrincipalId {
    type Err = PrincipalIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(PrincipalId, PrincipalIdParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_principal_id_with_accessor() {
        let id: PrincipalId = "u_42".parse().expect("valid principal id");
        assert_eq!(id.as_str(), "u_42");
    }

    #[test]
    fn rejects_invalid_inputs() {
        let cases = [
            ("", SegmentError::Empty),
            ("a:b", SegmentError::Reserved { ch: ':' }),
            ("a/b", SegmentError::Reserved { ch: '/' }),
            ("a*b", SegmentError::Reserved { ch: '*' }),
        ];

        for (input, expected_source) in cases {
            let err = input.parse::<PrincipalId>().unwrap_err();
            assert_eq!(
                err,
                PrincipalIdParseError {
                    source: expected_source
                }
            );
        }
    }

    #[test]
    fn display_matches_input_and_round_trips_through_parse() {
        let id = PrincipalId::new("u_42").expect("valid principal id");
        assert_eq!(id.to_string(), "u_42");

        let parsed: PrincipalId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = PrincipalId::new("u_42").expect("valid principal id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"u_42\"");

        let parsed: PrincipalId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<PrincipalId, _> = serde_json::from_str("\"bad:id\"");
        assert!(result.is_err());
    }
}
