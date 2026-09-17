use crate::ids::PrincipalId;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The stable identity shared by every actor kind. Introduced additively in
/// E4-S1: `custos.principals` is the source of truth for `kind`,
/// `display_name` and `deactivated_at`; `users` and `api_keys` keep their own
/// columns for compatibility and are written in the same transaction.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Principal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    pub display_name: String,
    pub deactivated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPrincipal {
    pub id: PrincipalId,
    pub kind: PrincipalKind,
    pub display_name: String,
    pub deactivated_at: Option<DateTime<Utc>>,
}

/// The two principal kinds the spec defines. Every V1 `ApiKeyType`
/// (`agent|cli|bot|integration`) collapses to `Agent`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrincipalKind {
    User,
    Agent,
}

impl PrincipalKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            PrincipalKind::User => "user",
            PrincipalKind::Agent => "agent",
        }
    }
}

impl std::str::FromStr for PrincipalKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "user" => Ok(PrincipalKind::User),
            "agent" => Ok(PrincipalKind::Agent),
            other => Err(format!("unknown principal kind: {other}")),
        }
    }
}
