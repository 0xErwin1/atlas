use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

use super::name::{RegistryIdError, validate_dotted_id};

/// A versioned schema contract owned by a component: `custos.principals.v1`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaContractId(String);

impl SchemaContractId {
    pub fn new(value: &str) -> Result<Self, RegistryIdError> {
        validate_dotted_id("schema contract id", value)?;
        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for SchemaContractId {
    type Err = RegistryIdError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for SchemaContractId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(SchemaContractId, RegistryIdError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_valid_schema_contract_id_with_accessor() {
        let id: SchemaContractId = "custos.principals.v1"
            .parse()
            .expect("valid schema contract id");
        assert_eq!(id.as_str(), "custos.principals.v1");
    }

    #[test]
    fn display_matches_input_and_round_trips_through_parse() {
        let id = SchemaContractId::new("custos.principals.v1").expect("valid schema contract id");
        assert_eq!(id.to_string(), "custos.principals.v1");

        let parsed: SchemaContractId = id.to_string().parse().expect("round trip parse");
        assert_eq!(id, parsed);
    }

    #[test]
    fn rejects_malformed_input_with_schema_contract_id_type() {
        let result = SchemaContractId::new("A");
        assert_eq!(
            result,
            Err(RegistryIdError::Charset {
                id_type: "schema contract id",
                ch: 'A'
            })
        );
    }

    #[test]
    fn serde_round_trips_and_rejects_malformed_json() {
        let id = SchemaContractId::new("custos.principals.v1").expect("valid schema contract id");
        let json = serde_json::to_string(&id).expect("serialize");
        assert_eq!(json, "\"custos.principals.v1\"");

        let parsed: SchemaContractId = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(id, parsed);

        let result: Result<SchemaContractId, _> = serde_json::from_str("\"bad:id\"");
        assert!(result.is_err());
    }
}
