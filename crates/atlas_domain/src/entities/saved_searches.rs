use crate::actor::Actor;
use crate::ids::{ApiKeyId, SavedSearchId, UserId, WorkspaceId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Identifies the ongoing owner of a saved search (user XOR api_key).
///
/// Structurally identical to `Actor` but semantically distinct: `Owner`
/// expresses a persistent ownership relationship, not an audit attribution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Owner {
    User(UserId),
    ApiKey(ApiKeyId),
}

impl Owner {
    pub fn from_actor(actor: &Actor) -> Self {
        match actor {
            Actor::User(uid) => Owner::User(*uid),
            Actor::ApiKey(kid) => Owner::ApiKey(*kid),
        }
    }

    pub fn matches_actor(&self, actor: &Actor) -> bool {
        match (self, actor) {
            (Owner::User(oid), Actor::User(aid)) => oid == aid,
            (Owner::ApiKey(oid), Actor::ApiKey(aid)) => oid == aid,
            _ => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSearch {
    pub id: SavedSearchId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub query: String,
    pub owner: Owner,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewSavedSearch {
    pub name: String,
    pub query: String,
}
