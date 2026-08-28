use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

use super::name::{RegistryIdError, validate_flat_id};

/// The name of a database schema owned by a component: `acta`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaId(String);

impl SchemaId {
    pub fn new(value: &str) -> Result<Self, RegistryIdError> {
        validate_flat_id("schema id", value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SchemaId {
    type Err = RegistryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for SchemaId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(SchemaId, RegistryIdError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_schema_id_with_accessor() {
        let id: SchemaId = "acta".parse().expect("valid schema id");
        assert_eq!(id.as_str(), "acta");
    }

    #[test]
    fn display_matches_input_and_round_trips_through_parse() {
        let id = SchemaId::new("acta").expect("valid schema id");
        assert_eq!(id.to_string(), "acta");

        let parsed: SchemaId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rejects_dotted_input_with_schema_id_type() {
        let result = SchemaId::new("a.b");
        assert_eq!(
            result,
            Err(RegistryIdError::Charset {
                id_type: "schema id",
                ch: '.'
            })
        );
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = SchemaId::new("acta").expect("valid schema id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"acta\"");

        let parsed: SchemaId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<SchemaId, _> = serde_json::from_str("\"bad:id\"");
        assert!(result.is_err());
    }
}
