use anyhow::Result;
use migration::Migrator;
use sea_orm::Database;
use sea_orm_migration::prelude::MigratorTrait;
use std::net::SocketAddr;
use tokio::sync::watch;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,atlas_server=debug,tower_http=info".into()),
        )
        .with_target(true)
        .init();

    let cfg = atlas_server::config::ServerConfig::from_env().map_err(|e| anyhow::anyhow!("{e}"))?;

    info!("connecting to database");
    let db = Database::connect(&cfg.database_url).await?;

    info!("applying migrations");
    Migrator::up(&db, None).await?;

    info!("running bootstrap");
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

    let state = atlas_server::state::AppState::new(db.clone(), &cfg)
        .await
        .map_err(|e| anyhow::anyhow!("AppState::new: {e}"))?;

    // Spawn the webhook dispatcher as a background task.
    //
    // A watch channel carries the shutdown signal: the main task sends `true`
    // after axum::serve returns, then awaits the dispatcher handle so any
    // in-flight deliveries drain before the process exits.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let dispatcher = atlas_server::dispatcher::WebhookDispatcher::new(
        db,
        state.webhook_crypto.clone(),
        state.dispatcher_config.clone(),
    );
    let dispatcher_handle = tokio::spawn(dispatcher.run(shutdown_rx));

    axum::serve(
        listener,
        atlas_server::app(state).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await?;

    // Signal the dispatcher and await its clean exit.
    let _ = shutdown_tx.send(true);
    if let Err(e) = dispatcher_handle.await {
        tracing::error!(error = %e, "dispatcher task panicked during shutdown");
    }

    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
