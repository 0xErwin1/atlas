use anyhow::Result;
use migration::Migrator;
use sea_orm::{ConnectOptions, Database};
use sea_orm_migration::prelude::MigratorTrait;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::time::Duration;
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
    // Log SQL queries at DEBUG, not the sea-orm default of INFO: the webhook
    // dispatcher polls the outbox every second, so at INFO the poll's UPDATEs
    // flood the logs with `sqlx::query` lines even when there is no work. They
    // stay available under a `sqlx=debug` filter for query-level debugging.
    let mut db_opts = ConnectOptions::new(cfg.database_url.clone());
    db_opts
        .max_connections(cfg.db_pool.max_connections)
        .min_connections(cfg.db_pool.min_connections)
        .acquire_timeout(Duration::from_secs(cfg.db_pool.acquire_timeout_secs))
        .sqlx_logging_level(log::LevelFilter::Debug);
    let db = Database::connect(db_opts).await?;

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
        state.allow_private_webhook_targets,
    );
    let dispatcher_handle = tokio::spawn(dispatcher.run(shutdown_rx.clone()));

    let attachment_reconciler_handle = tokio::spawn(
        atlas_server::persistence::repos::PgAttachmentLifecycle::run_reconciler(
            (*state.db).clone(),
            state.attachments.clone(),
            shutdown_rx.clone(),
        ),
    );

    // Spawn the Postgres LISTEN consumer that feeds the in-process live-event
    // hub. It shares the same watch-based shutdown signal as the dispatcher and
    // is drained on graceful shutdown alongside it.
    let live_pool = state.db.get_postgres_connection_pool().clone();
    let listener_handle = tokio::spawn(atlas_server::live::run_listener(
        live_pool,
        state.live.clone(),
        shutdown_rx.clone(),
    ));

    // Spawn the presence background tasks: the TTL sweeper that expires stale
    // presence entries, and the agent-activity consumer that marks an api-key
    // principal present while it is mutating a board. Both share the same
    // watch-based shutdown signal and are drained on graceful shutdown.
    let sweeper_handle = tokio::spawn(atlas_server::presence::run_presence_sweeper(
        state.clone(),
        shutdown_rx.clone(),
    ));
    let presence_agent_handle = tokio::spawn(atlas_server::presence::run_presence_agent_consumer(
        state.clone(),
        shutdown_rx,
    ));

    let make_service = atlas_server::app(state).into_make_service_with_connect_info::<SocketAddr>();

    // A watch flag drives axum's graceful shutdown: it flips to `true` on the
    // first OS signal so the server stops accepting and starts draining.
    let (drain_tx, drain_rx) = watch::channel(false);

    let mut serve_drain_rx = drain_rx.clone();
    let server = axum::serve(listener, make_service)
        .with_graceful_shutdown(async move {
            let _ = serve_drain_rx.wait_for(|drained| *drained).await;
        })
        .into_future();
    tokio::pin!(server);

    let signal = shutdown_signal();
    tokio::pin!(signal);

    // Serve until the first shutdown signal (or an early server error). On
    // signal, flip the drain flag and bound the drain with a timeout so
    // long-lived SSE streams cannot block process termination indefinitely.
    tokio::select! {
        result = &mut server => result?,
        _ = &mut signal => {
            info!("shutdown signal received; draining connections");
            let _ = drain_tx.send(true);

            let drain_timeout = Duration::from_secs(cfg.shutdown_timeout_secs);
            match tokio::time::timeout(drain_timeout, &mut server).await {
                Ok(result) => result?,
                Err(_) => tracing::warn!(
                    timeout_secs = cfg.shutdown_timeout_secs,
                    "graceful drain exceeded timeout; forcing shutdown"
                ),
            }
        }
    }

    // Signal the background tasks and await their clean exit.
    let _ = shutdown_tx.send(true);
    if let Err(e) = dispatcher_handle.await {
        tracing::error!(error = %e, "dispatcher task panicked during shutdown");
    }
    if let Err(e) = attachment_reconciler_handle.await {
        tracing::error!(error = %e, "attachment reconciler task panicked during shutdown");
    }
    if let Err(e) = listener_handle.await {
        tracing::error!(error = %e, "live event listener task panicked during shutdown");
    }
    if let Err(e) = sweeper_handle.await {
        tracing::error!(error = %e, "presence sweeper task panicked during shutdown");
    }
    if let Err(e) = presence_agent_handle.await {
        tracing::error!(error = %e, "presence agent consumer task panicked during shutdown");
    }

    Ok(())
}

/// Resolves on the first process shutdown signal.
///
/// Awaits `Ctrl-C` on every platform and, on Unix, also `SIGTERM` so that
/// `docker stop` and Kubernetes pod termination (which send `SIGTERM`) trigger
/// the same graceful drain rather than being killed on the fallback timeout.
/// Whichever signal arrives first wins.
async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut term) => {
                term.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to install SIGTERM handler; relying on Ctrl-C");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
