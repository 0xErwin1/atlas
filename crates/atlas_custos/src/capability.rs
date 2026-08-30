use serde::{Deserialize, Serialize};

/// The resource family a capability governs. Together with `CapabilityAction`
/// forms the closed `family:action` catalog that API key scopes are drawn from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityFamily {
    Tasks,
    Docs,
    Boards,
    Folders,
    Projects,
    Webhooks,
    Config,
    Grants,
    SavedSearches,
    TaskViews,
}

/// The CRUD verb of a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CapabilityAction {
    Read,
    Create,
    Update,
    Delete,
}

/// A single `family:action` capability, e.g. `tasks:read`. This is the unit of
/// an API key's scope set. The catalog (`Capability::ALL`) is the cross product
/// of families and actions, except `grants`, which is read-only and so
/// contributes only `grants:read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability {
    pub family: CapabilityFamily,
    pub action: CapabilityAction,
}

impl Capability {
    /// The closed catalog of every valid capability, in `family:action` order
    /// with families ordered `tasks, docs, boards, folders, projects, webhooks,
    /// config` and actions ordered `read, create, update, delete`. This is the
    /// single source of truth other derived sets (defaults, wire enums) are
    /// built from.
    pub const ALL: [Capability; 37] = [
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Webhooks,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::Config,
            action: CapabilityAction::Delete,
        },
        // `grants` is read-only: grant WRITES stay domain-blocked for agents by
        // `authorize_share` (AgentsNeverManageGrants), so the catalog exposes
        // only `grants:read` and no grant-write capability can ever be granted.
        Capability {
            family: CapabilityFamily::Grants,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::SavedSearches,
            action: CapabilityAction::Delete,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Create,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Update,
        },
        Capability {
            family: CapabilityFamily::TaskViews,
            action: CapabilityAction::Delete,
        },
    ];

    /// The scope set a newly created API key receives when the caller selects
    /// no scopes: read access to every family, write access to none.
    pub const DEFAULT_READ_ONLY: [Capability; 5] = [
        Capability {
            family: CapabilityFamily::Tasks,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Docs,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Boards,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Folders,
            action: CapabilityAction::Read,
        },
        Capability {
            family: CapabilityFamily::Projects,
            action: CapabilityAction::Read,
        },
    ];

    /// The wire/storage representation, e.g. `"tasks:read"`. Stable and used
    /// both for the `TEXT[]` storage column and the wire DTO.
    pub fn as_str(&self) -> &'static str {
        match (self.family, self.action) {
            (CapabilityFamily::Tasks, CapabilityAction::Read) => "tasks:read",
            (CapabilityFamily::Tasks, CapabilityAction::Create) => "tasks:create",
            (CapabilityFamily::Tasks, CapabilityAction::Update) => "tasks:update",
            (CapabilityFamily::Tasks, CapabilityAction::Delete) => "tasks:delete",
            (CapabilityFamily::Docs, CapabilityAction::Read) => "docs:read",
            (CapabilityFamily::Docs, CapabilityAction::Create) => "docs:create",
            (CapabilityFamily::Docs, CapabilityAction::Update) => "docs:update",
            (CapabilityFamily::Docs, CapabilityAction::Delete) => "docs:delete",
            (CapabilityFamily::Boards, CapabilityAction::Read) => "boards:read",
            (CapabilityFamily::Boards, CapabilityAction::Create) => "boards:create",
            (CapabilityFamily::Boards, CapabilityAction::Update) => "boards:update",
            (CapabilityFamily::Boards, CapabilityAction::Delete) => "boards:delete",
            (CapabilityFamily::Folders, CapabilityAction::Read) => "folders:read",
            (CapabilityFamily::Folders, CapabilityAction::Create) => "folders:create",
            (CapabilityFamily::Folders, CapabilityAction::Update) => "folders:update",
            (CapabilityFamily::Folders, CapabilityAction::Delete) => "folders:delete",
            (CapabilityFamily::Projects, CapabilityAction::Read) => "projects:read",
            (CapabilityFamily::Projects, CapabilityAction::Create) => "projects:create",
            (CapabilityFamily::Projects, CapabilityAction::Update) => "projects:update",
            (CapabilityFamily::Projects, CapabilityAction::Delete) => "projects:delete",
            (CapabilityFamily::Webhooks, CapabilityAction::Read) => "webhooks:read",
            (CapabilityFamily::Webhooks, CapabilityAction::Create) => "webhooks:create",
            (CapabilityFamily::Webhooks, CapabilityAction::Update) => "webhooks:update",
            (CapabilityFamily::Webhooks, CapabilityAction::Delete) => "webhooks:delete",
            (CapabilityFamily::Config, CapabilityAction::Read) => "config:read",
            (CapabilityFamily::Config, CapabilityAction::Create) => "config:create",
            (CapabilityFamily::Config, CapabilityAction::Update) => "config:update",
            (CapabilityFamily::Config, CapabilityAction::Delete) => "config:delete",
            // The grant-write arms keep this match total; because `grants:read`
            // is the only grants entry in `Capability::ALL` and `FromStr`
            // iterates `ALL`, these write strings are never produced or parsed.
            (CapabilityFamily::Grants, CapabilityAction::Read) => "grants:read",
            (CapabilityFamily::Grants, CapabilityAction::Create) => "grants:create",
            (CapabilityFamily::Grants, CapabilityAction::Update) => "grants:update",
            (CapabilityFamily::Grants, CapabilityAction::Delete) => "grants:delete",
            (CapabilityFamily::SavedSearches, CapabilityAction::Read) => "saved_searches:read",
            (CapabilityFamily::SavedSearches, CapabilityAction::Create) => "saved_searches:create",
            (CapabilityFamily::SavedSearches, CapabilityAction::Update) => "saved_searches:update",
            (CapabilityFamily::SavedSearches, CapabilityAction::Delete) => "saved_searches:delete",
            (CapabilityFamily::TaskViews, CapabilityAction::Read) => "task_views:read",
            (CapabilityFamily::TaskViews, CapabilityAction::Create) => "task_views:create",
            (CapabilityFamily::TaskViews, CapabilityAction::Update) => "task_views:update",
            (CapabilityFamily::TaskViews, CapabilityAction::Delete) => "task_views:delete",
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Capability::ALL
            .into_iter()
            .find(|cap| cap.as_str() == s)
            .ok_or_else(|| format!("unknown capability: {s}"))
    }
}
