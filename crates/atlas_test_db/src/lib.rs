//! Ephemeral Postgres fixtures shared by integration tests.

use migration::Migrator;
use sea_orm::{ConnectOptions, ConnectionTrait, Database, DatabaseConnection, DbErr};
use sea_orm_migration::prelude::MigratorTrait;
use uuid::Uuid;

/// A migrated database with a generated name and best-effort forced teardown.
pub struct TestDb {
    conn: DatabaseConnection,
    db_name: String,
    admin_url: String,
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy)]
enum FixtureFailure {
    TestConnection,
    Migration,
    TestConnectionAndCleanup,
    Teardown,
}

impl TestDb {
    /// Creates a fresh migrated database for one integration test.
    pub async fn create() -> Result<Self, DbErr> {
        Self::create_with_migration_steps(None).await
    }

    /// Creates a fresh database and applies only the requested migration prefix.
    pub async fn create_with_migration_steps(steps: Option<u32>) -> Result<Self, DbErr> {
        let database_url = std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://atlas:atlas@localhost:5432/atlas_dev".to_string());

        let db_name = format!("atlas_test_{}", Uuid::now_v7().as_simple());

        create_with_migration_steps_named(&database_url, &db_name, steps, None).await
    }

    /// Applies any migrations that are still pending for this fixture.
    pub async fn run_remaining_migrations(&self) -> Result<(), DbErr> {
        Migrator::up(&self.conn, None).await
    }

    /// Returns the test database connection.
    pub fn conn(&self) -> &DatabaseConnection {
        &self.conn
    }

    /// Returns the generated database name for fixture assertions.
    pub fn name(&self) -> &str {
        &self.db_name
    }

    /// Drops this fixture database, disconnecting active clients first.
    pub async fn teardown(self) -> Result<(), DbErr> {
        drop(self.conn);

        force_drop_database_with_failure(&self.admin_url, &self.db_name, None).await
    }

    #[cfg(test)]
    async fn teardown_with_failure(self, failure: FixtureFailure) -> Result<(), DbErr> {
        drop(self.conn);

        force_drop_database_with_failure(&self.admin_url, &self.db_name, Some(failure)).await
    }
}

async fn create_with_migration_steps_named(
    database_url: &str,
    db_name: &str,
    steps: Option<u32>,
    failure: Option<FixtureFailure>,
) -> Result<TestDb, DbErr> {
    let admin_url = admin_url_from(database_url);
    let admin = Database::connect(admin_opts(&admin_url)).await?;
    admin
        .execute_unprepared(&format!("CREATE DATABASE \"{db_name}\""))
        .await?;
    drop(admin);

    let test_url = replace_db_name(database_url, db_name);
    let conn = match injected_failure(failure, FixtureFailure::TestConnection) {
        Some(error) => return Err(cleanup_after_failure(&admin_url, db_name, error, failure).await),
        None => Database::connect(ConnectOptions::new(test_url)).await,
    };
    let conn = match conn {
        Ok(conn) => conn,
        Err(error) => return Err(cleanup_after_failure(&admin_url, db_name, error, failure).await),
    };

    let migration = match injected_failure(failure, FixtureFailure::Migration) {
        Some(error) => Err(error),
        None => Migrator::up(&conn, steps).await,
    };
    if let Err(error) = migration {
        drop(conn);

        return Err(cleanup_after_failure(&admin_url, db_name, error, failure).await);
    }

    Ok(TestDb {
        conn,
        db_name: db_name.to_owned(),
        admin_url,
    })
}

async fn cleanup_after_failure(
    admin_url: &str,
    db_name: &str,
    original: DbErr,
    failure: Option<FixtureFailure>,
) -> DbErr {
    let cleanup_failure = matches!(failure, Some(FixtureFailure::TestConnectionAndCleanup))
        .then_some(FixtureFailure::Teardown);

    match force_drop_database_with_failure(admin_url, db_name, cleanup_failure).await {
        Ok(()) => original,
        Err(cleanup) => DbErr::Custom(format!(
            "fixture setup failed: {original}; forced cleanup also failed: {cleanup}"
        )),
    }
}

async fn force_drop_database_with_failure(
    admin_url: &str,
    db_name: &str,
    failure: Option<FixtureFailure>,
) -> Result<(), DbErr> {
    if let Some(error) = injected_failure(failure, FixtureFailure::Teardown) {
        return Err(error);
    }

    let admin = Database::connect(admin_opts(admin_url)).await?;
    admin
        .execute_unprepared(&format!(
            "DROP DATABASE IF EXISTS \"{db_name}\" WITH (FORCE)"
        ))
        .await
        .map(|_| ())
}

fn injected_failure(failure: Option<FixtureFailure>, expected: FixtureFailure) -> Option<DbErr> {
    match (failure, expected) {
        (Some(FixtureFailure::TestConnection), FixtureFailure::TestConnection)
        | (Some(FixtureFailure::TestConnectionAndCleanup), FixtureFailure::TestConnection) => {
            Some(DbErr::Custom("injected test connection failure".to_owned()))
        }
        (Some(FixtureFailure::Migration), FixtureFailure::Migration) => {
            Some(DbErr::Custom("injected migration failure".to_owned()))
        }
        (Some(FixtureFailure::Teardown), FixtureFailure::Teardown) => {
            Some(DbErr::Custom("injected teardown failure".to_owned()))
        }
        _ => None,
    }
}

fn admin_opts(url: &str) -> ConnectOptions {
    let mut opts = ConnectOptions::new(url.to_owned());
    opts.max_connections(1).min_connections(0);
    opts
}

fn admin_url_from(url: &str) -> String {
    replace_db_name(url, "postgres")
}

fn replace_db_name(url: &str, new_db: &str) -> String {
    if let Some(slash_pos) = url.rfind('/') {
        let base = &url[..=slash_pos];
        let rest = &url[slash_pos + 1..];
        let db_only = rest.split('?').next().unwrap_or(rest);
        let query = if rest.contains('?') {
            &rest[db_only.len()..]
        } else {
            ""
        };
        format!("{base}{new_db}{query}")
    } else {
        url.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sea_orm::{DatabaseBackend, Statement};

    #[tokio::test]
    async fn connection_failure_after_create_forces_cleanup() -> Result<(), DbErr> {
        let database_url = test_database_url();
        let db_name = test_database_name();

        let error = create_with_migration_steps_named(
            &database_url,
            &db_name,
            None,
            Some(FixtureFailure::TestConnection),
        )
        .await
        .err()
        .ok_or_else(|| {
            DbErr::Custom("an injected test connection failure must be returned".to_owned())
        })?;

        assert!(
            error
                .to_string()
                .contains("injected test connection failure")
        );
        assert!(!database_exists(&database_url, &db_name).await?);

        Ok(())
    }

    #[tokio::test]
    async fn migration_failure_after_create_forces_cleanup() -> Result<(), DbErr> {
        let database_url = test_database_url();
        let db_name = test_database_name();

        let error = create_with_migration_steps_named(
            &database_url,
            &db_name,
            None,
            Some(FixtureFailure::Migration),
        )
        .await
        .err()
        .ok_or_else(|| {
            DbErr::Custom("an injected migration failure must be returned".to_owned())
        })?;

        assert!(error.to_string().contains("injected migration failure"));
        assert!(!database_exists(&database_url, &db_name).await?);

        Ok(())
    }

    #[tokio::test]
    async fn cleanup_failure_retains_original_and_cleanup_error_context() -> Result<(), DbErr> {
        let database_url = test_database_url();
        let db_name = test_database_name();

        let error = create_with_migration_steps_named(
            &database_url,
            &db_name,
            None,
            Some(FixtureFailure::TestConnectionAndCleanup),
        )
        .await
        .err()
        .ok_or_else(|| {
            DbErr::Custom(
                "an original failure and cleanup failure must be returned together".to_owned(),
            )
        })?;

        let error_message = error.to_string();
        assert!(error_message.contains("injected test connection failure"));
        assert!(error_message.contains("injected teardown failure"));

        force_drop_database_with_failure(&admin_url_from(&database_url), &db_name, None).await?;

        Ok(())
    }

    #[tokio::test]
    async fn teardown_failure_is_returned_to_the_caller() -> Result<(), DbErr> {
        let database_url = test_database_url();
        let db = TestDb::create().await?;
        let db_name = db.name().to_owned();

        let error = db
            .teardown_with_failure(FixtureFailure::Teardown)
            .await
            .err()
            .ok_or_else(|| {
                DbErr::Custom("an injected teardown failure must be returned".to_owned())
            })?;

        assert!(error.to_string().contains("injected teardown failure"));
        force_drop_database_with_failure(&admin_url_from(&database_url), &db_name, None).await?;
        assert!(!database_exists(&database_url, &db_name).await?);

        Ok(())
    }

    fn test_database_url() -> String {
        std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://atlas:atlas@localhost:5432/atlas_dev".to_owned())
    }

    fn test_database_name() -> String {
        format!("atlas_test_{}", Uuid::now_v7().as_simple())
    }

    async fn database_exists(database_url: &str, db_name: &str) -> Result<bool, DbErr> {
        let admin = Database::connect(admin_opts(&admin_url_from(database_url))).await?;
        let row = admin
            .query_one_raw(Statement::from_string(
                DatabaseBackend::Postgres,
                format!(
                    "SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = '{db_name}') AS database_exists"
                ),
            ))
            .await?
            .ok_or_else(|| DbErr::Custom("database existence query returned no row".to_owned()))?;

        row.try_get("", "database_exists")
    }
}
