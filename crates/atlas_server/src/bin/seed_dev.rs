use anyhow::Result;
use migration::Migrator;
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

    let cfg = atlas_server::config::ServerConfig::from_env().map_err(|e| anyhow::anyhow!("{e}"))?;

    let db = Database::connect(&cfg.database_url).await?;

    Migrator::up(&db, None).await?;
    info!("migrations applied");

    atlas_server::persistence::bootstrap::run_dev_seed(
        &atlas_server::persistence::bootstrap::BootstrapConfig {
            root_password: cfg.root_password.clone(),
        },
        &db,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    info!("dev seed complete");

    Ok(())
}
