#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_server::persistence::{
    entities::{identity::api_key, integration_config::integration_configs},
    repos::PgIntegrationConfigRepo,
};
use sea_orm::{EntityTrait, TransactionTrait};

// ---------------------------------------------------------------------------
// create provisions an Integration api_key and links it to the config
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_provisions_integration_api_key() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-create").await;

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0xAA; 32],
        vec![0xBB; 12],
        user.id.0,
    )
    .await
    .expect("create config");
    txn.commit().await.expect("commit");

    assert_eq!(config.workspace_id, ws.id.0);
    assert_eq!(config.integration, "github");
    assert!(config.deleted_at.is_none());

    let key = api_key::Entity::find_by_id(config.integration_api_key_id)
        .one(db.conn())
        .await
        .expect("find api_key")
        .expect("api_key must exist");

    assert_eq!(key.type_, "integration", "provisioned key must have type 'integration'");
    assert_eq!(
        key.created_by_user_id, user.id.0,
        "key must be attributed to the creating user"
    );
    assert!(key.revoked_at.is_none(), "newly provisioned key must not be revoked");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// find_active returns the active config for a workspace + integration pair
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_active_returns_active_config() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-find-active").await;

    let txn = db.conn().begin().await.expect("begin");
    let created = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0x01; 32],
        vec![0x02; 12],
        user.id.0,
    )
    .await
    .expect("create");
    txn.commit().await.expect("commit");

    let found = PgIntegrationConfigRepo::find_active(db.conn(), ws.id.0, "github")
        .await
        .expect("find_active");

    let found = found.expect("must return Some");
    assert_eq!(found.id, created.id);
    assert_eq!(found.integration, "github");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// find_active returns None after soft-delete
// ---------------------------------------------------------------------------

#[tokio::test]
async fn find_active_returns_none_after_soft_delete() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-find-none").await;

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0x03; 32],
        vec![0x04; 12],
        user.id.0,
    )
    .await
    .expect("create");
    txn.commit().await.expect("commit");

    let txn2 = db.conn().begin().await.expect("begin2");
    PgIntegrationConfigRepo::soft_delete_and_revoke_key(&txn2, ws.id.0, config.id)
        .await
        .expect("soft_delete");
    txn2.commit().await.expect("commit2");

    let after = PgIntegrationConfigRepo::find_active(db.conn(), ws.id.0, "github")
        .await
        .expect("find_active after delete");

    assert!(after.is_none(), "find_active must return None after soft-delete");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// soft_delete_and_revoke_key sets revoked_at on the provisioned api key
// ---------------------------------------------------------------------------

#[tokio::test]
async fn soft_delete_and_revoke_key_sets_revoked_at() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-revoke").await;

    let txn = db.conn().begin().await.expect("begin");
    let config = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0x05; 32],
        vec![0x06; 12],
        user.id.0,
    )
    .await
    .expect("create");
    txn.commit().await.expect("commit");

    let api_key_id = config.integration_api_key_id;

    let txn2 = db.conn().begin().await.expect("begin2");
    PgIntegrationConfigRepo::soft_delete_and_revoke_key(&txn2, ws.id.0, config.id)
        .await
        .expect("soft_delete");
    txn2.commit().await.expect("commit2");

    let revoked_key = api_key::Entity::find_by_id(api_key_id)
        .one(db.conn())
        .await
        .expect("find api_key")
        .expect("key must still exist (ON DELETE RESTRICT)");

    assert!(
        revoked_key.revoked_at.is_some(),
        "api_key.revoked_at must be set after soft_delete_and_revoke_key"
    );

    let deleted_config = integration_configs::Entity::find_by_id(config.id)
        .one(db.conn())
        .await
        .expect("find config")
        .expect("config row must still exist");

    assert!(
        deleted_config.deleted_at.is_some(),
        "integration_config.deleted_at must be set"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// list returns all non-deleted configs for a workspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn list_returns_active_configs() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-list").await;

    let txn = db.conn().begin().await.expect("begin");
    PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0x07; 32],
        vec![0x08; 12],
        user.id.0,
    )
    .await
    .expect("create");
    txn.commit().await.expect("commit");

    let configs = PgIntegrationConfigRepo::list(db.conn(), ws.id.0)
        .await
        .expect("list");

    assert_eq!(configs.len(), 1);
    assert_eq!(configs[0].integration, "github");

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// get_by_id returns the config by its UUID within the workspace
// ---------------------------------------------------------------------------

#[tokio::test]
async fn get_by_id_returns_config() {
    let db = support::TestDb::create().await.expect("TestDb");
    let (ws, user) = support::seed_workspace(&db, "ic-get").await;

    let txn = db.conn().begin().await.expect("begin");
    let created = PgIntegrationConfigRepo::create(
        &txn,
        ws.id.0,
        "github".to_string(),
        vec![0x09; 32],
        vec![0x0A; 12],
        user.id.0,
    )
    .await
    .expect("create");
    txn.commit().await.expect("commit");

    let found = PgIntegrationConfigRepo::get_by_id(db.conn(), ws.id.0, created.id)
        .await
        .expect("get_by_id")
        .expect("must return Some");

    assert_eq!(found.id, created.id);

    db.teardown().await;
}
