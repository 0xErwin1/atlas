use anyhow::Result;
use atlas_server::persistence::migrator::ComposedMigrator;
use sea_orm::Database;
use sea_orm_migration::prelude::MigratorTrait;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "seed_dev=info".into()),
        )
        .init();

    let storage_backend =
        atlas_server::reg5::storage_backend_from_env(&atlas_core::config::ProcessEnv)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    let cfg = atlas_server::config::AtlasConfig::from_registry(
        &atlas_server::reg5::reg5_component_entries(storage_backend),
        &atlas_core::config::ProcessEnv,
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let db = Database::connect(cfg.platform.postgres.database_url.expose().clone()).await?;

    ComposedMigrator::up(&db, None).await?;
    info!("migrations applied");

    atlas_server::persistence::bootstrap::run_dev_seed(
        &atlas_server::persistence::bootstrap::BootstrapConfig {
            root_password: cfg
                .custos
                .root_password
                .as_ref()
                .map(|password| password.expose().clone()),
        },
        &db,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    info!("dev seed complete");

    Ok(())
}
