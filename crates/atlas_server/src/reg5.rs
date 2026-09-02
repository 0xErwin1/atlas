//! REG-5 component entries for the initial V2-E3 release set (SHELL-REG-5).
//!
//! Populates the `atlas_core::registry` shape E1 built with the real data for
//! this release: `platform`, `custos`, `acta`, and the four capability
//! Modules. `platform`/`custos`/`acta`'s `api.routes` describe the live
//! `atlas_server` router exactly as `lib.rs::app()` serves it today — no
//! route, path, or URL is added, removed, moved, or renamed by this module.
//! Router derivation from these declarations (`router = registry`) and the
//! `/api/v2/{component}` mount are later V2-E3 slices, not this one.
//!
//! Route ownership (which of the 36 `routes/mod.rs` modules maps to which
//! component) is grounded in `docs/registry-route-ownership.md`, which is the
//! reviewable artifact this file's `api.routes` lists are generated from.
//!
//! `RouteDeclaration.path` here is deliberately the full, absolute path the
//! server serves today (e.g. `/api/workspaces/{ws}/tasks/{readable_id}`),
//! not a namespace-relative path as `entry.rs`'s own illustrative fixture
//! shows (`/tasks/{task_id}`). Making it namespace-relative belongs to
//! router derivation (S2/S3), which decides how a namespace-relative path
//! maps back onto the current mount; declaring it here would already be a
//! guess this slice is not allowed to make. See the ownership artifact for
//! the full rationale.

use atlas_core::ids::ActionId;
use atlas_core::registry::{
    Api, Authorization, Capabilities, CapabilityId, ComponentEntry, ComponentId, ComponentKind,
    ConfigDeclaration, ContractVersion, Dependency, Diagnostics, Experience, HttpMethod, Identity,
    Persistence, RouteDeclaration, RoutePath, SchemaContractId, SchemaId,
};

/// Selects which storage backend Module contributes the mandatory
/// `storage.blob` capability to the entry set `reg5_component_entries`
/// returns.
///
/// `storage.filesystem` and `storage.s3` both provide `storage.blob`, and
/// `atlas_core::registry::build()` rejects two providers of the same
/// mandatory capability (`MandatoryCapabilityAmbiguous`). Exactly one backend
/// is active per process, selected by configuration — this is the same
/// startup composition decision documented on the pre-existing
/// `shell_reg_5_valid_entries()` workspace-test fixture in
/// `atlas_core::registry` (which excludes `storage.s3` for the same reason).
/// PR2 wires the real selection from `atlas_server` config; this slice only
/// proves both backends are independently valid REG-5 Module entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageBackend {
    Filesystem,
    S3,
}

// Every literal passed through the six helpers below is a fixed string
// written in this file, not runtime input: `ComponentId`/`CapabilityId`/
// `SchemaId`/`SchemaContractId`/`ActionId`/`RoutePath`/`ConfigDeclaration`
// only reject shapes (empty segments, reserved characters, leading slash,
// braces) that none of these literals contain. The workspace test in this
// PR (`reg5_registry_build.rs`) exercises every one of them, so a typo here
// fails CI immediately rather than surfacing as a runtime panic.

#[allow(clippy::expect_used)]
fn component(value: &str) -> ComponentId {
    ComponentId::new(value).expect("valid component id")
}

#[allow(clippy::expect_used)]
fn capability(value: &str) -> CapabilityId {
    CapabilityId::new(value).expect("valid capability id")
}

#[allow(clippy::expect_used)]
fn schema(value: &str) -> SchemaId {
    SchemaId::new(value).expect("valid schema id")
}

#[allow(clippy::expect_used)]
fn contract(value: &str) -> SchemaContractId {
    SchemaContractId::new(value).expect("valid schema contract id")
}

#[allow(clippy::expect_used)]
fn action(value: &str) -> ActionId {
    value.parse().expect("valid action id")
}

#[allow(clippy::expect_used)]
fn config(struct_name: &str, env_prefix: &str) -> ConfigDeclaration {
    ConfigDeclaration::new(struct_name, env_prefix, true).expect("valid config declaration")
}

#[allow(clippy::expect_used)]
fn route_path(value: &str) -> RoutePath {
    RoutePath::new(value).expect("valid route path")
}

/// Platform: health/ready/version, server meta, self-service UI state, and
/// the OpenAPI document endpoint. No declared actions: none of its routes
/// enforce an `Authorized<T>` capability gate.
fn platform_entry() -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component("platform"),
            kind: ComponentKind::PlatformService,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![],
        capabilities: Capabilities {
            provided: vec![],
            required_mandatory: vec![],
            required_optional: vec![],
        },
        api: Api {
            namespace: Some("platform".to_string()),
            routes: vec![
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/me/ui-state"),
                    operation_id: "get_ui_state".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Put,
                    path: route_path("/me/ui-state"),
                    operation_id: "set_ui_state".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/meta"),
                    operation_id: "meta".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/health"),
                    operation_id: "health".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/ready"),
                    operation_id: "ready".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/version"),
                    operation_id: "version".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                // `v2-e3-s4` D3: representable now that `RoutePath` accepts
                // one interior dot in the final segment. Mount location is
                // unaffected (`routes::acta::public::router`, unchanged) —
                // only the registry ownership entry follows
                // `docs/registry-route-ownership.md`'s existing conceptual
                // assignment to `platform`.
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/openapi.json"),
                    operation_id: "openapi_json".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/scalar"),
                    operation_id: "scalar".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
            ],
            dto_owner: Some(component("platform")),
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: false,
        },
        diagnostics: Diagnostics {
            health: true,
            readiness: true,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: Some(Persistence {
            schema: schema("platform"),
            migration_owner: component("platform"),
            schema_contracts_provided: vec![contract("platform.core")],
            schema_contracts_required: vec![],
        }),
        config: Some(config("PlatformConfig", "ATLAS_PLATFORM_")),
        workers: vec![],
        satellites: vec![],
    }
}

/// Custos: identity, auth, API keys, groups, and the authorization surface
/// (grants, audit). Depends on `platform`.
fn custos_entry() -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component("custos"),
            kind: ComponentKind::PlatformService,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![Dependency {
            component: component("platform"),
            min_contract: ContractVersion::new(1),
        }],
        capabilities: Capabilities {
            provided: vec![],
            required_mandatory: vec![],
            required_optional: vec![],
        },
        api: Api {
            namespace: Some("custos".to_string()),
            routes: vec![
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/auth/logout"),
                    operation_id: "logout".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/auth/me"),
                    operation_id: "me".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/auth/change-password"),
                    operation_id: "change_password".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/users/me"),
                    operation_id: "update_me".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users"),
                    operation_id: "create_user".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/users"),
                    operation_id: "list_users".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users/{user_id}/disable"),
                    operation_id: "disable_user".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users/{user_id}/enable"),
                    operation_id: "enable_user".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users/{user_id}/reset-password"),
                    operation_id: "reset_password".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users/{user_id}/activation-link"),
                    operation_id: "regenerate_activation_link".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/users/{user_id}/system-admin"),
                    operation_id: "set_system_admin".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/users/{user_id}/memberships"),
                    operation_id: "list_user_memberships".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/admin/audit"),
                    operation_id: "list_platform_audit".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/api-keys"),
                    operation_id: "create_user_api_key".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/api-keys"),
                    operation_id: "list_user_api_keys".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/api-keys/{key_id}"),
                    operation_id: "revoke_user_api_key".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/api-keys/{key_id}"),
                    operation_id: "update_user_api_key".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/api-keys/{key_id}/grants"),
                    operation_id: "list_api_key_grants".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/api-keys/{key_id}/grants/{grant_id}"),
                    operation_id: "delete_api_key_grant".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/grants"),
                    operation_id: "create_project_grant".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/grants"),
                    operation_id: "list_project_grants".to_string(),
                    action: Some(action("custos::grants::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}"),
                    operation_id: "delete_project_grant".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/grants"),
                    operation_id: "create_workspace_grant".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/grants"),
                    operation_id: "list_workspace_grants".to_string(),
                    action: Some(action("custos::grants::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/grants/{grant_id}"),
                    operation_id: "delete_workspace_grant".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/groups"),
                    operation_id: "create_group".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/groups"),
                    operation_id: "list_groups".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/groups/{group_id}"),
                    operation_id: "delete_group".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/groups/{group_id}/members"),
                    operation_id: "add_group_member".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/groups/{group_id}/members"),
                    operation_id: "list_group_members".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/groups/{group_id}/members/{user_id}"),
                    operation_id: "remove_group_member".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/audit"),
                    operation_id: "list_workspace_audit".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/auth/login"),
                    operation_id: "login".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/activate/{token}"),
                    operation_id: "get_activation_info".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/activate/{token}"),
                    operation_id: "post_activate".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
            ],
            dto_owner: Some(component("custos")),
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![action("custos::grants::read")],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: true,
        },
        diagnostics: Diagnostics {
            health: true,
            readiness: true,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: Some(Persistence {
            schema: schema("custos"),
            migration_owner: component("custos"),
            schema_contracts_provided: vec![contract("custos.principals")],
            schema_contracts_required: vec![contract("platform.core")],
        }),
        config: Some(config("CustosConfig", "ATLAS_CUSTOS_")),
        workers: vec![],
        satellites: vec![],
    }
}

/// Acta: workspaces, projects, documents, boards, tasks, search, webhooks,
/// automations, and every workspace-scoped surface (see the ownership
/// artifact for the full module list, including the A1-A8 resolutions and
/// the `workspaces`/`members` schema-ownership findings). Depends on
/// `platform` and `custos`; requires `storage.blob` mandatorily and
/// `search.semantic` optionally.
fn acta_entry() -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component("acta"),
            kind: ComponentKind::Product,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![
            Dependency {
                component: component("platform"),
                min_contract: ContractVersion::new(1),
            },
            Dependency {
                component: component("custos"),
                min_contract: ContractVersion::new(1),
            },
        ],
        capabilities: Capabilities {
            provided: vec![],
            required_mandatory: vec![capability("storage.blob")],
            required_optional: vec![capability("search.semantic")],
        },
        api: Api {
            namespace: Some("acta".to_string()),
            routes: vec![
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/admin/trash"),
                    operation_id: "list_trash".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/admin/trash/restore"),
                    operation_id: "restore_trash".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/admin/trash/purge"),
                    operation_id: "purge_trash".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/admin/trash/purges/{operation_id}"),
                    operation_id: "get_purge_status".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces"),
                    operation_id: "list_workspaces".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces"),
                    operation_id: "create_workspace".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}"),
                    operation_id: "get_workspace".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}"),
                    operation_id: "update_workspace".to_string(),
                    action: Some(action("acta::config::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/admin/workspaces"),
                    operation_id: "admin_list_workspaces".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/admin/workspaces/{ws}"),
                    operation_id: "admin_update_workspace".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/admin/workspaces/{ws}"),
                    operation_id: "admin_delete_workspace".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/admin/status-templates"),
                    operation_id: "list_platform_status_templates".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/admin/status-templates"),
                    operation_id: "create_platform_status_template".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/admin/status-templates/{template_id}"),
                    operation_id: "update_platform_status_template".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/admin/status-templates/{template_id}"),
                    operation_id: "delete_platform_status_template".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/projects"),
                    operation_id: "create_project".to_string(),
                    action: Some(action("acta::projects::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects"),
                    operation_id: "list_projects".to_string(),
                    action: Some(action("acta::projects::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}"),
                    operation_id: "get_project".to_string(),
                    action: Some(action("acta::projects::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}"),
                    operation_id: "update_project".to_string(),
                    action: Some(action("acta::projects::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}"),
                    operation_id: "delete_project".to_string(),
                    action: Some(action("acta::projects::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/members"),
                    operation_id: "list_workspace_members".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/members"),
                    operation_id: "add_member".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/assignable-users"),
                    operation_id: "list_assignable_users".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/members/{user_id}"),
                    operation_id: "update_member_role".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/members/{user_id}"),
                    operation_id: "remove_member".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tags"),
                    operation_id: "list_tags".to_string(),
                    action: Some(action("acta::config::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tags"),
                    operation_id: "create_tag".to_string(),
                    action: Some(action("acta::config::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tags/used"),
                    operation_id: "list_used_labels".to_string(),
                    action: Some(action("acta::config::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/tags/{tag_id}"),
                    operation_id: "patch_tag".to_string(),
                    action: Some(action("acta::config::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/tags/{tag_id}"),
                    operation_id: "delete_tag".to_string(),
                    action: Some(action("acta::config::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/status-templates"),
                    operation_id: "list_status_templates".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/status-templates"),
                    operation_id: "create_status_template".to_string(),
                    action: Some(action("acta::boards::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/status-templates/{template_id}"),
                    operation_id: "update_status_template".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/status-templates/{template_id}"),
                    operation_id: "delete_status_template".to_string(),
                    action: Some(action("acta::boards::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/apply-status-templates"),
                    operation_id: "apply_status_templates".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/property-definitions"),
                    operation_id: "list_property_definitions".to_string(),
                    action: Some(action("acta::config::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/property-definitions"),
                    operation_id: "create_property_definition".to_string(),
                    action: Some(action("acta::config::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/property-definitions/{property_definition_id}",
                    ),
                    operation_id: "delete_property_definition".to_string(),
                    action: Some(action("acta::config::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/saved-searches"),
                    operation_id: "list_saved_searches".to_string(),
                    action: Some(action("acta::saved_searches::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/saved-searches"),
                    operation_id: "create_saved_search".to_string(),
                    action: Some(action("acta::saved_searches::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/saved-searches/{id}"),
                    operation_id: "rename_saved_search".to_string(),
                    action: Some(action("acta::saved_searches::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/saved-searches/{id}"),
                    operation_id: "delete_saved_search".to_string(),
                    action: Some(action("acta::saved_searches::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/task-views"),
                    operation_id: "list_task_views".to_string(),
                    action: Some(action("acta::task_views::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/task-views"),
                    operation_id: "create_task_view".to_string(),
                    action: Some(action("acta::task_views::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/task-views/{id}"),
                    operation_id: "get_task_view".to_string(),
                    action: Some(action("acta::task_views::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/task-views/{id}"),
                    operation_id: "update_task_view".to_string(),
                    action: Some(action("acta::task_views::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/task-views/{id}"),
                    operation_id: "delete_task_view".to_string(),
                    action: Some(action("acta::task_views::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/boards"),
                    operation_id: "create_board".to_string(),
                    action: Some(action("acta::boards::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/boards"),
                    operation_id: "list_boards".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/boards/{board_id}"),
                    operation_id: "get_board".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/boards/{board_id}"),
                    operation_id: "update_board".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/boards/{board_id}"),
                    operation_id: "delete_board".to_string(),
                    action: Some(action("acta::boards::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/move"),
                    operation_id: "move_board".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/archive"),
                    operation_id: "archive_board".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/unarchive"),
                    operation_id: "unarchive_board".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/columns"),
                    operation_id: "create_column".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/columns"),
                    operation_id: "list_columns".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/columns/{column_id}"),
                    operation_id: "update_column".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/columns/{column_id}"),
                    operation_id: "delete_column".to_string(),
                    action: Some(action("acta::boards::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/tasks"),
                    operation_id: "create_task".to_string(),
                    action: Some(action("acta::tasks::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/tasks"),
                    operation_id: "list_tasks".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/presence"),
                    operation_id: "heartbeat".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/boards/{board_id}/presence"),
                    operation_id: "leave".to_string(),
                    action: Some(action("acta::boards::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/presence"),
                    operation_id: "document_heartbeat".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/documents/{slug}/presence"),
                    operation_id: "document_leave".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks"),
                    operation_id: "list_workspace_tasks".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}"),
                    operation_id: "get_task".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}"),
                    operation_id: "update_task".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}"),
                    operation_id: "delete_task".to_string(),
                    action: Some(action("acta::tasks::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/move"),
                    operation_id: "move_task".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/assignees"),
                    operation_id: "list_assignees".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/assignees"),
                    operation_id: "add_assignee".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/assignees/{assignee_ref}",
                    ),
                    operation_id: "remove_assignee".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/references"),
                    operation_id: "list_references".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/references"),
                    operation_id: "create_reference".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/references/batch"),
                    operation_id: "create_references_batch".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/references/{reference_id}",
                    ),
                    operation_id: "delete_reference".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/attachments"),
                    operation_id: "upload_task_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/attachments"),
                    operation_id: "list_task_attachments".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}/content",
                    ),
                    operation_id: "download_task_attachment".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
                    ),
                    operation_id: "rename_task_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/attachments/{attachment_id}",
                    ),
                    operation_id: "delete_task_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
                    ),
                    operation_id: "upload_task_comment_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments",
                    ),
                    operation_id: "list_task_comment_attachments".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/comment-drafts"),
                    operation_id: "create_task_comment_draft".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}",
                    ),
                    operation_id: "cancel_task_comment_draft".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comment-drafts/{draft_id}/attachments",
                    ),
                    operation_id: "upload_task_comment_draft_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}/content",
                    ),
                    operation_id: "download_task_comment_attachment".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}/attachments/{attachment_id}",
                    ),
                    operation_id: "delete_task_comment_attachment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/backlinks"),
                    operation_id: "list_task_backlinks".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/graph"),
                    operation_id: "get_task_graph".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/checklist"),
                    operation_id: "list_checklist".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/checklist"),
                    operation_id: "create_checklist_item".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"),
                    operation_id: "update_checklist_item".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}"),
                    operation_id: "delete_checklist_item".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path(
                        "/workspaces/{ws}/tasks/{readable_id}/checklist/{item_id}/promote",
                    ),
                    operation_id: "promote_checklist_item".to_string(),
                    action: Some(action("acta::tasks::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/subtasks"),
                    operation_id: "list_subtasks".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/subtasks"),
                    operation_id: "create_subtask".to_string(),
                    action: Some(action("acta::tasks::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/promote"),
                    operation_id: "promote_subtask".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/parent"),
                    operation_id: "set_task_parent".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/activity"),
                    operation_id: "list_activity".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/comments"),
                    operation_id: "list_comments".to_string(),
                    action: Some(action("acta::tasks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/comments"),
                    operation_id: "create_task_comment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}"),
                    operation_id: "update_comment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/tasks/{readable_id}/comments/{comment_id}"),
                    operation_id: "delete_comment".to_string(),
                    action: Some(action("acta::tasks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/activity"),
                    operation_id: "list_workspace_activity".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/documents"),
                    operation_id: "create_document".to_string(),
                    action: Some(action("acta::docs::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/documents"),
                    operation_id: "list_documents".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}"),
                    operation_id: "get_document".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/documents/{slug}"),
                    operation_id: "update_document".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/documents/{slug}"),
                    operation_id: "delete_document".to_string(),
                    action: Some(action("acta::docs::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Put,
                    path: route_path("/workspaces/{ws}/documents/{slug}/content"),
                    operation_id: "update_content".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/compact"),
                    operation_id: "get_document_compact".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/content/range"),
                    operation_id: "get_content_range".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/documents/{slug}/content/range"),
                    operation_id: "edit_content_range".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/content/search"),
                    operation_id: "search_content".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/history"),
                    operation_id: "list_history".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/revisions/{seq}"),
                    operation_id: "get_revision_content".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/backlinks"),
                    operation_id: "list_backlinks".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/frontmatter"),
                    operation_id: "get_frontmatter".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/attachments"),
                    operation_id: "upload_attachment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/attachments"),
                    operation_id: "list_attachments".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/attachments"),
                    operation_id: "list_workspace_attachments".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/attachments/{attachment_id}"),
                    operation_id: "download_attachment".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/attachments/{attachment_id}"),
                    operation_id: "rename_workspace_attachment".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/attachments/{attachment_id}"),
                    operation_id: "delete_attachment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path(
                        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
                    ),
                    operation_id: "upload_document_comment_attachment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path(
                        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments",
                    ),
                    operation_id: "list_document_comment_attachments".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comment-drafts"),
                    operation_id: "create_document_comment_draft".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}"),
                    operation_id: "cancel_document_comment_draft".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path(
                        "/workspaces/{ws}/documents/{slug}/comment-drafts/{draft_id}/attachments",
                    ),
                    operation_id: "upload_document_comment_draft_attachment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path(
                        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
                    ),
                    operation_id: "download_document_comment_attachment".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path(
                        "/workspaces/{ws}/documents/{slug}/comments/{comment_id}/attachments/{attachment_id}",
                    ),
                    operation_id: "delete_document_comment_attachment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/documents/{slug}/move"),
                    operation_id: "move_document".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/moves/batch"),
                    operation_id: "move_documents_batch".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/copy"),
                    operation_id: "copy_document".to_string(),
                    action: Some(action("acta::docs::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comments"),
                    operation_id: "list_document_comments".to_string(),
                    action: Some(action("acta::docs::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comments"),
                    operation_id: "create_document_comment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comments/{comment_id}"),
                    operation_id: "update_document_comment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/documents/{slug}/comments/{comment_id}"),
                    operation_id: "delete_document_comment".to_string(),
                    action: Some(action("acta::docs::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/folders"),
                    operation_id: "create_folder".to_string(),
                    action: Some(action("acta::folders::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/projects/{project_slug}/folders"),
                    operation_id: "list_folders".to_string(),
                    action: Some(action("acta::folders::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/folders/{folder_id}"),
                    operation_id: "get_folder".to_string(),
                    action: Some(action("acta::folders::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/folders/{folder_id}"),
                    operation_id: "rename_folder".to_string(),
                    action: Some(action("acta::folders::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/folders/{folder_id}"),
                    operation_id: "delete_folder".to_string(),
                    action: Some(action("acta::folders::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/folders/{folder_id}/move"),
                    operation_id: "move_folder".to_string(),
                    action: Some(action("acta::folders::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/folders/{folder_id}/copy"),
                    operation_id: "copy_folder".to_string(),
                    action: Some(action("acta::folders::create")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/webhooks"),
                    operation_id: "create_webhook".to_string(),
                    action: Some(action("acta::webhooks::create")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/webhooks"),
                    operation_id: "list_webhooks".to_string(),
                    action: Some(action("acta::webhooks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/webhooks/{webhook_id}"),
                    operation_id: "get_webhook".to_string(),
                    action: Some(action("acta::webhooks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/webhooks/{webhook_id}"),
                    operation_id: "update_webhook".to_string(),
                    action: Some(action("acta::webhooks::update")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/webhooks/{webhook_id}"),
                    operation_id: "delete_webhook".to_string(),
                    action: Some(action("acta::webhooks::delete")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/webhooks/{webhook_id}/deliveries"),
                    operation_id: "list_webhook_deliveries".to_string(),
                    action: Some(action("acta::webhooks::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/integration-configs"),
                    operation_id: "create_integration_config".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/integration-configs"),
                    operation_id: "list_integration_configs".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/integration-configs/{config_id}"),
                    operation_id: "get_integration_config".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/integration-configs/{config_id}"),
                    operation_id: "patch_integration_config".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/integration-configs/{config_id}"),
                    operation_id: "delete_integration_config".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/automation-rules"),
                    operation_id: "create_automation_rule".to_string(),
                    action: None,
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/automation-rules"),
                    operation_id: "list_automation_rules".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/automation-rules/{rule_id}"),
                    operation_id: "get_automation_rule".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Patch,
                    path: route_path("/workspaces/{ws}/automation-rules/{rule_id}"),
                    operation_id: "patch_automation_rule".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Delete,
                    path: route_path("/workspaces/{ws}/automation-rules/{rule_id}"),
                    operation_id: "delete_automation_rule".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/events"),
                    operation_id: "stream_events".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/search"),
                    operation_id: "search".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/semantic-search"),
                    operation_id: "semantic_search".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Get,
                    path: route_path("/workspaces/{ws}/semantic-search/reindex"),
                    operation_id: "semantic_reindex_plan".to_string(),
                    action: Some(action("acta::config::read")),
                    idempotent: false,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/semantic-search/reindex"),
                    operation_id: "semantic_reindex_start".to_string(),
                    action: Some(action("acta::config::update")),
                    idempotent: true,
                    is_public: false,
                },
                RouteDeclaration {
                    method: HttpMethod::Post,
                    path: route_path("/workspaces/{ws}/integrations/{integration}/events"),
                    operation_id: "ingest_github_event".to_string(),
                    action: None,
                    idempotent: false,
                    is_public: true,
                },
            ],
            dto_owner: Some(component("acta")),
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![
                action("acta::boards::create"),
                action("acta::boards::delete"),
                action("acta::boards::read"),
                action("acta::boards::update"),
                action("acta::config::create"),
                action("acta::config::delete"),
                action("acta::config::read"),
                action("acta::config::update"),
                action("acta::docs::create"),
                action("acta::docs::delete"),
                action("acta::docs::read"),
                action("acta::docs::update"),
                action("acta::folders::create"),
                action("acta::folders::delete"),
                action("acta::folders::read"),
                action("acta::folders::update"),
                action("acta::projects::create"),
                action("acta::projects::delete"),
                action("acta::projects::read"),
                action("acta::projects::update"),
                action("acta::saved_searches::create"),
                action("acta::saved_searches::delete"),
                action("acta::saved_searches::read"),
                action("acta::saved_searches::update"),
                action("acta::task_views::create"),
                action("acta::task_views::delete"),
                action("acta::task_views::read"),
                action("acta::task_views::update"),
                action("acta::tasks::create"),
                action("acta::tasks::delete"),
                action("acta::tasks::read"),
                action("acta::tasks::update"),
                action("acta::webhooks::create"),
                action("acta::webhooks::delete"),
                action("acta::webhooks::read"),
                action("acta::webhooks::update"),
            ],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: true,
        },
        diagnostics: Diagnostics {
            health: true,
            readiness: true,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: Some(Persistence {
            schema: schema("acta"),
            migration_owner: component("acta"),
            schema_contracts_provided: vec![],
            schema_contracts_required: vec![
                contract("platform.core"),
                contract("custos.principals"),
            ],
        }),
        config: Some(config("ActaConfig", "ATLAS_ACTA_")),
        workers: vec![],
        satellites: vec![],
    }
}

/// `storage.filesystem`: mandatory `storage.blob` provider, local-disk
/// backend. No HTTP surface (Module, SHELL-REG-5).
fn storage_filesystem_entry() -> ComponentEntry {
    minimal_module("storage.filesystem", vec![capability("storage.blob")])
}

/// `storage.s3`: mandatory `storage.blob` provider, S3-compatible backend.
/// No HTTP surface (Module, SHELL-REG-5). Mutually exclusive with
/// `storage.filesystem` in a single `build()` call; see `StorageBackend`.
fn storage_s3_entry() -> ComponentEntry {
    minimal_module("storage.s3", vec![capability("storage.blob")])
}

/// `search.postgres_fts`: lexical search provider. No HTTP surface (Module,
/// SHELL-REG-5).
fn search_postgres_fts_entry() -> ComponentEntry {
    minimal_module("search.postgres_fts", vec![capability("search.lexical")])
}

/// `search.pgvector_embeddings`: optional-capability semantic search
/// provider (spec scenario "search.pgvector_embeddings is optional"). No
/// HTTP surface (Module, SHELL-REG-5).
fn search_pgvector_embeddings_entry() -> ComponentEntry {
    minimal_module(
        "search.pgvector_embeddings",
        vec![capability("search.semantic")],
    )
}

/// Shared shape for a capability/persistence-only Module: no HTTP surface,
/// no persistence, no config, no authorization (mirrors `entry.rs`'s own
/// `minimal_module_entry()` fixture, which this spec's Module requirement is
/// grounded in).
fn minimal_module(stable_id: &str, provided: Vec<CapabilityId>) -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component(stable_id),
            kind: ComponentKind::Module,
            contract_version: ContractVersion::new(1),
        },
        dependencies: vec![],
        capabilities: Capabilities {
            provided,
            required_mandatory: vec![],
            required_optional: vec![],
        },
        api: Api {
            namespace: None,
            routes: vec![],
            dto_owner: None,
        },
        authorization: Authorization {
            resource_kinds: vec![],
            actions: vec![],
            role_definitions: vec![],
            principal_sets: vec![],
            provider: false,
        },
        diagnostics: Diagnostics {
            health: false,
            readiness: false,
            doctor: false,
        },
        experience: Experience {
            navigation_providers: vec![],
            context_providers: vec![],
        },
        persistence: None,
        config: None,
        workers: vec![],
        satellites: vec![],
    }
}

/// Builds the REG-5 component entry set (SHELL-REG-5): `platform`, `custos`,
/// `acta`, and the storage/search Modules. Used by the workspace build test
/// (this PR) and by `atlas_server` startup validation (PR2) — both call this
/// one constructor so the entries a passing test proves and the entries the
/// server boots with never drift apart.
pub fn reg5_component_entries(storage_backend: StorageBackend) -> Vec<ComponentEntry> {
    let storage_entry = match storage_backend {
        StorageBackend::Filesystem => storage_filesystem_entry(),
        StorageBackend::S3 => storage_s3_entry(),
    };

    vec![
        platform_entry(),
        custos_entry(),
        acta_entry(),
        storage_entry,
        search_postgres_fts_entry(),
        search_pgvector_embeddings_entry(),
    ]
}

/// Resolves the active `StorageBackend` from `ATLAS_ATTACHMENT_BACKEND`, the
/// same environment variable `state::build_attachment_store` reads to
/// construct the live attachment store, so the startup registry gate and the
/// attachment store always agree on which backend is active.
pub fn storage_backend_from_env() -> Result<StorageBackend, String> {
    let value = crate::config::env_var_nonempty("ATLAS_ATTACHMENT_BACKEND");
    storage_backend_from_value(value.as_deref())
}

/// Pure mapping from an `ATLAS_ATTACHMENT_BACKEND` value to a
/// `StorageBackend`, split out from `storage_backend_from_env` so the
/// mapping itself is testable without mutating process-global environment
/// state. `None` (the variable unset) defaults to `disk`, mirroring
/// `state::build_attachment_store`'s own default.
fn storage_backend_from_value(value: Option<&str>) -> Result<StorageBackend, String> {
    match value.unwrap_or("disk") {
        "disk" => Ok(StorageBackend::Filesystem),
        "s3" => Ok(StorageBackend::S3),
        other => Err(format!(
            "unknown ATLAS_ATTACHMENT_BACKEND '{other}'; expected 'disk' or 's3'"
        )),
    }
}

#[cfg(test)]
mod storage_backend_from_value_tests {
    use super::*;

    #[test]
    fn defaults_to_filesystem_when_unset() {
        assert_eq!(
            storage_backend_from_value(None),
            Ok(StorageBackend::Filesystem)
        );
    }

    #[test]
    fn maps_disk_to_filesystem() {
        assert_eq!(
            storage_backend_from_value(Some("disk")),
            Ok(StorageBackend::Filesystem)
        );
    }

    #[test]
    fn maps_s3_to_s3() {
        assert_eq!(
            storage_backend_from_value(Some("s3")),
            Ok(StorageBackend::S3)
        );
    }

    #[test]
    fn rejects_an_unrecognised_value_without_defaulting_silently() {
        let error =
            storage_backend_from_value(Some("azure")).expect_err("azure is not a known backend");
        assert!(error.contains("azure"));
    }
}
