use crate::ids::{PrincipalId, UserId};
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

/// A first-class agent principal with its owning human user (`v2-e4-s3a-agents`).
/// Unlike [`Principal`] — the shared mirror record, whose owner is NULL for
/// `user` rows — an `Agent` always carries an owner: the
/// `custos_principals_kind_owner_check` invariant guarantees every `agent`
/// principal has exactly one owning human user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: PrincipalId,
    pub display_name: String,
    pub deactivated_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub owner_user_id: UserId,
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
