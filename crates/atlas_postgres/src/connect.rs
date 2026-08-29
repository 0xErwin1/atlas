use std::time::Duration;

use sea_orm::{ConnectOptions, Database, DatabaseConnection, DbErr};

use crate::config::PostgresConfig;

/// Builds the `sea_orm::ConnectOptions` for `config`, applying the pool
/// bounds and the debug-level SQL logging that `atlas_server::main` used to
/// configure inline.
pub fn connect_options(config: &PostgresConfig) -> ConnectOptions {
    let mut db_opts = ConnectOptions::new(config.database_url.expose().clone());

    // Log SQL queries at DEBUG, not the sea-orm default of INFO: the webhook
    // dispatcher polls the outbox every second, so at INFO the poll's UPDATEs
    // flood the logs with `sqlx::query` lines even when there is no work. They
    // stay available under a `sqlx=debug` filter for query-level debugging.
    db_opts
        .max_connections(config.pool.max_connections)
        .min_connections(config.pool.min_connections)
        .acquire_timeout(Duration::from_secs(config.pool.acquire_timeout_secs))
        .sqlx_logging_level(log::LevelFilter::Debug);

    db_opts
}

/// Opens a pooled `DatabaseConnection` for `config`.
pub async fn connect(config: &PostgresConfig) -> Result<DatabaseConnection, DbErr> {
    Database::connect(connect_options(config)).await
}
