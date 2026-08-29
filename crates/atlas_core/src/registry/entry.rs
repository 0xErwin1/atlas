use super::{
    Authorization, CapabilityId, ComponentId, ComponentKind, ConfigDeclaration, ContractVersion,
    Persistence, RouteDeclaration, SatelliteDeclaration,
};

/// Stable identity of a component within the registry (SHELL-REG-1).
pub struct Identity {
    pub stable_id: ComponentId,
    pub kind: ComponentKind,
    pub contract_version: ContractVersion,
}

/// A dependency on another component's contract (SHELL-REG-1).
pub struct Dependency {
    pub component: ComponentId,
    pub min_contract: ContractVersion,
}

/// Capabilities provided and required by a component (SHELL-REG-1).
pub struct Capabilities {
    pub provided: Vec<CapabilityId>,
    pub required_mandatory: Vec<CapabilityId>,
    pub required_optional: Vec<CapabilityId>,
}

/// A component's HTTP API surface (SHELL-REG-1, SHELL-REG-5). Both
/// `namespace` and `dto_owner` are `None` for a component with no HTTP
/// surface, such as a Module. `namespace` is the URL namespace segment
/// (e.g. `"acta"`), not a `ComponentId`, because the mount point is a URL
/// token.
pub struct Api {
    pub namespace: Option<String>,
    pub routes: Vec<RouteDeclaration>,
    pub dto_owner: Option<ComponentId>,
}

/// Diagnostics endpoints declared by a component (SHELL-REG-1).
pub struct Diagnostics {
    pub health: bool,
    pub readiness: bool,
    pub doctor: bool,
}

/// Experience integration points declared by a component (SHELL-REG-1).
pub struct Experience {
    pub navigation_providers: Vec<String>,
    pub context_providers: Vec<String>,
}

/// A background worker declared by a component. Start/drain ordering derives
/// from `ComponentEntry.dependencies`, not from per-worker fields.
pub struct WorkerDeclaration {
    pub name: String,
}

/// The complete registry entry for one component (SHELL-REG-1, SHELL-INT-3).
pub struct ComponentEntry {
    pub identity: Identity,
    pub dependencies: Vec<Dependency>,
    pub capabilities: Capabilities,
    pub api: Api,
    pub authorization: Authorization,
    pub diagnostics: Diagnostics,
    pub experience: Experience,
    pub persistence: Option<Persistence>,
    pub config: Option<ConfigDeclaration>,
    pub workers: Vec<WorkerDeclaration>,
    pub satellites: Vec<SatelliteDeclaration>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ActionId;
    use crate::registry::{
        ContractVersionRange, HttpMethod, RouteDeclaration, RoutePath, SatelliteMode, SchemaId,
    };

    fn full_entry() -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: ComponentId::new("acta").expect("valid component id"),
                kind: ComponentKind::Product,
                contract_version: ContractVersion::new(1),
            },
            dependencies: vec![Dependency {
                component: ComponentId::new("platform").expect("valid component id"),
                min_contract: ContractVersion::new(1),
            }],
            capabilities: Capabilities {
                provided: vec![CapabilityId::new("acta.tasks").expect("valid capability id")],
                required_mandatory: vec![
                    CapabilityId::new("storage.blob").expect("valid capability id"),
                ],
                required_optional: vec![],
            },
            api: Api {
                namespace: Some("acta".to_string()),
                routes: vec![RouteDeclaration {
                    method: HttpMethod::Get,
                    path: RoutePath::new("/tasks/{task_id}").expect("valid route path"),
                    operation_id: "getTask".to_string(),
                    action: Some(
                        "acta::task::read"
                            .parse::<ActionId>()
                            .expect("valid action id"),
                    ),
                    idempotent: false,
                }],
                dto_owner: Some(ComponentId::new("acta").expect("valid component id")),
            },
            authorization: Authorization {
                resource_kinds: vec!["task".to_string()],
                actions: vec![
                    "acta::task::read"
                        .parse::<ActionId>()
                        .expect("valid action id"),
                ],
                role_definitions: vec!["acta.admin".to_string()],
                principal_sets: vec!["acta.members".to_string()],
                provider: true,
            },
            diagnostics: Diagnostics {
                health: true,
                readiness: true,
                doctor: false,
            },
            experience: Experience {
                navigation_providers: vec!["acta.nav".to_string()],
                context_providers: vec!["acta.context".to_string()],
            },
            persistence: Some(Persistence {
                schema: SchemaId::new("acta").expect("valid schema id"),
                migration_owner: ComponentId::new("acta").expect("valid component id"),
                schema_contracts_provided: vec![],
                schema_contracts_required: vec![],
            }),
            config: Some(
                ConfigDeclaration::new("ActaConfig", "ATLAS_ACTA_", true)
                    .expect("valid config declaration"),
            ),
            workers: vec![WorkerDeclaration {
                name: "acta.reindex".to_string(),
            }],
            satellites: vec![
                SatelliteDeclaration::new(
                    ComponentId::new("acta").expect("valid component id"),
                    vec![CapabilityId::new("acta.tasks").expect("valid capability id")],
                    ContractVersion::new(1),
                    ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(1))
                        .expect("valid range"),
                    SatelliteMode::Local,
                    "handshake-v1",
                    "health-v1",
                )
                .expect("valid satellite declaration"),
            ],
        }
    }

    fn minimal_module_entry() -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: ComponentId::new("storage.filesystem").expect("valid component id"),
                kind: ComponentKind::Module,
                contract_version: ContractVersion::new(1),
            },
            dependencies: vec![],
            capabilities: Capabilities {
                provided: vec![],
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

    #[test]
    fn full_entry_constructs_and_reads_back_every_field() {
        let entry = full_entry();

        assert_eq!(entry.identity.stable_id.as_str(), "acta");
        assert_eq!(entry.identity.kind, ComponentKind::Product);
        assert_eq!(entry.identity.contract_version, ContractVersion::new(1));
        assert_eq!(entry.dependencies.len(), 1);
        assert_eq!(entry.dependencies[0].component.as_str(), "platform");
        assert_eq!(entry.capabilities.provided.len(), 1);
        assert_eq!(entry.capabilities.required_mandatory.len(), 1);
        assert!(entry.capabilities.required_optional.is_empty());
        assert_eq!(entry.api.namespace.as_deref(), Some("acta"));
        assert_eq!(entry.api.routes.len(), 1);
        assert_eq!(entry.api.routes[0].operation_id, "getTask");
        assert_eq!(
            entry.api.dto_owner.as_ref().map(ComponentId::as_str),
            Some("acta")
        );
        assert_eq!(entry.authorization.resource_kinds, vec!["task".to_string()]);
        assert!(entry.authorization.provider);
        assert!(entry.diagnostics.health);
        assert_eq!(
            entry.experience.navigation_providers,
            vec!["acta.nav".to_string()]
        );
        assert!(entry.persistence.is_some());
        assert!(entry.config.is_some());
        assert_eq!(entry.workers.len(), 1);
        assert_eq!(entry.satellites.len(), 1);
    }

    #[test]
    fn minimal_module_entry_has_no_api_persistence_or_config() {
        let entry = minimal_module_entry();

        assert_eq!(entry.identity.kind, ComponentKind::Module);
        assert!(entry.api.namespace.is_none());
        assert!(entry.api.dto_owner.is_none());
        assert!(entry.api.routes.is_empty());
        assert!(entry.persistence.is_none());
        assert!(entry.config.is_none());
        assert!(entry.workers.is_empty());
        assert!(entry.satellites.is_empty());
    }

    /// Maps every section-5 validation matrix row (D2) to the named
    /// `ComponentEntry` field it will read, proving no shape change is
    /// needed to implement `registry::build()`.
    #[test]
    fn traceability_maps_every_matrix_row_to_a_named_field() {
        let entry = full_entry();

        // Row: stable_id / namespace / paths / ActionId unique.
        let _stable_id = &entry.identity.stable_id;
        let _namespace = &entry.api.namespace;
        let _route_paths: Vec<&RoutePath> =
            entry.api.routes.iter().map(|route| &route.path).collect();
        let _actions = &entry.authorization.actions;

        // Row: dependencies exist, no cycles, min_contract.
        let _dependency_components: Vec<&ComponentId> = entry
            .dependencies
            .iter()
            .map(|dependency| &dependency.component)
            .collect();
        let _dependency_min_contracts: Vec<ContractVersion> = entry
            .dependencies
            .iter()
            .map(|dependency| dependency.min_contract)
            .collect();
        let _entry_contract_version = entry.identity.contract_version;

        // Row: capability mandatory/optional provider counts.
        let _required_mandatory = &entry.capabilities.required_mandatory;
        let _required_optional = &entry.capabilities.required_optional;

        // Row: schema owner, migration owner, config struct.
        let _schema = entry
            .persistence
            .as_ref()
            .map(|persistence| &persistence.schema);
        let _migration_owner = entry
            .persistence
            .as_ref()
            .map(|persistence| &persistence.migration_owner);
        let _config = &entry.config;

        // Row: migration order vs schema contracts.
        let _schema_contracts_required = entry
            .persistence
            .as_ref()
            .map(|persistence| &persistence.schema_contracts_required);

        // Row: config mandatory present.
        let _config_mandatory = entry.config.as_ref().map(ConfigDeclaration::mandatory);
        let _config_env_prefix = entry.config.as_ref().map(ConfigDeclaration::env_prefix);

        // Row: satellite owner/protocol/range/negotiation.
        let _satellite_owners: Vec<&ComponentId> = entry
            .satellites
            .iter()
            .map(SatelliteDeclaration::owner)
            .collect();
        let _satellite_protocols: Vec<ContractVersion> = entry
            .satellites
            .iter()
            .map(SatelliteDeclaration::protocol_version)
            .collect();
        let _satellite_ranges: Vec<ContractVersionRange> = entry
            .satellites
            .iter()
            .map(SatelliteDeclaration::compatible_range)
            .collect();
        let _satellite_negotiations: Vec<&str> = entry
            .satellites
            .iter()
            .map(SatelliteDeclaration::negotiation)
            .collect();

        // Row: router = registry.
        let _route_declarations = &entry.api.routes;

        // Row: CLI/MCP cover OpenAPI.
        let _operation_ids: Vec<&str> = entry
            .api
            .routes
            .iter()
            .map(|route| route.operation_id.as_str())
            .collect();

        // Row: absent product has no surface — asserted structurally by the
        // absence of an entry in the registry vec (D2), not a D1 field.
        assert!(entry.identity.stable_id.as_str() == "acta");
    }
}
