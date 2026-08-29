use std::fmt;
use std::str::FromStr;

use super::resource_ref::{ResourceRef, ResourceRefParseError};
use super::segment::{SegmentError, validate_segment};

/// The identifier of a named principal set attached to a resource, shaped as
/// `<scope>::<set>` where `<scope>` is a `ResourceRef` (e.g.
/// `acta::workspace::w_01::members`).
///
/// A set name always names a set declared by a provider (see
/// `ProviderCatalog::principal_sets`), never a concrete standalone group.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PrincipalSetId {
    scope: ResourceRef,
    set: String,
}

/// The error returned when a string is not a valid `PrincipalSetId`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PrincipalSetIdParseError {
    /// The input has no `::<set>` suffix to split off.
    #[error("principal set id must be `<scope>::<set>`")]
    Shape,
    /// The scope prefix is not a valid `ResourceRef`.
    #[error("invalid scope in principal set id: {source}")]
    Scope {
        #[source]
        source: ResourceRefParseError,
    },
    /// The set name suffix is not a valid segment.
    #[error("invalid set name in principal set id: {source}")]
    SetName {
        #[source]
        source: SegmentError,
    },
}

impl PrincipalSetId {
    /// Builds a `PrincipalSetId` from an already-parsed scope and a set name,
    /// validating only the set name.
    pub fn new(scope: ResourceRef, set: &str) -> Result<Self, PrincipalSetIdParseError> {
        validate_segment(set).map_err(|source| PrincipalSetIdParseError::SetName { source })?;
        Ok(Self {
            scope,
            set: set.to_string(),
        })
    }

    /// Returns the resource scope this set is attached to.
    pub fn scope(&self) -> &ResourceRef {
        &self.scope
    }

    /// Returns the set name (e.g. `members`).
    pub fn set(&self) -> &str {
        &self.set
    }
}

impl FromStr for PrincipalSetId {
    type Err = PrincipalSetIdParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let (scope, set) = s.rsplit_once("::").ok_or(PrincipalSetIdParseError::Shape)?;

        if scope.is_empty() {
            return Err(PrincipalSetIdParseError::Shape);
        }

        let scope: ResourceRef = scope
            .parse()
            .map_err(|source| PrincipalSetIdParseError::Scope { source })?;

        Self::new(scope, set)
    }
}

impl fmt::Display for PrincipalSetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}::{}", self.scope, self.set)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_principal_set_id_with_accessors() {
        let id: PrincipalSetId = "acta::workspace::w_01::members"
            .parse()
            .expect("valid principal set id");

        assert_eq!(id.scope().to_string(), "acta::workspace::w_01");
        assert_eq!(id.set(), "members");
    }

    #[test]
    fn rejects_invalid_inputs() {
        let cases = [
            ("members", PrincipalSetIdParseError::Shape),
            ("::members", PrincipalSetIdParseError::Shape),
            (
                "acta::members",
                PrincipalSetIdParseError::Scope {
                    source: ResourceRefParseError::Shape,
                },
            ),
            (
                "acta::workspace::w_01::",
                PrincipalSetIdParseError::SetName {
                    source: SegmentError::Empty,
                },
            ),
            (
                "acta::workspace::w_01:::extra",
                PrincipalSetIdParseError::Scope {
                    source: ResourceRefParseError::Segment {
                        field: "id",
                        source: SegmentError::Reserved { ch: ':' },
                    },
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(input.parse::<PrincipalSetId>().unwrap_err(), expected);
        }
    }

    #[test]
    fn display_matches_canonical_shape_and_round_trips_through_parse() {
        let scope: ResourceRef = "acta::workspace::w_01".parse().expect("valid resource ref");
        let id = PrincipalSetId::new(scope, "members").expect("valid principal set id");
        assert_eq!(id.to_string(), "acta::workspace::w_01::members");

        let parsed: PrincipalSetId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }
}
