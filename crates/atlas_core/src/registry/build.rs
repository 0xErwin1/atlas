use std::collections::{BTreeMap, BTreeSet};

use super::graph::topological_order;
use super::{
    CapabilityId, ComponentEntry, ComponentId, HttpMethod, Registry, RegistryBuildError, RoutePath,
    SchemaContractId, SchemaId,
};
use crate::ids::ActionId;

/// Builds a validated `Registry` from the full entry slice, or every matrix
/// violation found (SHELL-REG-3). Pure: runs all six validators over the
/// same input and never stops at the first violation.
pub fn build(entries: Vec<ComponentEntry>) -> Result<Registry, Vec<RegistryBuildError>> {
    let index = ComponentIndex::build(&entries);

    let mut errors = Vec::new();
    errors.extend(validate_unique_ids(&entries));
    errors.extend(validate_dependencies(&entries, &index));
    errors.extend(validate_capabilities(&entries));
    errors.extend(validate_persistence(&entries, &index));

    let migration_order = match validate_migration_order(&entries) {
        Ok(order) => order,
        Err(migration_errors) => {
            errors.extend(migration_errors);
            Vec::new()
        }
    };

    errors.extend(validate_satellites(&entries, &index));

    if !errors.is_empty() {
        return Err(errors);
    }

    Ok(Registry::new(entries, index.into_inner(), migration_order))
}

/// A lookup from `ComponentId` to its position in the entry slice. The first
/// entry with a given `stable_id` wins on duplicates; `validate_unique_ids`
/// reports the duplicate separately.
pub(crate) struct ComponentIndex(BTreeMap<ComponentId, usize>);

impl ComponentIndex {
    pub(crate) fn build(entries: &[ComponentEntry]) -> Self {
        let mut map = BTreeMap::new();

        for (position, entry) in entries.iter().enumerate() {
            map.entry(entry.identity.stable_id.clone())
                .or_insert(position);
        }

        Self(map)
    }

    pub(crate) fn contains(&self, id: &ComponentId) -> bool {
        self.0.contains_key(id)
    }

    pub(crate) fn position(&self, id: &ComponentId) -> Option<usize> {
        self.0.get(id).copied()
    }

    pub(crate) fn into_inner(self) -> BTreeMap<ComponentId, usize> {
        self.0
    }
}

/// Rejects duplicate `stable_id`, duplicate `api.namespace`, duplicate
/// `(namespace, method, path)` route triples, and duplicate `ActionId`
/// across all entries.
fn validate_unique_ids(entries: &[ComponentEntry]) -> Vec<RegistryBuildError> {
    let mut errors = Vec::new();

    let mut stable_ids: BTreeMap<ComponentId, usize> = BTreeMap::new();
    for entry in entries {
        *stable_ids
            .entry(entry.identity.stable_id.clone())
            .or_insert(0) += 1;
    }
    for (id, count) in &stable_ids {
        if *count > 1 {
            errors.push(RegistryBuildError::DuplicateStableId {
                component: id.clone(),
            });
        }
    }

    let mut namespaces: BTreeMap<String, Vec<ComponentId>> = BTreeMap::new();
    for entry in entries {
        if let Some(namespace) = &entry.api.namespace {
            namespaces
                .entry(namespace.clone())
                .or_default()
                .push(entry.identity.stable_id.clone());
        }
    }
    for (namespace, components) in &namespaces {
        if components.len() > 1 {
            errors.push(RegistryBuildError::DuplicateNamespace {
                namespace: namespace.clone(),
                components: components.clone(),
            });
        }
    }

    let mut routes: BTreeMap<(Option<String>, HttpMethod, RoutePath), Vec<ComponentId>> =
        BTreeMap::new();
    for entry in entries {
        for route in &entry.api.routes {
            routes
                .entry((
                    entry.api.namespace.clone(),
                    route.method,
                    route.path.clone(),
                ))
                .or_default()
                .push(entry.identity.stable_id.clone());
        }
    }
    for ((namespace, method, path), components) in &routes {
        if components.len() > 1 {
            errors.push(RegistryBuildError::DuplicateRoute {
                namespace: namespace.clone(),
                method: *method,
                path: path.clone(),
                components: components.clone(),
            });
        }
    }

    let mut actions: BTreeMap<ActionId, Vec<ComponentId>> = BTreeMap::new();
    for entry in entries {
        for action in &entry.authorization.actions {
            actions
                .entry(action.clone())
                .or_default()
                .push(entry.identity.stable_id.clone());
        }
    }
    for (action, components) in &actions {
        if components.len() > 1 {
            errors.push(RegistryBuildError::DuplicateAction {
                action: action.clone(),
                components: components.clone(),
            });
        }
    }

    errors
}

/// Rejects a dependency on an absent component, a dependency cycle, and a
/// dependency whose target's `contract_version` is below the declared
/// `min_contract`.
fn validate_dependencies(
    entries: &[ComponentEntry],
    index: &ComponentIndex,
) -> Vec<RegistryBuildError> {
    let mut errors = Vec::new();

    for entry in entries {
        for dependency in &entry.dependencies {
            match index.position(&dependency.component) {
                None => {
                    errors.push(RegistryBuildError::UnknownDependency {
                        component: entry.identity.stable_id.clone(),
                        dependency: dependency.component.clone(),
                    });
                }
                Some(target_position) => {
                    let Some(target) = entries.get(target_position) else {
                        continue;
                    };

                    if target.identity.contract_version < dependency.min_contract {
                        errors.push(RegistryBuildError::MinContractNotSatisfied {
                            component: entry.identity.stable_id.clone(),
                            dependency: dependency.component.clone(),
                            required: dependency.min_contract,
                            actual: target.identity.contract_version,
                        });
                    }
                }
            }
        }
    }

    let nodes: BTreeSet<ComponentId> = entries
        .iter()
        .map(|entry| entry.identity.stable_id.clone())
        .collect();

    let mut edges: BTreeMap<ComponentId, BTreeSet<ComponentId>> = BTreeMap::new();
    for entry in entries {
        for dependency in &entry.dependencies {
            if index.contains(&dependency.component) {
                edges
                    .entry(dependency.component.clone())
                    .or_default()
                    .insert(entry.identity.stable_id.clone());
            }
        }
    }

    if let Err(chain) = topological_order(&nodes, &edges) {
        errors.push(RegistryBuildError::DependencyCycle { chain });
    }

    errors
}

/// Resolves each `required_mandatory` capability to exactly one provider and
/// each `required_optional` capability to zero or one provider (SHELL-CAP-2).
/// Ambiguity is reported only for capabilities some entry actually requires
/// (D-6), once per capability regardless of how many entries require it.
fn validate_capabilities(entries: &[ComponentEntry]) -> Vec<RegistryBuildError> {
    let mut errors = Vec::new();

    let mut providers: BTreeMap<CapabilityId, BTreeSet<ComponentId>> = BTreeMap::new();
    for entry in entries {
        for capability in &entry.capabilities.provided {
            providers
                .entry(capability.clone())
                .or_default()
                .insert(entry.identity.stable_id.clone());
        }
    }

    let mut ambiguous_mandatory: BTreeSet<CapabilityId> = BTreeSet::new();
    let mut ambiguous_optional: BTreeSet<CapabilityId> = BTreeSet::new();

    for entry in entries {
        for capability in &entry.capabilities.required_mandatory {
            let provider_count = providers.get(capability).map_or(0, BTreeSet::len);

            match provider_count {
                0 => errors.push(RegistryBuildError::MandatoryCapabilityUnprovided {
                    capability: capability.clone(),
                    component: entry.identity.stable_id.clone(),
                }),
                1 => {}
                _ => {
                    ambiguous_mandatory.insert(capability.clone());
                }
            }
        }

        for capability in &entry.capabilities.required_optional {
            if providers.get(capability).map_or(0, BTreeSet::len) > 1 {
                ambiguous_optional.insert(capability.clone());
            }
        }
    }

    for capability in &ambiguous_mandatory {
        let provider_set = providers.get(capability).cloned().unwrap_or_default();
        errors.push(RegistryBuildError::MandatoryCapabilityAmbiguous {
            capability: capability.clone(),
            providers: provider_set.into_iter().collect(),
        });
    }

    for capability in &ambiguous_optional {
        let provider_set = providers.get(capability).cloned().unwrap_or_default();
        errors.push(RegistryBuildError::OptionalCapabilityAmbiguous {
            capability: capability.clone(),
            providers: provider_set.into_iter().collect(),
        });
    }

    errors
}

/// Rejects a persistent entry with no `config`, a duplicate `Persistence.schema`
/// owner, and a `migration_owner` that names no entry.
fn validate_persistence(
    entries: &[ComponentEntry],
    index: &ComponentIndex,
) -> Vec<RegistryBuildError> {
    let mut errors = Vec::new();

    for entry in entries {
        let Some(persistence) = &entry.persistence else {
            continue;
        };

        if !index.contains(&persistence.migration_owner) {
            errors.push(RegistryBuildError::UnknownMigrationOwner {
                component: entry.identity.stable_id.clone(),
                migration_owner: persistence.migration_owner.clone(),
            });
        }

        if entry.config.is_none() {
            errors.push(RegistryBuildError::PersistenceWithoutConfig {
                component: entry.identity.stable_id.clone(),
            });
        }
    }

    let mut schema_owners: BTreeMap<SchemaId, Vec<ComponentId>> = BTreeMap::new();
    for entry in entries {
        if let Some(persistence) = &entry.persistence {
            schema_owners
                .entry(persistence.schema.clone())
                .or_default()
                .push(entry.identity.stable_id.clone());
        }
    }
    for (schema, components) in &schema_owners {
        if components.len() > 1 {
            errors.push(RegistryBuildError::DuplicateSchemaOwner {
                schema: schema.clone(),
                components: components.clone(),
            });
        }
    }

    errors
}

/// Orders persistent entries so every provider of a `SchemaContractId`
/// precedes every entry requiring it. Rejects a required contract with no
/// provider and a contract cycle.
fn validate_migration_order(
    entries: &[ComponentEntry],
) -> Result<Vec<ComponentId>, Vec<RegistryBuildError>> {
    let nodes: BTreeSet<ComponentId> = entries
        .iter()
        .filter(|entry| entry.persistence.is_some())
        .map(|entry| entry.identity.stable_id.clone())
        .collect();

    let mut providers: BTreeMap<SchemaContractId, BTreeSet<ComponentId>> = BTreeMap::new();
    for entry in entries {
        if let Some(persistence) = &entry.persistence {
            for contract in &persistence.schema_contracts_provided {
                providers
                    .entry(contract.clone())
                    .or_default()
                    .insert(entry.identity.stable_id.clone());
            }
        }
    }

    let mut errors = Vec::new();
    let mut edges: BTreeMap<ComponentId, BTreeSet<ComponentId>> = BTreeMap::new();

    for entry in entries {
        let Some(persistence) = &entry.persistence else {
            continue;
        };

        for contract in &persistence.schema_contracts_required {
            match providers.get(contract) {
                None => errors.push(RegistryBuildError::UnprovidedSchemaContract {
                    component: entry.identity.stable_id.clone(),
                    contract: contract.clone(),
                }),
                Some(provider_set) => {
                    for provider in provider_set {
                        edges
                            .entry(provider.clone())
                            .or_default()
                            .insert(entry.identity.stable_id.clone());
                    }
                }
            }
        }
    }

    match topological_order(&nodes, &edges) {
        Ok(order) => {
            if errors.is_empty() {
                Ok(order)
            } else {
                Err(errors)
            }
        }
        Err(chain) => {
            errors.push(RegistryBuildError::SchemaContractCycle { chain });
            Err(errors)
        }
    }
}

/// Rejects a satellite whose `owner()` names no entry and a satellite whose
/// `protocol_version()` falls outside its own `compatible_range()`.
fn validate_satellites(
    entries: &[ComponentEntry],
    index: &ComponentIndex,
) -> Vec<RegistryBuildError> {
    let mut errors = Vec::new();

    for entry in entries {
        for satellite in &entry.satellites {
            if !index.contains(satellite.owner()) {
                errors.push(RegistryBuildError::UnknownSatelliteOwner {
                    component: entry.identity.stable_id.clone(),
                    owner: satellite.owner().clone(),
                });
            }

            if !satellite
                .compatible_range()
                .contains(satellite.protocol_version())
            {
                errors.push(RegistryBuildError::SatelliteProtocolOutOfRange {
                    component: entry.identity.stable_id.clone(),
                    protocol_version: satellite.protocol_version(),
                    compatible_range: satellite.compatible_range(),
                });
            }
        }
    }

    errors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{
        Api, Authorization, Capabilities, ComponentKind, ContractVersion, ContractVersionRange,
        Dependency, Diagnostics, Experience, Identity, Persistence, RouteDeclaration,
        SatelliteDeclaration, SatelliteMode,
    };

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn capability(value: &str) -> CapabilityId {
        CapabilityId::new(value).expect("valid capability id")
    }

    fn schema(value: &str) -> SchemaId {
        SchemaId::new(value).expect("valid schema id")
    }

    fn contract(value: &str) -> SchemaContractId {
        SchemaContractId::new(value).expect("valid schema contract id")
    }

    fn action(value: &str) -> ActionId {
        value.parse().expect("valid action id")
    }

    /// Minimal `ComponentEntry` with no dependencies, capabilities, API
    /// surface, persistence, config, workers, or satellites. Tests mutate
    /// the fields they need.
    fn base_entry(stable_id: &str) -> ComponentEntry {
        ComponentEntry {
            identity: Identity {
                stable_id: component(stable_id),
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

    fn with_route(mut entry: ComponentEntry, method: HttpMethod, path: &str) -> ComponentEntry {
        entry.api.routes.push(RouteDeclaration {
            method,
            path: RoutePath::new(path).expect("valid route path"),
            operation_id: "op".to_string(),
            action: None,
            idempotent: false,
            is_public: false,
        });
        entry
    }

    fn with_namespace(mut entry: ComponentEntry, namespace: &str) -> ComponentEntry {
        entry.api.namespace = Some(namespace.to_string());
        entry
    }

    fn with_action(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry.authorization.actions.push(action(value));
        entry
    }

    fn with_dependency(
        mut entry: ComponentEntry,
        target: &str,
        min_contract: u32,
    ) -> ComponentEntry {
        entry.dependencies.push(Dependency {
            component: component(target),
            min_contract: ContractVersion::new(min_contract),
        });
        entry
    }

    fn with_provided_capability(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry.capabilities.provided.push(capability(value));
        entry
    }

    fn with_mandatory_capability(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry
            .capabilities
            .required_mandatory
            .push(capability(value));
        entry
    }

    fn with_optional_capability(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry.capabilities.required_optional.push(capability(value));
        entry
    }

    fn with_config(mut entry: ComponentEntry) -> ComponentEntry {
        entry.config = Some(
            crate::registry::ConfigDeclaration::new("Config", "ATLAS_TEST_", true)
                .expect("valid config declaration"),
        );
        entry
    }

    fn with_persistence(
        mut entry: ComponentEntry,
        schema_value: &str,
        migration_owner: &str,
    ) -> ComponentEntry {
        entry.persistence = Some(Persistence {
            schema: schema(schema_value),
            migration_owner: component(migration_owner),
            schema_contracts_provided: vec![],
            schema_contracts_required: vec![],
        });
        entry
    }

    fn with_provided_contract(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry
            .persistence
            .as_mut()
            .expect("persistence set before adding a provided contract")
            .schema_contracts_provided
            .push(contract(value));
        entry
    }

    fn with_required_contract(mut entry: ComponentEntry, value: &str) -> ComponentEntry {
        entry
            .persistence
            .as_mut()
            .expect("persistence set before adding a required contract")
            .schema_contracts_required
            .push(contract(value));
        entry
    }

    fn with_satellite(
        mut entry: ComponentEntry,
        owner: &str,
        protocol_version: u32,
        range_min: u32,
        range_max: u32,
    ) -> ComponentEntry {
        entry.satellites.push(
            SatelliteDeclaration::new(
                component(owner),
                vec![],
                ContractVersion::new(protocol_version),
                ContractVersionRange::new(
                    ContractVersion::new(range_min),
                    ContractVersion::new(range_max),
                )
                .expect("valid range"),
                SatelliteMode::Local,
                "handshake-v1",
                "health-v1",
            )
            .expect("valid satellite declaration"),
        );
        entry
    }

    mod unique_ids {
        use super::*;

        #[test]
        fn valid_entries_produce_no_errors() {
            let entries = vec![
                with_action(
                    with_route(
                        with_namespace(base_entry("acta"), "acta"),
                        HttpMethod::Get,
                        "/tasks",
                    ),
                    "acta::task::read",
                ),
                with_action(
                    with_route(
                        with_namespace(base_entry("custos"), "custos"),
                        HttpMethod::Get,
                        "/principals",
                    ),
                    "custos::principal::read",
                ),
            ];

            assert!(validate_unique_ids(&entries).is_empty());
        }

        #[test]
        fn same_stable_id_twice_is_rejected() {
            let entries = vec![base_entry("acta"), base_entry("acta")];

            let errors = validate_unique_ids(&entries);
            assert!(errors.contains(&RegistryBuildError::DuplicateStableId {
                component: component("acta")
            }));
        }

        #[test]
        fn same_namespace_from_two_components_is_rejected() {
            let entries = vec![
                with_namespace(base_entry("acta"), "shared"),
                with_namespace(base_entry("custos"), "shared"),
            ];

            let errors = validate_unique_ids(&entries);
            assert!(errors.contains(&RegistryBuildError::DuplicateNamespace {
                namespace: "shared".to_string(),
                components: vec![component("acta"), component("custos")],
            }));
        }

        #[test]
        fn same_route_twice_inside_one_component_is_rejected() {
            let entry = with_route(
                with_route(
                    with_namespace(base_entry("acta"), "acta"),
                    HttpMethod::Get,
                    "/tasks",
                ),
                HttpMethod::Get,
                "/tasks",
            );

            let errors = validate_unique_ids(&[entry]);
            assert!(errors.contains(&RegistryBuildError::DuplicateRoute {
                namespace: Some("acta".to_string()),
                method: HttpMethod::Get,
                path: RoutePath::new("/tasks").expect("valid route path"),
                components: vec![component("acta"), component("acta")],
            }));
        }

        #[test]
        fn same_route_in_two_components_sharing_a_namespace_is_rejected() {
            let entries = vec![
                with_route(
                    with_namespace(base_entry("acta"), "shared"),
                    HttpMethod::Get,
                    "/tasks",
                ),
                with_route(
                    with_namespace(base_entry("custos"), "shared"),
                    HttpMethod::Get,
                    "/tasks",
                ),
            ];

            let errors = validate_unique_ids(&entries);
            assert!(errors.contains(&RegistryBuildError::DuplicateRoute {
                namespace: Some("shared".to_string()),
                method: HttpMethod::Get,
                path: RoutePath::new("/tasks").expect("valid route path"),
                components: vec![component("acta"), component("custos")],
            }));
        }

        #[test]
        fn same_path_in_different_namespaces_is_not_rejected() {
            let entries = vec![
                with_route(
                    with_namespace(base_entry("acta"), "acta"),
                    HttpMethod::Get,
                    "/tasks",
                ),
                with_route(
                    with_namespace(base_entry("custos"), "custos"),
                    HttpMethod::Get,
                    "/tasks",
                ),
            ];

            let errors = validate_unique_ids(&entries);
            assert!(
                !errors
                    .iter()
                    .any(|error| matches!(error, RegistryBuildError::DuplicateRoute { .. }))
            );
        }

        #[test]
        fn same_action_id_in_two_components_is_rejected() {
            let entries = vec![
                with_action(base_entry("acta"), "acta::task::read"),
                with_action(base_entry("custos"), "acta::task::read"),
            ];

            let errors = validate_unique_ids(&entries);
            assert!(errors.contains(&RegistryBuildError::DuplicateAction {
                action: action("acta::task::read"),
                components: vec![component("acta"), component("custos")],
            }));
        }
    }

    mod dependencies {
        use super::*;

        #[test]
        fn valid_dependency_produces_no_errors() {
            let entries = vec![
                base_entry("platform"),
                with_dependency(base_entry("acta"), "platform", 1),
            ];
            let index = ComponentIndex::build(&entries);

            assert!(validate_dependencies(&entries, &index).is_empty());
        }

        #[test]
        fn dependency_on_absent_id_is_rejected() {
            let entries = vec![with_dependency(base_entry("acta"), "ghost", 1)];
            let index = ComponentIndex::build(&entries);

            let errors = validate_dependencies(&entries, &index);
            assert!(errors.contains(&RegistryBuildError::UnknownDependency {
                component: component("acta"),
                dependency: component("ghost"),
            }));
        }

        #[test]
        fn min_contract_above_declared_is_rejected() {
            let entries = vec![
                base_entry("platform"),
                with_dependency(base_entry("acta"), "platform", 3),
            ];
            let index = ComponentIndex::build(&entries);

            let errors = validate_dependencies(&entries, &index);
            assert!(
                errors.contains(&RegistryBuildError::MinContractNotSatisfied {
                    component: component("acta"),
                    dependency: component("platform"),
                    required: ContractVersion::new(3),
                    actual: ContractVersion::new(1),
                })
            );
        }

        #[test]
        fn min_contract_equal_to_declared_passes() {
            let entries = vec![
                base_entry("platform"),
                with_dependency(base_entry("acta"), "platform", 1),
            ];
            let index = ComponentIndex::build(&entries);

            let errors = validate_dependencies(&entries, &index);
            assert!(
                !errors.iter().any(|error| matches!(
                    error,
                    RegistryBuildError::MinContractNotSatisfied { .. }
                ))
            );
        }

        #[test]
        fn two_cycle_is_rejected() {
            let entries = vec![
                with_dependency(base_entry("a"), "b", 1),
                with_dependency(base_entry("b"), "a", 1),
            ];
            let index = ComponentIndex::build(&entries);

            let errors = validate_dependencies(&entries, &index);
            assert!(errors.contains(&RegistryBuildError::DependencyCycle {
                chain: vec![component("a"), component("b")],
            }));
        }

        #[test]
        fn unknown_dependency_does_not_also_emit_a_cycle() {
            let entries = vec![with_dependency(base_entry("acta"), "ghost", 1)];
            let index = ComponentIndex::build(&entries);

            let errors = validate_dependencies(&entries, &index);
            assert!(
                !errors
                    .iter()
                    .any(|error| matches!(error, RegistryBuildError::DependencyCycle { .. }))
            );
        }
    }

    mod capabilities {
        use super::*;

        #[test]
        fn valid_capabilities_produce_no_errors() {
            let entries = vec![
                with_provided_capability(base_entry("storage.filesystem"), "storage.blob"),
                with_mandatory_capability(base_entry("acta"), "storage.blob"),
            ];

            assert!(validate_capabilities(&entries).is_empty());
        }

        #[test]
        fn mandatory_with_zero_providers_is_rejected() {
            let entries = vec![with_mandatory_capability(
                base_entry("acta"),
                "storage.blob",
            )];

            let errors = validate_capabilities(&entries);
            assert!(
                errors.contains(&RegistryBuildError::MandatoryCapabilityUnprovided {
                    capability: capability("storage.blob"),
                    component: component("acta"),
                })
            );
        }

        #[test]
        fn mandatory_with_two_providers_is_rejected_naming_both() {
            let entries = vec![
                with_provided_capability(base_entry("storage.filesystem"), "storage.blob"),
                with_provided_capability(base_entry("storage.s3"), "storage.blob"),
                with_mandatory_capability(base_entry("acta"), "storage.blob"),
            ];

            let errors = validate_capabilities(&entries);
            assert!(
                errors.contains(&RegistryBuildError::MandatoryCapabilityAmbiguous {
                    capability: capability("storage.blob"),
                    providers: vec![component("storage.filesystem"), component("storage.s3")],
                })
            );
        }

        #[test]
        fn optional_with_zero_providers_is_not_rejected() {
            let entries = vec![with_optional_capability(
                base_entry("acta"),
                "search.semantic",
            )];

            let errors = validate_capabilities(&entries);
            assert!(!errors.iter().any(|error| matches!(
                error,
                RegistryBuildError::OptionalCapabilityAmbiguous { .. }
            ) || matches!(
                error,
                RegistryBuildError::MandatoryCapabilityUnprovided { .. }
            )));
        }

        #[test]
        fn optional_with_two_providers_is_rejected() {
            let entries = vec![
                with_provided_capability(base_entry("search.postgres_fts"), "search.lexical"),
                with_provided_capability(
                    base_entry("search.pgvector_embeddings"),
                    "search.lexical",
                ),
                with_optional_capability(base_entry("acta"), "search.lexical"),
            ];

            let errors = validate_capabilities(&entries);
            assert!(
                errors.contains(&RegistryBuildError::OptionalCapabilityAmbiguous {
                    capability: capability("search.lexical"),
                    providers: vec![
                        component("search.pgvector_embeddings"),
                        component("search.postgres_fts")
                    ],
                })
            );
        }

        #[test]
        fn ambiguous_capability_required_by_two_components_is_reported_once() {
            let entries = vec![
                with_provided_capability(base_entry("storage.filesystem"), "storage.blob"),
                with_provided_capability(base_entry("storage.s3"), "storage.blob"),
                with_mandatory_capability(base_entry("acta"), "storage.blob"),
                with_mandatory_capability(base_entry("custos"), "storage.blob"),
            ];

            let errors = validate_capabilities(&entries);
            let ambiguous_count = errors
                .iter()
                .filter(|error| {
                    matches!(
                        error,
                        RegistryBuildError::MandatoryCapabilityAmbiguous { .. }
                    )
                })
                .count();
            assert_eq!(ambiguous_count, 1);
        }
    }

    mod persistence {
        use super::*;

        #[test]
        fn valid_persistence_produces_no_errors() {
            let entries = vec![with_config(with_persistence(
                base_entry("acta"),
                "acta",
                "acta",
            ))];
            let index = ComponentIndex::build(&entries);

            assert!(validate_persistence(&entries, &index).is_empty());
        }

        #[test]
        fn duplicate_schema_owner_is_rejected() {
            let entries = vec![
                with_config(with_persistence(base_entry("acta"), "acta", "acta")),
                with_config(with_persistence(base_entry("custos"), "acta", "custos")),
            ];
            let index = ComponentIndex::build(&entries);

            let errors = validate_persistence(&entries, &index);
            assert!(errors.contains(&RegistryBuildError::DuplicateSchemaOwner {
                schema: schema("acta"),
                components: vec![component("acta"), component("custos")],
            }));
        }

        #[test]
        fn unknown_migration_owner_is_rejected() {
            let entries = vec![with_config(with_persistence(
                base_entry("acta"),
                "acta",
                "ghost",
            ))];
            let index = ComponentIndex::build(&entries);

            let errors = validate_persistence(&entries, &index);
            assert!(errors.contains(&RegistryBuildError::UnknownMigrationOwner {
                component: component("acta"),
                migration_owner: component("ghost"),
            }));
        }

        #[test]
        fn persistent_component_with_no_config_is_rejected() {
            let entries = vec![with_persistence(base_entry("acta"), "acta", "acta")];
            let index = ComponentIndex::build(&entries);

            let errors = validate_persistence(&entries, &index);
            assert!(
                errors.contains(&RegistryBuildError::PersistenceWithoutConfig {
                    component: component("acta"),
                })
            );
        }

        #[test]
        fn non_persistent_component_with_no_config_is_not_rejected() {
            let entries = vec![base_entry("storage.filesystem")];
            let index = ComponentIndex::build(&entries);

            assert!(validate_persistence(&entries, &index).is_empty());
        }
    }

    mod migration_order {
        use super::*;

        #[test]
        fn valid_chain_order_is_returned() {
            let platform = with_provided_contract(
                with_config(with_persistence(
                    base_entry("platform"),
                    "platform",
                    "platform",
                )),
                "platform.core",
            );
            let custos = with_provided_contract(
                with_required_contract(
                    with_config(with_persistence(base_entry("custos"), "custos", "custos")),
                    "platform.core",
                ),
                "custos.principals",
            );
            let acta = with_required_contract(
                with_config(with_persistence(base_entry("acta"), "acta", "acta")),
                "custos.principals",
            );

            let order =
                validate_migration_order(&[platform, custos, acta]).expect("valid migration order");
            assert_eq!(
                order,
                vec![
                    component("platform"),
                    component("custos"),
                    component("acta")
                ]
            );
        }

        #[test]
        fn required_contract_with_no_provider_is_rejected() {
            let acta = with_required_contract(
                with_config(with_persistence(base_entry("acta"), "acta", "acta")),
                "platform.core",
            );

            let errors = validate_migration_order(&[acta]).expect_err("missing provider");
            assert!(
                errors.contains(&RegistryBuildError::UnprovidedSchemaContract {
                    component: component("acta"),
                    contract: contract("platform.core"),
                })
            );
        }

        #[test]
        fn contract_cycle_is_rejected() {
            let custos = with_provided_contract(
                with_required_contract(
                    with_config(with_persistence(base_entry("custos"), "custos", "custos")),
                    "acta.core",
                ),
                "custos.core",
            );
            let acta = with_provided_contract(
                with_required_contract(
                    with_config(with_persistence(base_entry("acta"), "acta", "acta")),
                    "custos.core",
                ),
                "acta.core",
            );

            let errors = validate_migration_order(&[custos, acta]).expect_err("contract cycle");
            assert!(
                errors
                    .iter()
                    .any(|error| matches!(error, RegistryBuildError::SchemaContractCycle { .. }))
            );
        }

        #[test]
        fn non_persistent_components_are_excluded_from_the_order() {
            let platform = with_config(with_persistence(
                base_entry("platform"),
                "platform",
                "platform",
            ));
            let module = base_entry("storage.filesystem");

            let order = validate_migration_order(&[platform, module]).expect("valid order");
            assert_eq!(order, vec![component("platform")]);
        }

        #[test]
        fn independent_persistent_components_order_deterministically() {
            let first = with_config(with_persistence(base_entry("beta"), "beta", "beta"));
            let second = with_config(with_persistence(base_entry("alpha"), "alpha", "alpha"));

            let order = validate_migration_order(&[first, second]).expect("valid order");
            assert_eq!(order, vec![component("alpha"), component("beta")]);
        }
    }

    mod satellites {
        use super::*;

        #[test]
        fn valid_satellite_produces_no_errors() {
            let entries = vec![with_satellite(base_entry("acta"), "acta", 2, 1, 3)];
            let index = ComponentIndex::build(&entries);

            assert!(validate_satellites(&entries, &index).is_empty());
        }

        #[test]
        fn unknown_owner_is_rejected() {
            let entries = vec![with_satellite(base_entry("acta"), "ghost", 1, 1, 1)];
            let index = ComponentIndex::build(&entries);

            let errors = validate_satellites(&entries, &index);
            assert!(errors.contains(&RegistryBuildError::UnknownSatelliteOwner {
                component: component("acta"),
                owner: component("ghost"),
            }));
        }

        #[test]
        fn protocol_version_above_range_max_is_rejected() {
            let entries = vec![with_satellite(base_entry("acta"), "acta", 4, 1, 3)];
            let index = ComponentIndex::build(&entries);

            let errors = validate_satellites(&entries, &index);
            assert!(
                errors.contains(&RegistryBuildError::SatelliteProtocolOutOfRange {
                    component: component("acta"),
                    protocol_version: ContractVersion::new(4),
                    compatible_range: ContractVersionRange::new(
                        ContractVersion::new(1),
                        ContractVersion::new(3)
                    )
                    .expect("valid range"),
                })
            );
        }

        #[test]
        fn protocol_version_below_range_min_is_rejected() {
            let entries = vec![with_satellite(base_entry("acta"), "acta", 0, 1, 3)];
            let index = ComponentIndex::build(&entries);

            let errors = validate_satellites(&entries, &index);
            assert!(
                errors.contains(&RegistryBuildError::SatelliteProtocolOutOfRange {
                    component: component("acta"),
                    protocol_version: ContractVersion::new(0),
                    compatible_range: ContractVersionRange::new(
                        ContractVersion::new(1),
                        ContractVersion::new(3)
                    )
                    .expect("valid range"),
                })
            );
        }

        #[test]
        fn protocol_version_on_inclusive_bound_is_not_rejected() {
            let entries = vec![with_satellite(base_entry("acta"), "acta", 3, 1, 3)];
            let index = ComponentIndex::build(&entries);

            assert!(validate_satellites(&entries, &index).is_empty());
        }
    }

    mod composition {
        use super::*;

        fn valid_entries() -> Vec<ComponentEntry> {
            vec![
                with_provided_capability(base_entry("platform"), "platform.core"),
                base_entry("acta"),
            ]
        }

        #[test]
        fn ok_path_resolves_every_entry_via_index() {
            let registry = build(valid_entries()).expect("valid entries build");

            for stable_id in ["platform", "acta"] {
                assert!(registry.get(&component(stable_id)).is_some());
            }
        }

        fn broken_entries() -> Vec<ComponentEntry> {
            vec![
                with_dependency(base_entry("acta"), "ghost", 1),
                with_mandatory_capability(base_entry("custos"), "storage.blob"),
            ]
        }

        #[test]
        fn errors_from_two_validators_appear_in_validator_order() {
            let errors = build(broken_entries()).expect_err("broken entries fail to build");

            let dependency_position = errors
                .iter()
                .position(|error| matches!(error, RegistryBuildError::UnknownDependency { .. }))
                .expect("unknown dependency error present");
            let capability_position = errors
                .iter()
                .position(|error| {
                    matches!(
                        error,
                        RegistryBuildError::MandatoryCapabilityUnprovided { .. }
                    )
                })
                .expect("mandatory capability error present");

            assert!(dependency_position < capability_position);
        }

        #[test]
        fn identical_error_vectors_across_two_runs() {
            let first = build(broken_entries()).expect_err("broken entries fail to build");
            let second = build(broken_entries()).expect_err("broken entries fail to build");

            assert_eq!(first, second);
        }
    }
}
