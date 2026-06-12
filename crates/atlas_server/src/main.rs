use anyhow::Result;
use migration::Migrator;
use sea_orm::Database;
use sea_orm_migration::prelude::MigratorTrait;
use std::net::SocketAddr;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "atlas_server=info".into()),
        )
        .init();

    let cfg = atlas_server::config::ServerConfig::from_env().map_err(|e| anyhow::anyhow!("{e}"))?;

    let db = Database::connect(&cfg.database_url).await?;

    Migrator::up(&db, None).await?;

    atlas_server::persistence::bootstrap::run_bootstrap(
        &atlas_server::persistence::bootstrap::BootstrapConfig {
            root_password: cfg.root_password.clone(),
        },
        &db,
    )
    .await
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    let port: u16 = std::env::var("ATLAS_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8080);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("atlas_server listening on {addr}");

    let state = atlas_server::state::AppState::new(db);
    axum::serve(
        listener,
        atlas_server::app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
