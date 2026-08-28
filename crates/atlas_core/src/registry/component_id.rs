use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

use super::name::{RegistryIdError, validate_dotted_id};

/// Stable identity of a component: `platform`, `acta`, `storage.filesystem`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ComponentId(String);

impl ComponentId {
    pub fn new(value: &str) -> Result<Self, RegistryIdError> {
        validate_dotted_id("component id", value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for ComponentId {
    type Err = RegistryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for ComponentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(ComponentId, RegistryIdError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_component_id_with_accessor() {
        let id: ComponentId = "storage.filesystem".parse().expect("valid component id");
        assert_eq!(id.as_str(), "storage.filesystem");
    }

    #[test]
    fn display_matches_input_and_round_trips_through_parse() {
        let id = ComponentId::new("platform").expect("valid component id");
        assert_eq!(id.to_string(), "platform");

        let parsed: ComponentId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rejects_malformed_input_with_component_id_type() {
        let result = ComponentId::new("A");
        assert_eq!(
            result,
            Err(RegistryIdError::Charset {
                id_type: "component id",
                ch: 'A'
            })
        );
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = ComponentId::new("acta").expect("valid component id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"acta\"");

        let parsed: ComponentId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<ComponentId, _> = serde_json::from_str("\"bad:id\"");
        assert!(result.is_err());
    }
}
