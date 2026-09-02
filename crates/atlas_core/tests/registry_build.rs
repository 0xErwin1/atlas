#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use atlas_core::ids::ActionId;
use atlas_core::registry::{
    Api, Authorization, Capabilities, CapabilityId, ComponentEntry, ComponentId, ComponentKind,
    ConfigDeclaration, ContractVersion, ContractVersionRange, Dependency, Diagnostics, Experience,
    HttpMethod, Identity, Persistence, RegistryBuildError, RouteDeclaration, RoutePath,
    SatelliteDeclaration, SatelliteMode, SchemaContractId, SchemaId, build,
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

fn config() -> ConfigDeclaration {
    ConfigDeclaration::new("Config", "ATLAS_TEST_", true).expect("valid config declaration")
}

/// Minimal `ComponentEntry` with every optional surface empty. Tests mutate
/// the fields they need.
fn base_entry(stable_id: &str, kind: ComponentKind) -> ComponentEntry {
    ComponentEntry {
        identity: Identity {
            stable_id: component(stable_id),
            kind,
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

/// Builds the SHELL-REG-5 valid entry set: `platform`, `custos`, `acta`,
/// `storage.filesystem`, `search.postgres_fts`, `search.pgvector_embeddings`.
/// `storage.s3` is deliberately absent — two providers of `storage.blob`
/// would be a SHELL-CAP-2 / SH4 failure, and which storage Module to pass to
/// `build()` is a startup composition decision owned by `atlas_server`.
fn shell_reg_5_valid_entries() -> Vec<ComponentEntry> {
    let mut platform = base_entry("platform", ComponentKind::PlatformService);
    platform.persistence = Some(Persistence {
        schema: schema("platform"),
        migration_owner: component("platform"),
        schema_contracts_provided: vec![contract("platform.core")],
        schema_contracts_required: vec![],
    });
    platform.config = Some(config());

    let mut custos = base_entry("custos", ComponentKind::PlatformService);
    custos.persistence = Some(Persistence {
        schema: schema("custos"),
        migration_owner: component("custos"),
        schema_contracts_provided: vec![contract("custos.principals")],
        schema_contracts_required: vec![contract("platform.core")],
    });
    custos.config = Some(config());

    let mut acta = base_entry("acta", ComponentKind::Product);
    acta.persistence = Some(Persistence {
        schema: schema("acta"),
        migration_owner: component("acta"),
        schema_contracts_provided: vec![],
        schema_contracts_required: vec![contract("platform.core"), contract("custos.principals")],
    });
    acta.config = Some(config());
    acta.capabilities
        .required_mandatory
        .push(capability("storage.blob"));
    acta.capabilities
        .required_optional
        .push(capability("search.semantic"));

    let mut storage_filesystem = base_entry("storage.filesystem", ComponentKind::Module);
    storage_filesystem
        .capabilities
        .provided
        .push(capability("storage.blob"));

    let mut search_postgres_fts = base_entry("search.postgres_fts", ComponentKind::Module);
    search_postgres_fts
        .capabilities
        .provided
        .push(capability("search.lexical"));

    let mut search_pgvector_embeddings =
        base_entry("search.pgvector_embeddings", ComponentKind::Module);
    search_pgvector_embeddings
        .capabilities
        .provided
        .push(capability("search.semantic"));

    vec![
        platform,
        custos,
        acta,
        storage_filesystem,
        search_postgres_fts,
        search_pgvector_embeddings,
    ]
}

#[test]
fn shell_reg_5_registry_builds() {
    let registry = build(shell_reg_5_valid_entries()).expect("SHELL-REG-5 entries are valid");

    assert_eq!(registry.entries().len(), 6);

    for stable_id in [
        "platform",
        "custos",
        "acta",
        "storage.filesystem",
        "search.postgres_fts",
        "search.pgvector_embeddings",
    ] {
        assert!(
            registry.get(&component(stable_id)).is_some(),
            "expected {stable_id} to resolve"
        );
    }

    let migration_order: Vec<&str> = registry
        .migration_order()
        .iter()
        .map(ComponentId::as_str)
        .collect();
    assert_eq!(migration_order, vec!["platform", "custos", "acta"]);
}

/// One SHELL-REG-5-shaped fixture deliberately broken to trigger at least
/// one `RegistryBuildError` variant from every validator, asserting exact
/// error payloads and run-to-run determinism.
fn multi_violation_entries() -> Vec<ComponentEntry> {
    let mut acta = base_entry("acta", ComponentKind::Product);
    acta.api.namespace = Some("acta".to_string());
    acta.api.routes.push(RouteDeclaration {
        method: HttpMethod::Get,
        path: RoutePath::new("/tasks").expect("valid route path"),
        operation_id: "listTasks".to_string(),
        action: None,
        idempotent: true,
        is_public: false,
    });
    acta.authorization.actions.push(action("acta::task::read"));
    acta.dependencies.push(Dependency {
        component: component("hermes"),
        min_contract: ContractVersion::new(1),
    });
    acta.dependencies.push(Dependency {
        component: component("custos"),
        min_contract: ContractVersion::new(1),
    });
    acta.dependencies.push(Dependency {
        component: component("platform"),
        min_contract: ContractVersion::new(3),
    });
    acta.capabilities
        .required_mandatory
        .push(capability("storage.blob"));
    acta.capabilities
        .required_mandatory
        .push(capability("search.lexical"));
    acta.capabilities
        .required_optional
        .push(capability("search.semantic"));
    acta.persistence = Some(Persistence {
        schema: schema("acta"),
        migration_owner: component("acta"),
        schema_contracts_provided: vec![contract("acta.core")],
        schema_contracts_required: vec![contract("custos.principals"), contract("custos.core")],
    });
    acta.config = Some(config());

    let mut acta_duplicate = base_entry("acta", ComponentKind::Product);
    acta_duplicate.api.namespace = Some("acta".to_string());
    acta_duplicate.api.routes.push(RouteDeclaration {
        method: HttpMethod::Get,
        path: RoutePath::new("/tasks").expect("valid route path"),
        operation_id: "listTasksDuplicate".to_string(),
        action: None,
        idempotent: true,
        is_public: false,
    });
    acta_duplicate
        .authorization
        .actions
        .push(action("acta::task::read"));

    let mut custos = base_entry("custos", ComponentKind::PlatformService);
    custos.dependencies.push(Dependency {
        component: component("acta"),
        min_contract: ContractVersion::new(1),
    });
    custos.persistence = Some(Persistence {
        schema: schema("acta"),
        migration_owner: component("ghost"),
        schema_contracts_provided: vec![contract("custos.core")],
        schema_contracts_required: vec![contract("acta.core")],
    });
    custos.satellites.push(
        SatelliteDeclaration::new(
            component("mnemosyne"),
            vec![],
            ContractVersion::new(9),
            ContractVersionRange::new(ContractVersion::new(1), ContractVersion::new(3))
                .expect("valid range"),
            SatelliteMode::Remote,
            "handshake-v1",
            "health-v1",
        )
        .expect("valid satellite declaration"),
    );

    let platform = base_entry("platform", ComponentKind::PlatformService);

    let mut storage_filesystem = base_entry("storage.filesystem", ComponentKind::Module);
    storage_filesystem
        .capabilities
        .provided
        .push(capability("storage.blob"));

    let mut storage_s3 = base_entry("storage.s3", ComponentKind::Module);
    storage_s3
        .capabilities
        .provided
        .push(capability("storage.blob"));

    let mut search_postgres_fts = base_entry("search.postgres_fts", ComponentKind::Module);
    search_postgres_fts
        .capabilities
        .provided
        .push(capability("search.semantic"));

    let mut search_pgvector_embeddings =
        base_entry("search.pgvector_embeddings", ComponentKind::Module);
    search_pgvector_embeddings
        .capabilities
        .provided
        .push(capability("search.semantic"));

    vec![
        acta,
        acta_duplicate,
        custos,
        platform,
        storage_filesystem,
        storage_s3,
        search_postgres_fts,
        search_pgvector_embeddings,
    ]
}

#[test]
fn multi_violation_registry_reports_every_matrix_rule() {
    let errors = build(multi_violation_entries()).expect_err("fixture is deliberately broken");

    assert_eq!(errors.len(), 17, "unexpected error count: {errors:?}");

    let expected = [
        RegistryBuildError::DuplicateStableId {
            component: component("acta"),
        },
        RegistryBuildError::DuplicateNamespace {
            namespace: "acta".to_string(),
            components: vec![component("acta"), component("acta")],
        },
        RegistryBuildError::DuplicateRoute {
            namespace: Some("acta".to_string()),
            method: HttpMethod::Get,
            path: RoutePath::new("/tasks").expect("valid route path"),
            components: vec![component("acta"), component("acta")],
        },
        RegistryBuildError::DuplicateAction {
            action: action("acta::task::read"),
            components: vec![component("acta"), component("acta")],
        },
        RegistryBuildError::UnknownDependency {
            component: component("acta"),
            dependency: component("hermes"),
        },
        RegistryBuildError::DependencyCycle {
            chain: vec![component("acta"), component("custos")],
        },
        RegistryBuildError::MinContractNotSatisfied {
            component: component("acta"),
            dependency: component("platform"),
            required: ContractVersion::new(3),
            actual: ContractVersion::new(1),
        },
        RegistryBuildError::MandatoryCapabilityUnprovided {
            capability: capability("search.lexical"),
            component: component("acta"),
        },
        RegistryBuildError::MandatoryCapabilityAmbiguous {
            capability: capability("storage.blob"),
            providers: vec![component("storage.filesystem"), component("storage.s3")],
        },
        RegistryBuildError::OptionalCapabilityAmbiguous {
            capability: capability("search.semantic"),
            providers: vec![
                component("search.pgvector_embeddings"),
                component("search.postgres_fts"),
            ],
        },
        RegistryBuildError::PersistenceWithoutConfig {
            component: component("custos"),
        },
        RegistryBuildError::DuplicateSchemaOwner {
            schema: schema("acta"),
            components: vec![component("acta"), component("custos")],
        },
        RegistryBuildError::UnknownMigrationOwner {
            component: component("custos"),
            migration_owner: component("ghost"),
        },
        RegistryBuildError::UnprovidedSchemaContract {
            component: component("acta"),
            contract: contract("custos.principals"),
        },
        RegistryBuildError::SchemaContractCycle {
            chain: vec![component("acta"), component("custos")],
        },
        RegistryBuildError::UnknownSatelliteOwner {
            component: component("custos"),
            owner: component("mnemosyne"),
        },
        RegistryBuildError::SatelliteProtocolOutOfRange {
            component: component("custos"),
            protocol_version: ContractVersion::new(9),
            compatible_range: ContractVersionRange::new(
                ContractVersion::new(1),
                ContractVersion::new(3),
            )
            .expect("valid range"),
        },
    ];

    for expected_error in &expected {
        assert!(
            errors.contains(expected_error),
            "missing expected error: {expected_error:?}"
        );
    }

    let second_run = build(multi_violation_entries()).expect_err("fixture is deliberately broken");
    assert_eq!(errors, second_run, "build() must be deterministic");
}
