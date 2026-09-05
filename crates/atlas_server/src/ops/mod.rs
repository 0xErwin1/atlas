//! Diagnostics registry: binds S2's `Health`/`Readiness` implementers to
//! this server's components (E11-S3a design D1).
//!
//! Named `DiagnosticsRegistry`, not `Diagnostics`: `atlas_core::registry::Diagnostics`
//! is the declaration struct (`reg5.rs`'s `ComponentEntry.diagnostics`) and is
//! already imported there — two `Diagnostics` types in one crate would be a
//! readability trap for zero gain (design D1).

pub mod acta;
pub mod custos;
pub mod deadline;
pub mod meta;
pub mod modules;
pub mod platform;
pub mod supervisor;
pub mod workers;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use atlas_core::capabilities::{Doctor, Health, Readiness};
use atlas_core::registry::{ComponentId, Registry};
use sea_orm::{DatabaseConnection, DbErr};

use crate::config::StorageConfig;
use crate::reg5::{StorageBackend, reg5_component_entries};
use atlas_acta::ports::attachment_store::AttachmentStore;
use atlas_acta::semantic_search::EmbeddingProvider;

use self::acta::{ActaDiagnostics, ActaDoctor};
use self::custos::{CustosDiagnostics, CustosDoctor};
use self::modules::{
    DiskStorageDiagnostics, DiskStorageDoctor, S3StorageDiagnostics, S3StorageDoctor,
    SearchLexicalDiagnostics, SearchLexicalDoctor, SearchSemanticDiagnostics, SearchSemanticDoctor,
};
use self::platform::{PlatformDiagnostics, PlatformDoctor};
use self::workers::WorkerStates;

/// The active storage Module's bound identity plus its three diagnostics
/// implementers, returned by the `storage` match in `default_registry`.
type StorageDiagnosticsParts = (
    ComponentId,
    Arc<dyn Health>,
    Arc<dyn Readiness>,
    Arc<dyn Doctor>,
);

/// One component's bound diagnostics implementers. `doctor` is optional: a
/// component may bind health/readiness with no doctor implementer, but a
/// registry entry declaring `diagnostics.doctor == true` MUST have one
/// (enforced by `bind`).
pub struct ComponentDiagnostics {
    pub health: Arc<dyn Health>,
    pub readiness: Arc<dyn Readiness>,
    pub doctor: Option<Arc<dyn Doctor>>,
}

/// One binding drift between the registry and the implementers `bind` was
/// given, mirroring `registry::BoundWorkers::bind`'s bulk-error, both-direction
/// contract (design D1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsBindError {
    /// A registry entry declares `diagnostics.health`/`diagnostics.readiness`
    /// but no table row was bound for it.
    UnboundComponent { component: ComponentId },
    /// A bound table row's id matches no entry in the registry at all.
    UnknownComponent { component: ComponentId },
    /// A registry entry declares `diagnostics.doctor == true` but its bound
    /// row has no `doctor` implementer (design D5).
    DeclaredDoctorWithNoImplementer { component: ComponentId },
    /// A bound row carries a `doctor` implementer for a component whose
    /// entry does not declare `diagnostics.doctor == true`.
    ImplementerForUndeclaredDoctor { component: ComponentId },
}

/// A validated table of diagnostics implementers keyed by `ComponentId`
/// (design D1): held on `AppState`, consulted by root `/ready` (via
/// `readiness_set`) and by each per-component `/health`/`/ready` handler
/// (via `get`). `readiness_components` is the registry's mandatory set,
/// captured once by `bind` so root `/ready` never rebuilds the registry
/// per request.
pub struct DiagnosticsRegistry {
    table: BTreeMap<ComponentId, ComponentDiagnostics>,
    readiness_components: Vec<ComponentId>,
}

impl DiagnosticsRegistry {
    /// Reconciles `registry`'s diagnostics-bearing entries against `table`,
    /// reporting every violation at once rather than stopping at the first.
    ///
    /// A row is accepted for any id the registry declares, whether or not
    /// that entry's own `diagnostics.health`/`diagnostics.readiness` are
    /// true: a storage/search Module has no HTTP probe of its own, but its
    /// `Readiness` implementer is still bound here so `acta`'s own
    /// readiness can compose it (design D1.2/T1.7).
    pub fn bind(
        registry: &Registry,
        table: Vec<(ComponentId, ComponentDiagnostics)>,
    ) -> Result<Self, Vec<DiagnosticsBindError>> {
        let bound: BTreeMap<ComponentId, ComponentDiagnostics> = table.into_iter().collect();
        let mut errors = Vec::new();

        for entry in registry.entries() {
            let needs_diagnostics = entry.diagnostics.health || entry.diagnostics.readiness;
            if needs_diagnostics && !bound.contains_key(&entry.identity.stable_id) {
                errors.push(DiagnosticsBindError::UnboundComponent {
                    component: entry.identity.stable_id.clone(),
                });
            }

            if entry.diagnostics.doctor {
                let has_doctor = bound
                    .get(&entry.identity.stable_id)
                    .is_some_and(|diagnostics| diagnostics.doctor.is_some());
                if !has_doctor {
                    errors.push(DiagnosticsBindError::DeclaredDoctorWithNoImplementer {
                        component: entry.identity.stable_id.clone(),
                    });
                }
            }
        }

        for id in bound.keys() {
            if registry.get(id).is_none() {
                errors.push(DiagnosticsBindError::UnknownComponent {
                    component: id.clone(),
                });
            }
        }

        for (id, diagnostics) in &bound {
            if diagnostics.doctor.is_some() {
                let declares_doctor = registry
                    .get(id)
                    .is_some_and(|entry| entry.diagnostics.doctor);
                if !declares_doctor {
                    errors.push(DiagnosticsBindError::ImplementerForUndeclaredDoctor {
                        component: id.clone(),
                    });
                }
            }
        }

        if errors.is_empty() {
            Ok(Self {
                table: bound,
                readiness_components: registry.readiness_components(),
            })
        } else {
            Err(errors)
        }
    }

    /// Returns the mandatory readiness set captured at `bind` time
    /// (`Registry::readiness_components()`) paired with each component's
    /// bound `Readiness` implementer, in the registry's own order — root
    /// `/ready`'s input to `aggregate_readiness`. `Err` names the first
    /// mandatory component with no bound row, so the handler can answer
    /// 503 instead of aggregating a partial set.
    pub fn readiness_set(&self) -> Result<Vec<(ComponentId, &dyn Readiness)>, ComponentId> {
        self.readiness_components
            .iter()
            .map(|id| {
                self.table
                    .get(id)
                    .map(|diagnostics| (id.clone(), diagnostics.readiness.as_ref()))
                    .ok_or_else(|| id.clone())
            })
            .collect()
    }

    /// Looks up one component's bound diagnostics, for the per-component
    /// `/health`/`/ready` routes (INV-NO-REINTERPRET: the handler renders
    /// this result directly, with no aggregation).
    pub fn get(&self, id: &ComponentId) -> Option<&ComponentDiagnostics> {
        self.table.get(id)
    }

    /// Every doctor-bearing component (`Registry::doctor_components()`)
    /// paired with its bound `Doctor` implementer, in the registry's own
    /// order — `POST /api/v2/platform/doctor`'s input to `run_doctor`
    /// (design D5). `bind` already guarantees every declared doctor has an
    /// implementer, so a missing row here is silently skipped rather than
    /// panicking: this accessor stays total even if that invariant is ever
    /// violated by a future caller.
    pub fn doctor_set(&self, registry: &Registry) -> Vec<(ComponentId, &dyn Doctor)> {
        registry
            .doctor_components()
            .into_iter()
            .filter_map(|id| {
                self.table
                    .get(&id)
                    .and_then(|diagnostics| diagnostics.doctor.as_deref())
                    .map(|doctor| (id, doctor))
            })
            .collect()
    }
}

fn component(value: &str) -> ComponentId {
    #[allow(clippy::expect_used)]
    ComponentId::new(value).expect("valid component id")
}

/// The variant name of a `DbErr`, for probe logs that must never carry the
/// error's `Display` (it can include the connection string, INV-NO-SECRET).
pub(crate) fn db_error_kind(error: &DbErr) -> &'static str {
    match error {
        DbErr::ConnectionAcquire(_) => "connection_acquire",
        DbErr::Conn(_) => "connection",
        DbErr::Exec(_) => "execution",
        DbErr::Query(_) => "query",
        _ => "other",
    }
}

/// Builds the validated component registry for the configured storage
/// backend: the one `Registry` an `AppState` holds for its whole lifetime,
/// so `/version`, `/api/v2/platform/meta` and `default_registry` all read
/// the same entries and none of them rebuilds it per request.
pub fn component_registry(storage: &StorageConfig) -> Result<Registry, anyhow::Error> {
    let backend = match storage {
        StorageConfig::Disk { .. } => StorageBackend::Filesystem,
        StorageConfig::S3 { .. } => StorageBackend::S3,
    };

    atlas_core::registry::build(reg5_component_entries(backend))
        .map_err(|errors| anyhow::anyhow!("registry validation failed: {errors:?}"))
}

/// Builds the standard six-component diagnostics table shared by
/// `AppState::new` and `AppState::for_test` (design D1.2), so the table the
/// server boots with and the table ~300 tests construct never drift apart.
/// The active storage Module is derived from `storage`, matching whichever
/// entry [`component_registry`] put in `registry`, and the table is bound
/// against that same registry so any drift between the two fails startup
/// instead of a probe.
pub fn default_registry(
    registry: &Registry,
    db: Arc<DatabaseConnection>,
    attachments: Arc<dyn AttachmentStore>,
    embedding_provider: Option<Arc<dyn EmbeddingProvider>>,
    storage: &StorageConfig,
    workers: Arc<WorkerStates>,
    dispatcher_poll_interval: Duration,
) -> Result<DiagnosticsRegistry, anyhow::Error> {
    let platform = Arc::new(PlatformDiagnostics::new(db.clone()));
    let platform_doctor = Arc::new(PlatformDoctor::new(registry));
    let custos = Arc::new(CustosDiagnostics::new(db.clone()));
    let custos_doctor = Arc::new(CustosDoctor::new(db.clone()));
    let lexical = Arc::new(SearchLexicalDiagnostics::new(db.clone()));
    let lexical_doctor = Arc::new(SearchLexicalDoctor::new(db.clone()));
    let semantic = Arc::new(SearchSemanticDiagnostics::new(
        db.clone(),
        embedding_provider.clone(),
    ));
    let semantic_doctor = Arc::new(SearchSemanticDoctor::new(db.clone(), embedding_provider));

    let (storage_id, storage_health, storage_readiness, storage_doctor): StorageDiagnosticsParts =
        match storage {
            StorageConfig::Disk { root } => {
                let disk = Arc::new(DiskStorageDiagnostics::new(root.clone()));
                let doctor = Arc::new(DiskStorageDoctor::new(root.clone()));
                (component("storage.filesystem"), disk.clone(), disk, doctor)
            }
            StorageConfig::S3 { bucket, .. } => {
                let s3 = Arc::new(S3StorageDiagnostics::new(attachments.clone()));
                let doctor = Arc::new(S3StorageDoctor::new(attachments, bucket.clone()));
                (component("storage.s3"), s3.clone(), s3, doctor)
            }
        };

    let acta = Arc::new(ActaDiagnostics::new(db.clone(), storage_readiness.clone()));
    let acta_doctor = Arc::new(ActaDoctor::new(
        db,
        storage_readiness.clone(),
        workers,
        dispatcher_poll_interval,
    ));

    let table: Vec<(ComponentId, ComponentDiagnostics)> = vec![
        (
            component("platform"),
            ComponentDiagnostics {
                health: platform.clone(),
                readiness: platform,
                doctor: Some(platform_doctor),
            },
        ),
        (
            component("custos"),
            ComponentDiagnostics {
                health: custos.clone(),
                readiness: custos,
                doctor: Some(custos_doctor),
            },
        ),
        (
            component("acta"),
            ComponentDiagnostics {
                health: acta.clone(),
                readiness: acta,
                doctor: Some(acta_doctor),
            },
        ),
        (
            component("search.postgres_fts"),
            ComponentDiagnostics {
                health: lexical.clone(),
                readiness: lexical,
                doctor: Some(lexical_doctor),
            },
        ),
        (
            component("search.pgvector_embeddings"),
            ComponentDiagnostics {
                health: semantic.clone(),
                readiness: semantic,
                doctor: Some(semantic_doctor),
            },
        ),
        (
            storage_id,
            ComponentDiagnostics {
                health: storage_health,
                readiness: storage_readiness,
                doctor: Some(storage_doctor),
            },
        ),
    ];

    DiagnosticsRegistry::bind(registry, table)
        .map_err(|errors| anyhow::anyhow!("diagnostics binding failed: {errors:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use atlas_core::capabilities::{HealthStatus, ReadinessStatus};
    use atlas_core::registry::{
        Api, Authorization, Capabilities, ComponentEntry, ComponentKind, ContractVersion,
        Diagnostics, Experience, Identity, build,
    };

    struct Stub;

    impl Health for Stub {
        fn health(&self) -> HealthStatus {
            HealthStatus::Ok
        }
    }

    #[async_trait]
    impl Readiness for Stub {
        async fn readiness(&self) -> ReadinessStatus {
            ReadinessStatus::Ready
        }
    }

    #[async_trait]
    impl Doctor for Stub {
        async fn doctor(&self) -> Vec<atlas_core::capabilities::DoctorFinding> {
            vec![]
        }
    }

    fn stub_diagnostics() -> ComponentDiagnostics {
        ComponentDiagnostics {
            health: Arc::new(Stub),
            readiness: Arc::new(Stub),
            doctor: None,
        }
    }

    fn stub_diagnostics_with_doctor() -> ComponentDiagnostics {
        ComponentDiagnostics {
            health: Arc::new(Stub),
            readiness: Arc::new(Stub),
            doctor: Some(Arc::new(Stub)),
        }
    }

    fn minimal_entry(stable_id: &str, health: bool, readiness: bool) -> ComponentEntry {
        minimal_entry_with_doctor(stable_id, health, readiness, false)
    }

    fn minimal_entry_with_doctor(
        stable_id: &str,
        health: bool,
        readiness: bool,
        doctor: bool,
    ) -> ComponentEntry {
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
                health,
                readiness,
                doctor,
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

    fn registry_with(entries: Vec<ComponentEntry>) -> Registry {
        build(entries).expect("valid registry")
    }

    /// `DiagnosticsRegistry` holds `Arc<dyn Health>`/`Arc<dyn Readiness>`
    /// trait objects, so it has no `Debug` impl and `expect_err` cannot be
    /// used directly on `bind`'s `Result`. This unwraps the error side
    /// without requiring one.
    fn expect_bind_err(
        result: Result<DiagnosticsRegistry, Vec<DiagnosticsBindError>>,
        message: &str,
    ) -> Vec<DiagnosticsBindError> {
        match result {
            Ok(_) => panic!("{message}"),
            Err(errors) => errors,
        }
    }

    #[test]
    fn bind_refuses_a_diagnostics_bearing_entry_with_no_bound_row() {
        let registry = registry_with(vec![minimal_entry("platform", true, true)]);

        let errors = expect_bind_err(
            DiagnosticsRegistry::bind(&registry, vec![]),
            "platform has no row",
        );

        assert_eq!(
            errors,
            vec![DiagnosticsBindError::UnboundComponent {
                component: component("platform")
            }]
        );
    }

    #[test]
    fn bind_refuses_a_bound_row_matching_no_registry_entry() {
        let registry = registry_with(vec![minimal_entry("platform", false, false)]);

        let errors = expect_bind_err(
            DiagnosticsRegistry::bind(&registry, vec![(component("ghost"), stub_diagnostics())]),
            "ghost matches no entry",
        );

        assert_eq!(
            errors,
            vec![DiagnosticsBindError::UnknownComponent {
                component: component("ghost")
            }]
        );
    }

    #[test]
    fn bind_reports_both_directions_in_one_call() {
        let registry = registry_with(vec![
            minimal_entry("platform", true, true),
            minimal_entry("custos", false, false),
        ]);

        let errors = expect_bind_err(
            DiagnosticsRegistry::bind(&registry, vec![(component("ghost"), stub_diagnostics())]),
            "both platform is unbound and ghost is unknown",
        );

        assert_eq!(
            errors,
            vec![
                DiagnosticsBindError::UnboundComponent {
                    component: component("platform")
                },
                DiagnosticsBindError::UnknownComponent {
                    component: component("ghost")
                },
            ]
        );
    }

    #[test]
    fn bind_accepts_a_row_for_a_module_declaring_no_diagnostics_of_its_own() {
        let registry = registry_with(vec![minimal_entry("storage.filesystem", false, false)]);

        let bound = DiagnosticsRegistry::bind(
            &registry,
            vec![(component("storage.filesystem"), stub_diagnostics())],
        )
        .expect("a diagnostics-free module may still have a bound row");

        assert!(bound.get(&component("storage.filesystem")).is_some());
    }

    #[test]
    fn readiness_set_follows_the_registry_mandatory_order() {
        let registry = registry_with(vec![
            minimal_entry("platform", true, true),
            minimal_entry("custos", true, true),
            minimal_entry("storage.filesystem", false, false),
        ]);

        let bound = DiagnosticsRegistry::bind(
            &registry,
            vec![
                (component("custos"), stub_diagnostics()),
                (component("platform"), stub_diagnostics()),
                (component("storage.filesystem"), stub_diagnostics()),
            ],
        )
        .expect("valid binding");

        let ids: Vec<ComponentId> = bound
            .readiness_set()
            .expect("every mandatory component is bound")
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(ids, registry.readiness_components());
    }

    #[test]
    fn bind_refuses_a_declared_doctor_with_no_bound_implementer() {
        let registry = registry_with(vec![minimal_entry_with_doctor("custos", true, true, true)]);

        let errors = expect_bind_err(
            DiagnosticsRegistry::bind(&registry, vec![(component("custos"), stub_diagnostics())]),
            "custos declares a doctor but its row has none",
        );

        assert_eq!(
            errors,
            vec![DiagnosticsBindError::DeclaredDoctorWithNoImplementer {
                component: component("custos")
            }]
        );
    }

    #[test]
    fn bind_refuses_an_implementer_for_an_undeclared_doctor() {
        let registry = registry_with(vec![minimal_entry_with_doctor("custos", true, true, false)]);

        let errors = expect_bind_err(
            DiagnosticsRegistry::bind(
                &registry,
                vec![(component("custos"), stub_diagnostics_with_doctor())],
            ),
            "custos does not declare a doctor but its row has one",
        );

        assert_eq!(
            errors,
            vec![DiagnosticsBindError::ImplementerForUndeclaredDoctor {
                component: component("custos")
            }]
        );
    }

    #[test]
    fn doctor_set_follows_the_registry_doctor_order() {
        let registry = registry_with(vec![
            minimal_entry_with_doctor("platform", true, true, true),
            minimal_entry_with_doctor("custos", true, true, true),
            minimal_entry_with_doctor("storage.filesystem", false, false, false),
        ]);

        let bound = DiagnosticsRegistry::bind(
            &registry,
            vec![
                (component("custos"), stub_diagnostics_with_doctor()),
                (component("platform"), stub_diagnostics_with_doctor()),
                (component("storage.filesystem"), stub_diagnostics()),
            ],
        )
        .expect("valid binding");

        let ids: Vec<ComponentId> = bound
            .doctor_set(&registry)
            .into_iter()
            .map(|(id, _)| id)
            .collect();

        assert_eq!(ids, registry.doctor_components());
        assert!(
            !ids.is_empty(),
            "must be non-vacuous over the two doctor-bearing entries"
        );
    }

    struct UnusedAttachmentStore;

    #[async_trait]
    impl AttachmentStore for UnusedAttachmentStore {
        async fn put(&self, _data: &[u8]) -> Result<String, atlas_core::error::DomainError> {
            unreachable!("not exercised by this test")
        }

        async fn get(&self, _digest: &str) -> Result<bytes::Bytes, atlas_core::error::DomainError> {
            unreachable!("not exercised by this test")
        }

        async fn exists(&self, _digest: &str) -> Result<bool, atlas_core::error::DomainError> {
            unreachable!("not exercised by this test")
        }

        async fn delete(&self, _digest: &str) -> Result<(), atlas_core::error::DomainError> {
            unreachable!("not exercised by this test")
        }
    }

    fn default_registry_for(storage: &StorageConfig) -> DiagnosticsRegistry {
        let registry = component_registry(storage).expect("valid registry");

        default_registry(
            &registry,
            Arc::new(DatabaseConnection::default()),
            Arc::new(UnusedAttachmentStore),
            None,
            storage,
            Arc::new(WorkerStates::from_registry(&registry)),
            Duration::from_secs(60),
        )
        .expect("default table binds against the reg5 registry")
    }

    #[test]
    fn default_registry_binds_against_the_reg5_registry_for_both_backends() {
        let disk = StorageConfig::Disk {
            root: "/nonexistent".to_string(),
        };
        let s3 = StorageConfig::S3 {
            bucket: "bucket".to_string(),
            endpoint: "http://localhost:9000".to_string(),
            access_key_id: "key".to_string(),
            secret_access_key: atlas_core::config::Secret::new("secret".to_string()),
            region: "auto".to_string(),
        };

        for (storage, backend, storage_id) in [
            (disk, StorageBackend::Filesystem, "storage.filesystem"),
            (s3, StorageBackend::S3, "storage.s3"),
        ] {
            let bound = default_registry_for(&storage);
            let registry = build(reg5_component_entries(backend)).expect("valid registry");

            let mandatory: Vec<ComponentId> = bound
                .readiness_set()
                .expect("every mandatory component is bound")
                .into_iter()
                .map(|(id, _)| id)
                .collect();

            assert_eq!(mandatory, registry.readiness_components());
            assert!(bound.get(&component(storage_id)).is_some());
        }
    }
}
