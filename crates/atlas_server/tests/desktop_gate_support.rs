#![cfg(feature = "desktop-gate-support")]

use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

#[tokio::test]
async fn test_database_lifecycle_is_available_to_desktop_gate_support()
-> Result<(), Box<dyn std::error::Error>> {
    let db = TestDb::create().await?;

    let state = atlas_server::desktop_gate_support::app_state(db.conn().clone()).await?;

    assert!(db.name().starts_with("atlas_test_"));
    assert!(!state.cookie_secure);

    db.teardown().await?;

    Ok(())
}

#[tokio::test]
async fn partial_fixture_applies_remaining_migrations() -> Result<(), Box<dyn std::error::Error>> {
    let db = TestDb::create_with_migration_steps(Some(1)).await?;

    db.run_remaining_migrations().await?;

    let row = db
        .conn()
        .query_one_raw(Statement::from_string(
            DatabaseBackend::Postgres,
            "SELECT to_regclass('public.users') IS NOT NULL AS users_table_exists",
        ))
        .await?
        .ok_or("expected migration query row")?;
    let users_table_exists: bool = row.try_get("", "users_table_exists")?;

    assert!(users_table_exists);

    db.teardown().await?;

    Ok(())
}
