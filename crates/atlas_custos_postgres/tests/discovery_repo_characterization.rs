//! Container-backed characterization tests for `PgDiscoveryRepo` (E11-S8
//! PR1). Runs against a disposable Postgres named by
//! `ATLAS_TEST_DATABASE_URL` (see `atlas_test_db`); compile-only where no
//! such database is reachable (`cargo test -p atlas_custos_postgres --no-run`).
//!
//! Covers the scenarios design D3/D7 name for this adapter: a direct user
//! grant, a group-derived grant (including one via a soft-deleted group,
//! which must be excluded), and an api-key grant looked up by its own id.
//! The membership-derived union (D-S8-9) and the admin short-circuit (D4)
//! are `atlas_server` handler concerns, out of scope for this adapter-level
//! suite.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use atlas_core::principal::UserId;
use atlas_custos::WorkspaceScope;
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::{ApiKeyType, NewApiKey, NewUser};
use atlas_custos::entities::permissions::{NewPermissionGrant, ResourceRole};
use atlas_custos::ports::discovery::{DiscoveryPort, DiscoveryPrincipal};
use atlas_custos::ports::grant_repo::PermissionGrantRepo as PermissionGrantRepoTrait;
use atlas_custos::ports::group_repo::GroupRepo as GroupRepoTrait;
use atlas_custos::ports::identity::{ApiKeyRepo as ApiKeyRepoTrait, UserRepo as UserRepoTrait};
use atlas_custos_postgres::repos::discovery::PgDiscoveryRepo;
use atlas_custos_postgres::repos::identity::{PgApiKeyRepo, PgUserRepo};
use atlas_custos_postgres::repos::permissions::{PgGroupRepo, PgPermissionGrantRepo};
use atlas_test_db::TestDb;
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// The number of migrations Custos owns plus the historical block, i.e. the
/// prefix of `historical() ++ custos_new() ++ acta_new()` that ends right
/// after `custos_new()`. Computed rather than hardcoded so it tracks
/// whichever migrations actually exist.
///
/// This test suite only needs Custos-owned tables (`users`, `groups`,
/// `group_members`, `permission_grants`) and deliberately stops before
/// `acta_new()`: `m20260905_000057_acta_search_attachments_lifecycle_set_schema`
/// unconditionally moves `search_embeddings` (created only when the `vector`
/// extension is available, `m20260708_000039_search_embeddings.rs`), so a
/// Postgres without `pgvector` installed cannot run past `acta_new()` at
/// all — a pre-existing gap unrelated to Custos, reproduced identically by
/// `atlas_acta_postgres`'s own container tests on the same database.
fn custos_only_migration_steps() -> u32 {
    let historical = migration::Migrator::migrations().len();
    let custos = atlas_custos_postgres::migrations::custos_new().len();
    (historical + custos) as u32
}

async fn create_custos_only_db() -> TestDb {
    TestDb::create_with_migration_steps(Some(custos_only_migration_steps()))
        .await
        .expect("TestDb::create_with_migration_steps")
}

async fn seed_user(db: &TestDb, username: &str) -> UserId {
    let repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    repo.create(NewUser {
        username: username.to_string(),
        display_name: username.to_string(),
        email: None,
        password_hash: None,
        is_root: false,
        is_system_admin: false,
    })
    .await
    .expect("seed user")
    .id
}

#[tokio::test]
async fn a_direct_user_grant_is_discovered() {
    let db = create_custos_only_db().await;
    let user_id = seed_user(&db, "alice").await;
    let workspace_id = WorkspaceScope(Uuid::now_v7());

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id,
            user_id: Some(user_id),
            api_key_id: None,
            group_id: None,
            resource_ref: "acta::workspace::direct-ws".parse().unwrap(),
            role: ResourceRole::Viewer,
            created_by_user_id: Some(user_id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert direct grant");

    let discovery = PgDiscoveryRepo {
        conn: db.conn().clone(),
    };
    let principal = DiscoveryPrincipal {
        user_id: Some(user_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let scopes = discovery
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    let acta = scopes.for_product("acta").expect("acta scopes present");
    assert!(acta.contains(&"acta::workspace::direct-ws".parse().unwrap()));

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn a_grant_reached_only_through_group_membership_is_discovered() {
    let db = create_custos_only_db().await;
    let owner_id = seed_user(&db, "owner").await;
    let member_id = seed_user(&db, "member").await;
    let workspace_id = WorkspaceScope(Uuid::now_v7());

    let group_repo = PgGroupRepo {
        conn: db.conn().clone(),
    };
    let group = group_repo
        .create(NewGroup {
            workspace_id,
            name: "engineers".to_string(),
            created_by: owner_id,
        })
        .await
        .expect("create group");
    group_repo
        .add_member(group.id, member_id)
        .await
        .expect("add member");

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id,
            user_id: None,
            api_key_id: None,
            group_id: Some(group.id),
            resource_ref: "acta::project::group-project".parse().unwrap(),
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert group grant");

    let discovery = PgDiscoveryRepo {
        conn: db.conn().clone(),
    };
    let principal = DiscoveryPrincipal {
        user_id: Some(member_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let scopes = discovery
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    let acta = scopes.for_product("acta").expect("acta scopes present");
    assert!(acta.contains(&"acta::project::group-project".parse().unwrap()));

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn a_grant_via_a_soft_deleted_group_is_excluded() {
    let db = create_custos_only_db().await;
    let owner_id = seed_user(&db, "owner2").await;
    let member_id = seed_user(&db, "member2").await;
    let workspace_id = WorkspaceScope(Uuid::now_v7());

    let group_repo = PgGroupRepo {
        conn: db.conn().clone(),
    };
    let group = group_repo
        .create(NewGroup {
            workspace_id,
            name: "contractors".to_string(),
            created_by: owner_id,
        })
        .await
        .expect("create group");
    group_repo
        .add_member(group.id, member_id)
        .await
        .expect("add member");

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id,
            user_id: None,
            api_key_id: None,
            group_id: Some(group.id),
            resource_ref: "acta::project::deleted-group-project".parse().unwrap(),
            role: ResourceRole::Editor,
            created_by_user_id: Some(owner_id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert group grant");

    group_repo
        .soft_delete(group.id, workspace_id)
        .await
        .expect("soft delete group");

    let discovery = PgDiscoveryRepo {
        conn: db.conn().clone(),
    };
    let principal = DiscoveryPrincipal {
        user_id: Some(member_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let scopes = discovery
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    assert_eq!(
        scopes.for_product("acta"),
        None,
        "a grant reached only through a soft-deleted group must not surface"
    );

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn an_api_key_grant_is_looked_up_by_its_own_id() {
    let db = create_custos_only_db().await;
    let owner_id = seed_user(&db, "keyowner").await;

    let api_key_repo = PgApiKeyRepo {
        conn: db.conn().clone(),
    };
    let key = api_key_repo
        .create_for_user(
            owner_id,
            NewApiKey {
                name: "ci-bot".to_string(),
                token_hash: "hash".to_string(),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: vec![],
            },
        )
        .await
        .expect("create api key");

    let workspace_id = WorkspaceScope(Uuid::now_v7());
    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id,
            user_id: None,
            api_key_id: Some(key.id),
            group_id: None,
            resource_ref: "acta::project::key-project".parse().unwrap(),
            role: ResourceRole::Viewer,
            created_by_user_id: Some(owner_id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert api key grant");

    let discovery = PgDiscoveryRepo {
        conn: db.conn().clone(),
    };
    let principal = DiscoveryPrincipal {
        user_id: None,
        api_key_id: Some(key.id),
        is_root: false,
        is_system_admin: false,
    };

    let scopes = discovery
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");

    let acta = scopes.for_product("acta").expect("acta scopes present");
    assert!(acta.contains(&"acta::project::key-project".parse().unwrap()));

    db.teardown().await.expect("teardown");
}
