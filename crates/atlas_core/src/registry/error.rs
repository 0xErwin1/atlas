use super::{
    CapabilityId, ComponentId, ContractVersion, ContractVersionRange, HttpMethod, RoutePath,
    SchemaContractId, SchemaId, WorkerId,
};
use crate::ids::ActionId;

/// A single violation of the registry validation matrix, returned in bulk by
/// `build()` so every violation is reported at once (SHELL-REG-3).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryBuildError {
    #[error("duplicate component stable_id `{component}`")]
    DuplicateStableId { component: ComponentId },
    #[error("api namespace `{namespace}` is declared by {}", join_ids(.components))]
    DuplicateNamespace {
        namespace: String,
        components: Vec<ComponentId>,
    },
    #[error(
        "duplicate route `{method} {path}` in namespace `{}` declared by {}",
        namespace_label(.namespace), join_ids(.components)
    )]
    DuplicateRoute {
        namespace: Option<String>,
        method: HttpMethod,
        path: RoutePath,
        components: Vec<ComponentId>,
    },
    #[error("action `{action}` is declared by {}", join_ids(.components))]
    DuplicateAction {
        action: ActionId,
        components: Vec<ComponentId>,
    },

    #[error("component `{component}` depends on unknown component `{dependency}`")]
    UnknownDependency {
        component: ComponentId,
        dependency: ComponentId,
    },
    #[error("dependency cycle: {}", join_chain(.chain))]
    DependencyCycle { chain: Vec<ComponentId> },
    #[error(
        "component `{component}` requires `{dependency}` contract >= {required} but it declares {actual}"
    )]
    MinContractNotSatisfied {
        component: ComponentId,
        dependency: ComponentId,
        required: ContractVersion,
        actual: ContractVersion,
    },

    #[error("mandatory capability `{capability}` required by `{component}` has no provider")]
    MandatoryCapabilityUnprovided {
        capability: CapabilityId,
        component: ComponentId,
    },
    #[error(
        "mandatory capability `{capability}` has more than one provider: {}",
        join_ids(.providers)
    )]
    MandatoryCapabilityAmbiguous {
        capability: CapabilityId,
        providers: Vec<ComponentId>,
    },
    #[error(
        "optional capability `{capability}` has more than one provider: {}",
        join_ids(.providers)
    )]
    OptionalCapabilityAmbiguous {
        capability: CapabilityId,
        providers: Vec<ComponentId>,
    },

    #[error("persistent component `{component}` declares no config struct")]
    PersistenceWithoutConfig { component: ComponentId },
    #[error("schema `{schema}` is owned by {}", join_ids(.components))]
    DuplicateSchemaOwner {
        schema: SchemaId,
        components: Vec<ComponentId>,
    },
    #[error("component `{component}` names unknown migration owner `{migration_owner}`")]
    UnknownMigrationOwner {
        component: ComponentId,
        migration_owner: ComponentId,
    },

    #[error(
        "component `{component}` requires schema contract `{contract}` that no component provides"
    )]
    UnprovidedSchemaContract {
        component: ComponentId,
        contract: SchemaContractId,
    },
    #[error("schema contract cycle: {}", join_chain(.chain))]
    SchemaContractCycle { chain: Vec<ComponentId> },

    #[error("satellite of `{component}` names unknown owner `{owner}`")]
    UnknownSatelliteOwner {
        component: ComponentId,
        owner: ComponentId,
    },
    #[error(
        "satellite of `{component}` declares protocol_version {protocol_version} outside compatible_range {compatible_range}"
    )]
    SatelliteProtocolOutOfRange {
        component: ComponentId,
        protocol_version: ContractVersion,
        compatible_range: ContractVersionRange,
    },

    #[error("worker id `{worker}` is declared by {}", join_ids(.components))]
    DuplicateWorkerId {
        worker: WorkerId,
        components: Vec<ComponentId>,
    },
    #[error(
        "critical worker `{worker}` declared by `{component}` has no diagnostics readiness surface"
    )]
    CriticalWorkerWithoutReadiness {
        component: ComponentId,
        worker: WorkerId,
    },
    #[error("startup order cycle: {}", join_chain(.chain))]
    StartupOrderCycle { chain: Vec<ComponentId> },
}

/// Formats component ids as a comma-separated list: `"acta, custos"`.
fn join_ids(ids: &[ComponentId]) -> String {
    ids.iter()
        .map(ComponentId::as_str)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Formats a cycle chain as `"acta -> custos -> acta"`, repeating the first
/// element at the end.
fn join_chain(chain: &[ComponentId]) -> String {
    let Some(first) = chain.first() else {
        return String::new();
    };

    let mut labels: Vec<&str> = chain.iter().map(ComponentId::as_str).collect();
    labels.push(first.as_str());
    labels.join(" -> ")
}

/// Renders an optional API namespace for error messages: `Some` yields the
/// value, `None` yields `"<none>"`.
fn namespace_label(namespace: &Option<String>) -> &str {
    namespace.as_deref().unwrap_or("<none>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    #[test]
    fn duplicate_stable_id_message() {
        let error = RegistryBuildError::DuplicateStableId {
            component: component("acta"),
        };
        assert_eq!(error.to_string(), "duplicate component stable_id `acta`");
    }

    #[test]
    fn duplicate_namespace_message() {
        let error = RegistryBuildError::DuplicateNamespace {
            namespace: "acta".to_string(),
            components: vec![component("acta"), component("custos")],
        };
        assert_eq!(
            error.to_string(),
            "api namespace `acta` is declared by acta, custos"
        );
    }

    #[test]
    fn duplicate_route_message_with_namespace() {
        let error = RegistryBuildError::DuplicateRoute {
            namespace: Some("acta".to_string()),
            method: HttpMethod::Get,
            path: RoutePath::new("/tasks/{task_id}").expect("valid route path"),
            components: vec![component("acta"), component("custos")],
        };
        assert_eq!(
            error.to_string(),
            "duplicate route `GET /tasks/{task_id}` in namespace `acta` declared by acta, custos"
        );
    }

    #[test]
    fn duplicate_route_message_without_namespace() {
        let error = RegistryBuildError::DuplicateRoute {
            namespace: None,
            method: HttpMethod::Post,
            path: RoutePath::new("/tasks").expect("valid route path"),
            components: vec![component("acta")],
        };
        assert_eq!(
            error.to_string(),
            "duplicate route `POST /tasks` in namespace `<none>` declared by acta"
        );
    }

    #[test]
    fn duplicate_action_message() {
        let error = RegistryBuildError::DuplicateAction {
            action: "acta::task::read".parse().expect("valid action id"),
            components: vec![component("acta"), component("custos")],
        };
        assert_eq!(
            error.to_string(),
            "action `acta::task::read` is declared by acta, custos"
        );
    }

    #[test]
    fn unknown_dependency_message() {
        let error = RegistryBuildError::UnknownDependency {
            component: component("acta"),
            dependency: component("ghost"),
        };
        assert_eq!(
            error.to_string(),
            "component `acta` depends on unknown component `ghost`"
        );
    }

    #[test]
    fn dependency_cycle_message() {
        let error = RegistryBuildError::DependencyCycle {
            chain: vec![component("a"), component("b"), component("c")],
        };
        assert_eq!(error.to_string(), "dependency cycle: a -> b -> c -> a");
    }

    #[test]
    fn min_contract_not_satisfied_message() {
        let error = RegistryBuildError::MinContractNotSatisfied {
            component: component("acta"),
            dependency: component("platform"),
            required: ContractVersion::new(2),
            actual: ContractVersion::new(1),
        };
        assert_eq!(
            error.to_string(),
            "component `acta` requires `platform` contract >= 2 but it declares 1"
        );
    }

    #[test]
    fn mandatory_capability_unprovided_message() {
        let error = RegistryBuildError::MandatoryCapabilityUnprovided {
            capability: CapabilityId::new("storage.blob").expect("valid capability id"),
            component: component("acta"),
        };
        assert_eq!(
            error.to_string(),
            "mandatory capability `storage.blob` required by `acta` has no provider"
        );
    }

    #[test]
    fn mandatory_capability_ambiguous_message() {
        let error = RegistryBuildError::MandatoryCapabilityAmbiguous {
            capability: CapabilityId::new("storage.blob").expect("valid capability id"),
            providers: vec![component("storage.filesystem"), component("storage.s3")],
        };
        assert_eq!(
            error.to_string(),
            "mandatory capability `storage.blob` has more than one provider: storage.filesystem, storage.s3"
        );
    }

    #[test]
    fn optional_capability_ambiguous_message() {
        let error = RegistryBuildError::OptionalCapabilityAmbiguous {
            capability: CapabilityId::new("search.lexical").expect("valid capability id"),
            providers: vec![
                component("search.postgres_fts"),
                component("search.pgvector_embeddings"),
            ],
        };
        assert_eq!(
            error.to_string(),
            "optional capability `search.lexical` has more than one provider: search.postgres_fts, search.pgvector_embeddings"
        );
    }

    #[test]
    fn persistence_without_config_message() {
        let error = RegistryBuildError::PersistenceWithoutConfig {
            component: component("acta"),
        };
        assert_eq!(
            error.to_string(),
            "persistent component `acta` declares no config struct"
        );
    }

    #[test]
    fn duplicate_schema_owner_message() {
        let error = RegistryBuildError::DuplicateSchemaOwner {
            schema: SchemaId::new("acta").expect("valid schema id"),
            components: vec![component("acta"), component("custos")],
        };
        assert_eq!(error.to_string(), "schema `acta` is owned by acta, custos");
    }

    #[test]
    fn unknown_migration_owner_message() {
        let error = RegistryBuildError::UnknownMigrationOwner {
            component: component("acta"),
            migration_owner: component("ghost"),
        };
        assert_eq!(
            error.to_string(),
            "component `acta` names unknown migration owner `ghost`"
        );
    }

    #[test]
    fn unprovided_schema_contract_message() {
        let error = RegistryBuildError::UnprovidedSchemaContract {
            component: component("acta"),
            contract: SchemaContractId::new("platform.core").expect("valid schema contract id"),
        };
        assert_eq!(
            error.to_string(),
            "component `acta` requires schema contract `platform.core` that no component provides"
        );
    }

    #[test]
    fn schema_contract_cycle_message() {
        let error = RegistryBuildError::SchemaContractCycle {
            chain: vec![component("custos"), component("acta")],
        };
        assert_eq!(
            error.to_string(),
            "schema contract cycle: custos -> acta -> custos"
        );
    }

    #[test]
    fn unknown_satellite_owner_message() {
        let error = RegistryBuildError::UnknownSatelliteOwner {
            component: component("acta"),
            owner: component("ghost"),
        };
        assert_eq!(
            error.to_string(),
            "satellite of `acta` names unknown owner `ghost`"
        );
    }

    #[test]
    fn satellite_protocol_out_of_range_message() {
        let error = RegistryBuildError::SatelliteProtocolOutOfRange {
            component: component("acta"),
            protocol_version: ContractVersion::new(4),
            compatible_range: ContractVersionRange::new(
                ContractVersion::new(1),
                ContractVersion::new(3),
            )
            .expect("valid range"),
        };
        assert_eq!(
            error.to_string(),
            "satellite of `acta` declares protocol_version 4 outside compatible_range 1..=3"
        );
    }

    fn worker(value: &str) -> WorkerId {
        WorkerId::new(value).expect("valid worker id")
    }

    #[test]
    fn duplicate_worker_id_message() {
        let error = RegistryBuildError::DuplicateWorkerId {
            worker: worker("acta.reindex"),
            components: vec![component("acta"), component("custos")],
        };
        assert_eq!(
            error.to_string(),
            "worker id `acta.reindex` is declared by acta, custos"
        );
    }

    #[test]
    fn critical_worker_without_readiness_message() {
        let error = RegistryBuildError::CriticalWorkerWithoutReadiness {
            component: component("acta"),
            worker: worker("acta.reindex"),
        };
        assert_eq!(
            error.to_string(),
            "critical worker `acta.reindex` declared by `acta` has no diagnostics readiness surface"
        );
    }

    #[test]
    fn startup_order_cycle_message() {
        let error = RegistryBuildError::StartupOrderCycle {
            chain: vec![component("a"), component("b")],
        };
        assert_eq!(error.to_string(), "startup order cycle: a -> b -> a");
    }

    #[test]
    fn join_ids_formats_empty_and_single_and_multiple() {
        assert_eq!(join_ids(&[]), "");
        assert_eq!(join_ids(&[component("acta")]), "acta");
        assert_eq!(
            join_ids(&[component("acta"), component("custos")]),
            "acta, custos"
        );
    }

    #[test]
    fn join_chain_repeats_first_element_at_the_end() {
        assert_eq!(join_chain(&[component("a")]), "a -> a");
        assert_eq!(
            join_chain(&[component("a"), component("b"), component("c")]),
            "a -> b -> c -> a"
        );
    }

    #[test]
    fn namespace_label_maps_some_and_none() {
        assert_eq!(namespace_label(&Some("acta".to_string())), "acta");
        assert_eq!(namespace_label(&None), "<none>");
    }
}
