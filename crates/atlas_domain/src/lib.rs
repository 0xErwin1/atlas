#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("entity not found: {id}")]
    NotFound { id: String },
    #[error("invalid input: {message}")]
    InvalidInput { message: String },
    #[error("internal error: {message}")]
    Internal { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityId(pub Uuid);

impl EntityId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EntityId {
    fn default() -> Self {
        Self::new()
    }
}

pub trait HealthProbe {
    fn ping(&self) -> Result<(), DomainError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entity_id_serde_round_trip() {
        let id = EntityId::new();
        let json = serde_json::to_string(&id).unwrap();
        let decoded: EntityId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, decoded);
    }
}
