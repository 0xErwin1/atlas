use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

use super::name::{RegistryIdError, validate_dotted_id};

/// A capability a component provides or requires: `storage.blob`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapabilityId(String);

impl CapabilityId {
    pub fn new(value: &str) -> Result<Self, RegistryIdError> {
        validate_dotted_id("capability id", value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for CapabilityId {
    type Err = RegistryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(CapabilityId, RegistryIdError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_capability_id_with_accessor() {
        let id: CapabilityId = "storage.blob".parse().expect("valid capability id");
        assert_eq!(id.as_str(), "storage.blob");
    }

    #[test]
    fn display_matches_input_and_round_trips_through_parse() {
        let id = CapabilityId::new("storage.blob").expect("valid capability id");
        assert_eq!(id.to_string(), "storage.blob");

        let parsed: CapabilityId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rejects_malformed_input_with_capability_id_type() {
        let result = CapabilityId::new("A");
        assert_eq!(
            result,
            Err(RegistryIdError::Charset {
                id_type: "capability id",
                ch: 'A'
            })
        );
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = CapabilityId::new("storage.blob").expect("valid capability id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"storage.blob\"");

        let parsed: CapabilityId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<CapabilityId, _> = serde_json::from_str("\"bad:id\"");
        assert!(result.is_err());
    }
}
