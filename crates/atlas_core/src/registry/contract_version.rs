use std::fmt;

use serde::{Deserialize, Serialize};

/// A protocol contract version, ordered numerically (no semver).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContractVersion(u32);

impl ContractVersion {
    pub const fn new(value: u32) -> Self {
        Self(value)
    }

    pub const fn value(&self) -> u32 {
        self.0
    }
}

impl From<u32> for ContractVersion {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl fmt::Display for ContractVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Inclusive `[min, max]` protocol range (SHELL-INT-3 `compatible_range`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContractVersionRange {
    min: ContractVersion,
    max: ContractVersion,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractVersionRangeError {
    #[error("contract version range min {min} is greater than max {max}")]
    MinGreaterThanMax {
        min: ContractVersion,
        max: ContractVersion,
    },
}

impl ContractVersionRange {
    pub fn new(
        min: ContractVersion,
        max: ContractVersion,
    ) -> Result<Self, ContractVersionRangeError> {
        if min > max {
            return Err(ContractVersionRangeError::MinGreaterThanMax { min, max });
        }

        Ok(Self { min, max })
    }

    pub fn min(&self) -> ContractVersion {
        self.min
    }

    pub fn max(&self) -> ContractVersion {
        self.max
    }

    pub fn contains(&self, version: ContractVersion) -> bool {
        self.min <= version && version <= self.max
    }
}

impl fmt::Display for ContractVersionRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}..={}", self.min, self.max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn orders_by_numeric_value() {
        assert!(ContractVersion::new(1) < ContractVersion::new(2));
    }

    #[test]
    fn serde_emits_a_bare_number() {
        let version = ContractVersion::new(3);
        let json = serde_json::to_string(&version).expect("serialize");
        assert_eq!(json, "3");

        let parsed: ContractVersion = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, version);
    }

    #[test]
    fn range_rejects_inverted_bounds() {
        let result = ContractVersionRange::new(ContractVersion::new(2), ContractVersion::new(1));
        assert_eq!(
            result.unwrap_err(),
            ContractVersionRangeError::MinGreaterThanMax {
                min: ContractVersion::new(2),
                max: ContractVersion::new(1),
            }
        );
    }

    #[test]
    fn range_allows_equal_bounds() {
        let range = ContractVersionRange::new(ContractVersion::new(2), ContractVersion::new(2))
            .expect("equal bounds are valid");
        assert_eq!(range.min(), ContractVersion::new(2));
        assert_eq!(range.max(), ContractVersion::new(2));
    }

    #[test]
    fn contains_reflects_inclusive_bounds() {
        let range = ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(3))
            .expect("valid range");

        assert!(range.contains(ContractVersion::new(1)));
        assert!(range.contains(ContractVersion::new(2)));
        assert!(range.contains(ContractVersion::new(3)));
        assert!(!range.contains(ContractVersion::new(4)));
    }
}
