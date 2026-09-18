//! E4-S1 W4 — characterization tests for the `custos.principals` mirror
//! written by the Custos identity adapters. Every user/api-key write path
//! must move the principal row in the same transaction as its base row:
//!
//! - users: `principals.display_name == users.display_name` and
//!   `principals.deactivated_at == users.disabled_at`;
//! - api keys: an `agent` principal exists whose `deactivated_at` tracks
//!   `api_keys.revoked_at` and whose `display_name` is the key name.
//!
//! These tests go through the repository adapters (`PgUserRepo` /
//! `PgApiKeyRepo`), which own the sync; the routes never write principals.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_core::Attribution;
use atlas_core::attribution::UserAttributionId;
use atlas_core::principal::UserId;
use atlas_custos::WorkspaceScope;
use atlas_custos::capability::Capability;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos::ids::ApiKeyId;
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey, NewUser, UserRepo};
use sea_orm::{DatabaseBackend, FromQueryResult, Statement, TransactionTrait};

type StdResult<T> = Result<T, Box<dyn std::error::Error>>;

fn new_user(username: &str) -> NewUser {
    NewUser {
        username: username.to_string(),
        display_name: format!("Display {username}"),
        email: None,
        password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
        is_root: false,
        is_system_admin: false,
    }
}

fn new_key(name: &str) -> NewApiKey {
    NewApiKey {
        name: name.to_string(),
        token_hash: format!("hash-{name}"),
        type_: ApiKeyType::Agent,
        expires_at: None,
        scopes: vec!["docs:read".parse::<Capability>().expect("valid capability")],
    }
}

#[derive(FromQueryResult, Debug)]
struct UserMirrorRow {
    username: String,
    display_name: String,
    disabled_at: Option<chrono::DateTime<chrono::Utc>>,
    principal_kind: String,
    principal_display_name: String,
    principal_deactivated_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn user_mirror(db: &support::TestDb, user_id: UserId) -> Option<UserMirrorRow> {
    UserMirrorRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT u.username, u.display_name, u.disabled_at, \
                p.kind AS principal_kind, p.display_name AS principal_display_name, \
                p.deactivated_at AS principal_deactivated_at \
         FROM custos.users u \
         JOIN custos.principals p ON p.id = u.principal_id \
         WHERE u.id = $1",
        [user_id.0.into()],
    ))
    .one(db.conn())
    .await
    .expect("query user/principal mirror")
}

#[derive(FromQueryResult, Debug)]
struct KeyMirrorRow {
    name: String,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
    principal_kind: String,
    principal_display_name: String,
    principal_deactivated_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn key_mirror(db: &support::TestDb, key_id: ApiKeyId) -> Option<KeyMirrorRow> {
    KeyMirrorRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT k.name, k.revoked_at, \
                p.kind AS principal_kind, p.display_name AS principal_display_name, \
                p.deactivated_at AS principal_deactivated_at \
         FROM custos.api_keys k \
         JOIN custos.principals p ON p.id = k.principal_id \
         WHERE k.id = $1",
        [key_id.0.into()],
    ))
    .one(db.conn())
    .await
    .expect("query api key/principal mirror")
}

#[derive(FromQueryResult, Debug)]
struct KeyPrincipalLinkRow {
    principal_kind: String,
    links_owner_principal: bool,
}

fn assert_user_mirror_matches(row: &UserMirrorRow) {
    assert_eq!(row.principal_kind, "user");
    assert_eq!(
        row.principal_display_name, row.display_name,
        "principals.display_name must equal users.display_name"
    );
    assert_eq!(
        row.principal_deactivated_at, row.disabled_at,
        "principals.deactivated_at must equal users.disabled_at"
    );
}

async fn key_principal_link(db: &support::TestDb, key_id: ApiKeyId) -> Option<KeyPrincipalLinkRow> {
    KeyPrincipalLinkRow::find_by_statement(Statement::from_sql_and_values(
        DatabaseBackend::Postgres,
        "SELECT p.kind AS principal_kind, (p.id = u.principal_id) AS links_owner_principal \
         FROM custos.api_keys k \
         JOIN custos.principals p ON p.id = k.principal_id \
         JOIN custos.users u ON u.id = k.created_by_user_id \
         WHERE k.id = $1",
        [key_id.0.into()],
    ))
    .one(db.conn())
    .await
    .expect("query api key principal link")
}

fn assert_key_mirror_matches(row: &KeyMirrorRow) {
    assert_eq!(row.principal_kind, "agent");
    assert_eq!(
        row.principal_display_name, row.name,
        "the agent principal's display_name must equal the key name"
    );
    assert_eq!(
        row.principal_deactivated_at, row.revoked_at,
        "the agent principal's deactivated_at must track api_keys.revoked_at"
    );
}

// ── PgUserRepo ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn user_create_mirrors_the_principal_row() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let created = db.user_repo().create(new_user("mirror-create")).await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after create");
    assert_eq!(row.username, "mirror-create");
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn user_disable_mirrors_disabled_at_into_the_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let repo = db.user_repo();
    let created = repo.create(new_user("mirror-disable")).await?;

    repo.disable(created.id).await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after disable");
    assert!(
        row.disabled_at.is_some(),
        "disable must set users.disabled_at"
    );
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn user_enable_clears_the_principal_mirror_again() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let repo = db.user_repo();
    let created = repo.create(new_user("mirror-enable")).await?;

    repo.disable(created.id).await?;
    repo.enable(created.id).await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after enable");
    assert!(row.disabled_at.is_none(), "enable must clear disabled_at");
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn user_disable_in_mirrors_disabled_at_inside_the_callers_transaction() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let created = db.user_repo().create(new_user("mirror-disable-in")).await?;

    let txn = db.conn().begin().await?;
    atlas_custos_postgres::repos::identity::PgUserRepo::disable_in(&txn, created.id).await?;
    txn.commit().await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after disable_in");
    assert!(row.disabled_at.is_some());
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn user_enable_in_clears_the_principal_mirror_inside_the_callers_transaction() -> StdResult<()>
{
    let db = support::TestDb::create().await?;
    let repo = db.user_repo();
    let created = repo.create(new_user("mirror-enable-in")).await?;
    repo.disable(created.id).await?;

    let txn = db.conn().begin().await?;
    atlas_custos_postgres::repos::identity::PgUserRepo::enable_in(&txn, created.id).await?;
    txn.commit().await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after enable_in");
    assert!(row.disabled_at.is_none());
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn user_update_profile_mirrors_the_new_display_name() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let repo = db.user_repo();
    let created = repo.create(new_user("mirror-profile")).await?;

    repo.update_profile(created.id, None, Some("Renamed Person".to_string()))
        .await?;

    let row = user_mirror(&db, created.id)
        .await
        .expect("user principal must exist after update_profile");
    assert_eq!(row.display_name, "Renamed Person");
    assert_user_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

// ── credential kind (E4-S2C) ────────────────────────────────────────────────

#[tokio::test]
async fn personal_key_links_to_the_owners_user_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db
        .user_repo()
        .create(new_user("personal-key-owner"))
        .await?;

    let txn = db.conn().begin().await?;
    let key = atlas_custos_postgres::repos::identity::PgApiKeyRepo::create_for_user_in_with_kind(
        &txn,
        user.id,
        atlas_custos::entities::identity::ApiKeyKind::Personal,
        new_key("personal-key"),
    )
    .await?;
    txn.commit().await?;

    let row = key_principal_link(&db, key.id)
        .await
        .expect("principal row must exist for a personal key");
    assert_eq!(row.principal_kind, "user");
    assert!(
        row.links_owner_principal,
        "a personal key must link to the owner's user principal, not a fresh agent principal"
    );

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn agent_key_keeps_a_fresh_agent_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db.user_repo().create(new_user("agent-key-owner")).await?;

    let txn = db.conn().begin().await?;
    let key = atlas_custos_postgres::repos::identity::PgApiKeyRepo::create_for_user_in_with_kind(
        &txn,
        user.id,
        atlas_custos::entities::identity::ApiKeyKind::Agent,
        new_key("agent-key"),
    )
    .await?;
    txn.commit().await?;

    let row = key_principal_link(&db, key.id)
        .await
        .expect("principal row must exist for an agent key");
    assert_eq!(row.principal_kind, "agent");
    assert!(
        !row.links_owner_principal,
        "an agent key must link to a fresh agent principal, not the owner's user principal"
    );

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn personal_key_creation_rejects_a_missing_owner_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    // A user id whose principal row does not exist: personal creation must be
    // fail-closed instead of dangling the FK or silently minting an agent
    // principal.
    let ghost = UserId::new();

    let txn = db.conn().begin().await?;
    let result =
        atlas_custos_postgres::repos::identity::PgApiKeyRepo::create_for_user_in_with_kind(
            &txn,
            ghost,
            atlas_custos::entities::identity::ApiKeyKind::Personal,
            new_key("ghost-owner-key"),
        )
        .await;
    txn.rollback().await?;

    assert!(result.is_err(), "missing owner principal must be rejected");

    db.teardown().await;
    Ok(())
}

// ── PgApiKeyRepo ────────────────────────────────────────────────────────────

#[tokio::test]
async fn api_key_create_scopes_an_agent_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db.user_repo().create(new_user("key-owner-create")).await?;
    let repo = db.api_key_repo();

    let key = repo
        .create(
            WorkspaceScope(uuid::Uuid::now_v7()),
            &Attribution::User(UserAttributionId(user.id.0)),
            new_key("scoped-key"),
        )
        .await?;

    let row = key_mirror(&db, key.id)
        .await
        .expect("agent principal must exist after create");
    assert_key_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn api_key_create_for_user_scopes_an_agent_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db
        .user_repo()
        .create(new_user("key-owner-for-user"))
        .await?;
    let repo = db.api_key_repo();

    let key = repo.create_for_user(user.id, new_key("user-key")).await?;

    let row = key_mirror(&db, key.id)
        .await
        .expect("agent principal must exist after create_for_user");
    assert_key_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn api_key_create_for_user_in_scopes_an_agent_principal_inside_the_callers_transaction()
-> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db
        .user_repo()
        .create(new_user("key-owner-for-user-in"))
        .await?;

    let txn = db.conn().begin().await?;
    let key = atlas_custos_postgres::repos::identity::PgApiKeyRepo::create_for_user_in(
        &txn,
        user.id,
        new_key("user-key-in"),
    )
    .await?;
    txn.commit().await?;

    let row = key_mirror(&db, key.id)
        .await
        .expect("agent principal must exist after create_for_user_in");
    assert_key_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}

#[tokio::test]
async fn api_key_revocation_tracks_revoked_at_in_the_agent_principal() -> StdResult<()> {
    let db = support::TestDb::create().await?;
    let user = db.user_repo().create(new_user("key-owner-revoke")).await?;
    let key = db
        .api_key_repo()
        .create_for_user(user.id, new_key("revoked-key"))
        .await?;

    let txn = db.conn().begin().await?;
    atlas_custos_postgres::repos::identity::PgApiKeyRepo::revoke_for_user_in(&txn, user.id, key.id)
        .await?;
    txn.commit().await?;

    let row = key_mirror(&db, key.id)
        .await
        .expect("agent principal must exist after revoke");
    assert!(
        row.revoked_at.is_some(),
        "revoke must set api_keys.revoked_at"
    );
    assert_key_mirror_matches(&row);

    db.teardown().await;
    Ok(())
}
