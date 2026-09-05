use anyhow::Result;
use atlas_server::persistence::migrator::ComposedMigrator;
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

    // Validate the REG-5 registry before any database connection or route is
    // opened (SHELL-REG-3). An invalid registry — e.g. a duplicate
    // `stable_id` reintroduced by a future change — must never get the
    // chance to serve traffic; the process exits non-zero here instead.
    let storage_backend =
        atlas_server::reg5::storage_backend_from_env(&atlas_core::config::ProcessEnv)
            .map_err(|e| anyhow::anyhow!("{e}"))?;

    if let Some(exit_code) = atlas_server::startup::run_registry_gate(
        atlas_server::reg5::reg5_component_entries(storage_backend),
        &mut std::io::stderr(),
    ) {
        std::process::exit(exit_code);
    }

    let cfg = match atlas_server::startup::run_config_gate(
        &atlas_server::reg5::reg5_component_entries(storage_backend),
        &atlas_core::config::ProcessEnv,
        &mut std::io::stderr(),
    ) {
        Ok(cfg) => cfg,
        Err(exit_code) => std::process::exit(exit_code),
    };

    info!("connecting to database");
    let db = atlas_postgres::connect(&cfg.platform.postgres).await?;

    info!("applying migrations");
    ComposedMigrator::up(&db, None).await?;

    info!("running bootstrap");
    atlas_server::persistence::bootstrap::run_bootstrap(
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

    let port = cfg.platform.port;

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("atlas_server listening on {addr}");

    let state = atlas_server::state::AppState::new(db, &cfg)
        .await
        .map_err(|e| anyhow::anyhow!("AppState::new: {e}"))?;

    // Wraps today's six loops as `Worker` implementations, binds them
    // against the registry's `WorkerDeclaration`s (refusing to start on any
    // drift), and starts them in `startup_order()`-then-declaration order.
    // The barrier is structural: `state` above is already a fully
    // constructed `AppState`, so no worker starts before the shared pool,
    // attachment store, and every other handle it captures exist.
    let workers = atlas_server::ops::workers::build_workers(&state, &cfg, state.workers.clone());
    let bound = match atlas_server::startup::run_worker_bind_gate(
        &state.registry,
        workers,
        &mut std::io::stderr(),
    ) {
        Ok(bound) => bound,
        Err(exit_code) => std::process::exit(exit_code),
    };
    let running_workers =
        atlas_server::ops::supervisor::start_workers(&state.registry, bound, state.workers.clone());

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

            let drain_timeout = Duration::from_secs(cfg.platform.shutdown_timeout_secs);
            match tokio::time::timeout(drain_timeout, &mut server).await {
                Ok(result) => result?,
                Err(_) => tracing::warn!(
                    timeout_secs = cfg.platform.shutdown_timeout_secs,
                    "graceful drain exceeded timeout; forcing shutdown"
                ),
            }
        }
    }

    // Cancel and join every worker in the exact reverse of its start order,
    // bounded by one global budget (E11-S3b design D4). A worker that does
    // not observe cancellation within its remaining slice is cut off and
    // reported `Failed`, rather than hanging process exit indefinitely.
    let drain_budget = Duration::from_secs(cfg.platform.shutdown_timeout_secs);
    let outcome = running_workers.drain(drain_budget).await;
    if !outcome.failed.is_empty() {
        tracing::error!(
            failed = ?outcome.failed,
            "one or more workers panicked during shutdown"
        );
    }
    if !outcome.timed_out.is_empty() {
        tracing::warn!(
            timed_out = ?outcome.timed_out,
            "one or more workers did not drain within the shutdown budget"
        );
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
