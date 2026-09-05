//! `platform`'s diagnostics implementer (design D1): a `SELECT 1` on the
//! shared pool, moved verbatim from `routes/health.rs`'s pre-S3a `ready`
//! handler. Also `PlatformDoctor` (design D5, orchestrator decision
//! 2026-09-04): platform's minimal doctor, re-checking registry-shaped
//! invariants and config-composition presence rather than duplicating its
//! own `SELECT 1` readiness signal.

use std::sync::Arc;

use async_trait::async_trait;
use atlas_core::capabilities::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus, Severity,
};
use atlas_core::registry::{ComponentId, Registry};
use sea_orm::{ConnectionTrait, DatabaseBackend, DatabaseConnection, Statement};

use super::db_error_kind;

/// `health()` never touches the pool (SHELL-OPS-1): the process answering
/// HTTP at all is the only signal. `readiness()` issues one `SELECT 1` on
/// the shared pool, mapping any error to `NotReady` with a fixed reason —
/// never the raw `sea_orm::DbErr::Display`, which can carry the connection
/// string (INV-NO-SECRET).
pub struct PlatformDiagnostics {
    db: Arc<DatabaseConnection>,
}

impl PlatformDiagnostics {
    pub fn new(db: Arc<DatabaseConnection>) -> Self {
        Self { db }
    }
}

impl Health for PlatformDiagnostics {
    fn health(&self) -> HealthStatus {
        HealthStatus::Ok
    }
}

#[async_trait]
impl Readiness for PlatformDiagnostics {
    async fn readiness(&self) -> ReadinessStatus {
        let probe = self
            .db
            .execute_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                "SELECT 1",
            ))
            .await;

        match probe {
            Ok(_) => ReadinessStatus::Ready,
            Err(error) => {
                tracing::warn!(
                    target: "ops.platform",
                    event = "readiness_failed",
                    error_kind = db_error_kind(&error),
                    "platform readiness probe failed: database unreachable"
                );
                ReadinessStatus::NotReady {
                    reason: "database is unreachable".to_string(),
                }
            }
        }
    }
}

/// A mandatory config declaration whose env prefix has no matching variable
/// name in the process environment.
struct MandatoryConfig {
    component: ComponentId,
    env_prefix: String,
}

/// Platform's minimal doctor (design D5, orchestrator decision 2026-09-04).
/// `Registry::get`/`build()` already guarantee every dependency resolves and
/// every mandatory capability has a provider before a `Registry` value can
/// exist at all, so re-asserting those two invariants here would be
/// unreachable dead code, not a findable condition. What this doctor
/// actually contributes is the one live-registry fact `build()` cannot see:
/// whether the process environment carries a variable under each
/// mandatory config's declared prefix — checked by **name only**, never a
/// value, so no finding can carry a secret (INV-NO-SECRET). Precomputed
/// once at construction; `doctor()` only re-reads the frozen result plus a
/// snapshot of environment variable *names*, so no I/O crosses the deadline
/// boundary.
pub struct PlatformDoctor {
    mandatory_configs: Vec<MandatoryConfig>,
    env_var_names: std::collections::BTreeSet<String>,
}

impl PlatformDoctor {
    pub fn new(registry: &Registry) -> Self {
        Self::built_from(registry, std::env::vars().map(|(key, _)| key).collect())
    }

    #[cfg(test)]
    fn for_test(registry: &Registry, env_var_names: std::collections::BTreeSet<String>) -> Self {
        Self::built_from(registry, env_var_names)
    }

    fn built_from(registry: &Registry, env_var_names: std::collections::BTreeSet<String>) -> Self {
        let mandatory_configs = registry
            .entries()
            .iter()
            .filter_map(|entry| {
                entry.config.as_ref().and_then(|config| {
                    config.mandatory().then(|| MandatoryConfig {
                        component: entry.identity.stable_id.clone(),
                        env_prefix: config.env_prefix().to_string(),
                    })
                })
            })
            .collect();

        Self {
            mandatory_configs,
            env_var_names,
        }
    }
}

#[async_trait]
impl Doctor for PlatformDoctor {
    async fn doctor(&self) -> Vec<DoctorFinding> {
        let platform = super::component("platform");
        let mut findings = Vec::new();

        for config in &self.mandatory_configs {
            let has_variable = self
                .env_var_names
                .iter()
                .any(|name| name.starts_with(config.env_prefix.as_str()));
            if !has_variable {
                findings.push(DoctorFinding {
                    component: platform.clone(),
                    severity: Severity::Warning,
                    finding: format!(
                        "`{}`'s mandatory config prefix `{}` has no matching environment variable",
                        config.component, config.env_prefix
                    ),
                    action: "set the component's required environment variables".to_string(),
                });
            }
        }

        findings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_is_ok_synchronously_with_no_await_reachable() {
        // The absence of `.await` on this call is the assertion: `Health::health`
        // is not `async` (SHELL-OPS-1), so this line would not compile if it were.
        let diagnostics = PlatformDiagnostics::new(Arc::new(unreachable_db()));
        assert_eq!(diagnostics.health(), HealthStatus::Ok);
    }

    /// A `DatabaseConnection` that is never actually queried by this test:
    /// `health()` performs no I/O, so constructing one only proves the type
    /// compiles without a live database. The `readiness()` SELECT-1 path is
    /// covered by the container-backed `tests/api_readiness.rs` suite instead,
    /// since a real pool is required to exercise it meaningfully.
    fn unreachable_db() -> DatabaseConnection {
        DatabaseConnection::default()
    }

    #[tokio::test]
    async fn readiness_maps_a_pool_error_to_not_ready_without_a_raw_error_string() {
        let diagnostics = PlatformDiagnostics::new(Arc::new(unreachable_db()));

        let status = diagnostics.readiness().await;

        assert_eq!(
            status,
            ReadinessStatus::NotReady {
                reason: "database is unreachable".to_string()
            }
        );
    }

    use atlas_core::registry::{
        Api, Authorization, Capabilities, CapabilityId, ComponentKind, ConfigDeclaration,
        ContractVersion, Dependency, Diagnostics, Experience, Identity, build,
    };
    use std::collections::BTreeSet;

    fn component_id(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    fn entry(
        stable_id: &str,
        dependencies: Vec<Dependency>,
        required_mandatory: Vec<CapabilityId>,
        provided: Vec<CapabilityId>,
        config: Option<ConfigDeclaration>,
    ) -> atlas_core::registry::ComponentEntry {
        atlas_core::registry::ComponentEntry {
            identity: Identity {
                stable_id: component_id(stable_id),
                kind: ComponentKind::Module,
                contract_version: ContractVersion::new(1),
            },
            dependencies,
            capabilities: Capabilities {
                provided,
                required_mandatory,
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
            config,
            workers: vec![],
            satellites: vec![],
        }
    }

    #[tokio::test]
    async fn a_healthy_registry_yields_no_findings() {
        let registry =
            build(vec![entry("platform", vec![], vec![], vec![], None)]).expect("valid registry");
        let doctor = PlatformDoctor::for_test(&registry, BTreeSet::new());

        assert!(doctor.doctor().await.is_empty());
    }

    #[tokio::test]
    async fn a_mandatory_config_with_no_matching_env_variable_is_a_warning_naming_only_the_prefix()
    {
        let config = ConfigDeclaration::new("CustosConfig", "ATLAS_CUSTOS_", true)
            .expect("valid config declaration");
        let registry = build(vec![entry("custos", vec![], vec![], vec![], Some(config))])
            .expect("valid registry");
        let doctor = PlatformDoctor::for_test(&registry, BTreeSet::new());

        let findings = doctor.doctor().await;

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Warning);
        assert!(findings[0].finding.contains("ATLAS_CUSTOS_"));
        assert!(
            !findings[0].finding.contains('='),
            "the finding must name the prefix only, never a variable's value"
        );
    }

    #[tokio::test]
    async fn a_mandatory_config_with_a_matching_env_variable_name_yields_no_finding() {
        let config = ConfigDeclaration::new("CustosConfig", "ATLAS_CUSTOS_", true)
            .expect("valid config declaration");
        let registry = build(vec![entry("custos", vec![], vec![], vec![], Some(config))])
            .expect("valid registry");
        let mut env_var_names = BTreeSet::new();
        env_var_names.insert("ATLAS_CUSTOS_DATABASE_URL".to_string());
        let doctor = PlatformDoctor::for_test(&registry, env_var_names);

        assert!(doctor.doctor().await.is_empty());
    }
}
