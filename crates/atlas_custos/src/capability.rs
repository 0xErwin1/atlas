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

/// A single capability, canonically spelled `<product>::<kind>::<action>`
/// (e.g. `acta::tasks::read`). This is the unit of an API key's scope set.
/// The catalog (`Capability::ALL`) is the cross product of families and
/// actions, except `grants`, which is read-only and so contributes only
/// `custos::grants::read`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Capability {
    pub family: CapabilityFamily,
    pub action: CapabilityAction,
}

impl Capability {
    /// The closed catalog of every valid capability, with families ordered
    /// `tasks, docs, boards, folders, projects, webhooks, config, grants,
    /// saved_searches, task_views` and actions ordered `read, create, update,
    /// delete`. Every entry's wire/storage spelling is the canonical
    /// `<product>::<kind>::<action>` form (e.g. `acta::tasks::read`,
    /// `custos::grants::read`). This is the single source of truth other
    /// derived sets (defaults, wire enums) are built from.
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
    /// no scopes: read access to the five default families (`tasks`, `docs`,
    /// `boards`, `folders`, `projects`), write access to none.
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

    /// The canonical wire/storage representation, e.g.
    /// `"acta::tasks::read"`. Stable and used both for the `TEXT[]` storage
    /// column and the wire DTO. Rows stored in the legacy `<family>:<action>`
    /// spelling still parse via `Capability::from_str`'s dual accept; only
    /// the canonical form is ever written.
    pub fn as_str(&self) -> &'static str {
        match (self.family, self.action) {
            (CapabilityFamily::Tasks, CapabilityAction::Read) => "acta::tasks::read",
            (CapabilityFamily::Tasks, CapabilityAction::Create) => "acta::tasks::create",
            (CapabilityFamily::Tasks, CapabilityAction::Update) => "acta::tasks::update",
            (CapabilityFamily::Tasks, CapabilityAction::Delete) => "acta::tasks::delete",
            (CapabilityFamily::Docs, CapabilityAction::Read) => "acta::docs::read",
            (CapabilityFamily::Docs, CapabilityAction::Create) => "acta::docs::create",
            (CapabilityFamily::Docs, CapabilityAction::Update) => "acta::docs::update",
            (CapabilityFamily::Docs, CapabilityAction::Delete) => "acta::docs::delete",
            (CapabilityFamily::Boards, CapabilityAction::Read) => "acta::boards::read",
            (CapabilityFamily::Boards, CapabilityAction::Create) => "acta::boards::create",
            (CapabilityFamily::Boards, CapabilityAction::Update) => "acta::boards::update",
            (CapabilityFamily::Boards, CapabilityAction::Delete) => "acta::boards::delete",
            (CapabilityFamily::Folders, CapabilityAction::Read) => "acta::folders::read",
            (CapabilityFamily::Folders, CapabilityAction::Create) => "acta::folders::create",
            (CapabilityFamily::Folders, CapabilityAction::Update) => "acta::folders::update",
            (CapabilityFamily::Folders, CapabilityAction::Delete) => "acta::folders::delete",
            (CapabilityFamily::Projects, CapabilityAction::Read) => "acta::projects::read",
            (CapabilityFamily::Projects, CapabilityAction::Create) => "acta::projects::create",
            (CapabilityFamily::Projects, CapabilityAction::Update) => "acta::projects::update",
            (CapabilityFamily::Projects, CapabilityAction::Delete) => "acta::projects::delete",
            (CapabilityFamily::Webhooks, CapabilityAction::Read) => "acta::webhooks::read",
            (CapabilityFamily::Webhooks, CapabilityAction::Create) => "acta::webhooks::create",
            (CapabilityFamily::Webhooks, CapabilityAction::Update) => "acta::webhooks::update",
            (CapabilityFamily::Webhooks, CapabilityAction::Delete) => "acta::webhooks::delete",
            (CapabilityFamily::Config, CapabilityAction::Read) => "acta::config::read",
            (CapabilityFamily::Config, CapabilityAction::Create) => "acta::config::create",
            (CapabilityFamily::Config, CapabilityAction::Update) => "acta::config::update",
            (CapabilityFamily::Config, CapabilityAction::Delete) => "acta::config::delete",
            // The grant-write arms keep this match total; because
            // `custos::grants::read` is the only grants entry in
            // `Capability::ALL` and `FromStr` iterates `ALL`, these write
            // strings are never produced or parsed.
            (CapabilityFamily::Grants, CapabilityAction::Read) => "custos::grants::read",
            (CapabilityFamily::Grants, CapabilityAction::Create) => "custos::grants::create",
            (CapabilityFamily::Grants, CapabilityAction::Update) => "custos::grants::update",
            (CapabilityFamily::Grants, CapabilityAction::Delete) => "custos::grants::delete",
            (CapabilityFamily::SavedSearches, CapabilityAction::Read) => {
                "acta::saved_searches::read"
            }
            (CapabilityFamily::SavedSearches, CapabilityAction::Create) => {
                "acta::saved_searches::create"
            }
            (CapabilityFamily::SavedSearches, CapabilityAction::Update) => {
                "acta::saved_searches::update"
            }
            (CapabilityFamily::SavedSearches, CapabilityAction::Delete) => {
                "acta::saved_searches::delete"
            }
            (CapabilityFamily::TaskViews, CapabilityAction::Read) => "acta::task_views::read",
            (CapabilityFamily::TaskViews, CapabilityAction::Create) => "acta::task_views::create",
            (CapabilityFamily::TaskViews, CapabilityAction::Update) => "acta::task_views::update",
            (CapabilityFamily::TaskViews, CapabilityAction::Delete) => "acta::task_views::delete",
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    /// Accepts both the canonical `<product>::<kind>::<action>` spelling and
    /// the legacy `<family>:<action>` spelling (which pre-canonicalization
    /// stored rows still carry). Anything else is rejected, so an unknown
    /// string can never coerce into a granted capability.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Capability::ALL
            .into_iter()
            .find(|cap| cap.as_str() == s)
            .or_else(|| {
                legacy_to_canonical(s).and_then(|canonical| {
                    Capability::ALL
                        .into_iter()
                        .find(|cap| cap.as_str() == canonical)
                })
            })
            .ok_or_else(|| format!("unknown capability: {s}"))
    }
}

/// Translates the legacy `<family>:<action>` spelling to its canonical
/// `<product>::<family>::<action>` form. The family vocabulary is closed:
/// nine acta families plus the read-only `grants` family under custos.
fn legacy_to_canonical(s: &str) -> Option<String> {
    let (family, action) = s.split_once(':')?;
    let product = match family {
        "tasks" | "docs" | "boards" | "folders" | "projects" | "webhooks" | "config"
        | "saved_searches" | "task_views" => "acta",
        "grants" => "custos",
        _ => return None,
    };
    Some(format!("{product}::{family}::{action}"))
}
