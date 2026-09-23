//! Composition of the V2 `AuthorizationService` (`v2-e5-s5b-grant-authority`):
//! the Custos resource provider over Postgres, the stored-facts load, the V1
//! group-membership source, the Tokio-backed timer the provider timeout
//! races against, and the validation catalog every product's registry
//! declaration contributes to.
//!
//! The registry is the product catalog declaration: a product enters the
//! catalog only once it declares V2 resource kinds, and only its actions of
//! a declared (singular, V2) kind are taken, so a V1 plural scope such as
//! `custos::grants::read` never becomes a grantable action.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sea_orm::DatabaseConnection;

use atlas_core::registry::{ComponentId, Registry};
use atlas_custos::authorize::{AuthorizationService, AuthorizationSettings, ProviderSet, Sleeper};
use atlas_custos::eval::{Catalog, CatalogError, DenyMode, ProductSpec};
use atlas_custos::provider::CustosResourceProvider;
use atlas_custos_postgres::repos::authorize::{
    PgAuthorizationFactsStore, PgCustosResourceStore, PgGroupMembershipSource,
};

use crate::config::DenyModeConfig;

/// The service the server composes: Postgres-backed facts and membership.
pub type ServerAuthorizationService =
    AuthorizationService<PgAuthorizationFactsStore, PgGroupMembershipSource>;

/// The provider timeout timer, backed by the Tokio runtime the server runs
/// on.
pub struct TokioSleeper;

#[async_trait]
impl Sleeper for TokioSleeper {
    async fn sleep(&self, duration: Duration) {
        tokio::time::sleep(duration).await;
    }
}

/// The catalog data of every product that has published V2 resource kinds.
pub fn product_specs(registry: &Registry) -> Vec<ProductSpec> {
    registry
        .entries()
        .iter()
        .filter(|entry| !entry.authorization.resource_kinds.is_empty())
        .map(|entry| {
            let kinds = entry.authorization.resource_kinds.clone();
            let actions = entry
                .authorization
                .actions
                .iter()
                .filter(|action| kinds.iter().any(|kind| kind == action.kind()))
                .cloned()
                .collect();

            ProductSpec {
                product: entry.identity.stable_id.as_str().to_string(),
                kinds,
                actions,
                roles: vec![],
                principal_sets: entry.authorization.principal_sets.clone(),
            }
        })
        .collect()
}

/// The validation catalog built from [`product_specs`].
pub fn validation_catalog(registry: &Registry) -> Result<Catalog, CatalogError> {
    Catalog::new(product_specs(registry))
}

/// The evaluator's deny mode for the configured one. `Enforced` is refused
/// at configuration load until E7, so it never reaches here in practice.
pub fn deny_mode(config: DenyModeConfig) -> DenyMode {
    match config {
        DenyModeConfig::Disabled => DenyMode::Disabled,
        DenyModeConfig::Audit => DenyMode::Audit,
        DenyModeConfig::Enforced => DenyMode::Enforced,
    }
}

/// Builds the server's authorization service: the Custos provider publishes
/// the registry's Custos `Authorization`, facts and memberships come from
/// Postgres, and every provider call is bounded by `provider_timeout`.
pub fn build_authorization_service(
    registry: &Registry,
    db: DatabaseConnection,
    deny_mode_config: DenyModeConfig,
    provider_timeout: Duration,
) -> Result<ServerAuthorizationService, anyhow::Error> {
    let custos = ComponentId::new("custos")
        .ok()
        .and_then(|id| registry.get(&id))
        .ok_or_else(|| anyhow::anyhow!("the registry declares no custos component"))?;
    let provider = CustosResourceProvider::new(
        PgCustosResourceStore { conn: db.clone() },
        &custos.authorization,
    );
    let catalog = validation_catalog(registry).map_err(|e| {
        anyhow::anyhow!("registry authorization declarations do not form a catalog: {e}")
    })?;

    let settings = AuthorizationSettings {
        catalog,
        deny_mode: deny_mode(deny_mode_config),
        provider_timeout,
    };

    Ok(AuthorizationService::new(
        ProviderSet::new().with("custos", Arc::new(provider)),
        PgAuthorizationFactsStore { conn: db.clone() },
        PgGroupMembershipSource { conn: db },
        Arc::new(TokioSleeper),
        settings,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_tokio_sleeper_waits_for_the_requested_duration() {
        let started = tokio::time::Instant::now();

        TokioSleeper.sleep(Duration::from_millis(20)).await;

        assert!(started.elapsed() >= Duration::from_millis(20));
    }

    #[test]
    fn the_configured_deny_mode_maps_onto_the_evaluators() {
        assert_eq!(deny_mode(DenyModeConfig::Disabled), DenyMode::Disabled);
        assert_eq!(deny_mode(DenyModeConfig::Audit), DenyMode::Audit);
        assert_eq!(deny_mode(DenyModeConfig::Enforced), DenyMode::Enforced);
    }
}
