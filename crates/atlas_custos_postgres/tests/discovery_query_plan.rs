//! Permanent guard for the spec's "The reverse query does not scan"
//! scenario: seeds a representative row count in `custos.permission_grants`,
//! then asserts the exact SQL `PgDiscoveryRepo` issues (via
//! `atlas_custos_postgres::repos::discovery::build_query`, never a
//! hand-copied approximation) never resorts to a sequential scan on
//! `permission_grants`.
//!
//! **Why seeded rows, not `SET enable_seqscan = off`**: forcing the planner
//! off sequential scans proves nothing about the query shape — a planner
//! that would otherwise choose a seq scan will simply pick whatever plan is
//! left, which is not the same claim as "this query is naturally indexable
//! at realistic scale". Instead this test seeds 2,000 grants across four
//! noise principals (Postgres's default `default_statistics_target`/planner
//! heuristics reliably prefer an index scan over ~2k rows, versus the
//! literal handful a bare-bones fixture would otherwise contain — a tiny
//! table can make even a well-indexed query cheaper to plan as a seq scan,
//! which is exactly the false-negative this guard must not produce).
//! `ANALYZE` is run after seeding so the planner's row-count estimates
//! reflect the real data instead of stale (pre-seed) statistics.
//!
//! **`EXPLAIN` text, not `FORMAT JSON`**: this crate's `sea-orm` dependency
//! does not enable the `with-json` feature, so a `json`-typed column cannot
//! be decoded generically through `sea_orm::QueryResult` without adding that
//! feature workspace-wide. Plain-text `EXPLAIN (ANALYZE)` output decodes
//! cleanly as `String` and is asserted the same way a JSON plan would be
//! (per-node scan type), so the guard is equivalent in strength.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use atlas_core::principal::UserId;
use atlas_custos::WorkspaceScope;
use atlas_custos::entities::groups::NewGroup;
use atlas_custos::entities::identity::NewUser;
use atlas_custos::entities::permissions::{NewPermissionGrant, ResourceRole};
use atlas_custos::ports::discovery::DiscoveryPrincipal;
use atlas_custos::ports::discovery::{DiscoveryPort, MAX_DISCOVERY_SCOPES};
use atlas_custos::ports::grant_repo::PermissionGrantRepo as PermissionGrantRepoTrait;
use atlas_custos::ports::group_repo::GroupRepo as GroupRepoTrait;
use atlas_custos::ports::identity::UserRepo as UserRepoTrait;
use atlas_custos_postgres::repos::discovery::PgDiscoveryRepo;
use atlas_custos_postgres::repos::discovery::build_query;
use atlas_custos_postgres::repos::identity::PgUserRepo;
use atlas_custos_postgres::repos::permissions::{PgGroupRepo, PgPermissionGrantRepo};
use atlas_test_db::TestDb;
use sea_orm::{ConnectionTrait, Statement};
use sea_orm_migration::MigratorTrait;
use uuid::Uuid;

/// Mirrors `discovery_repo_characterization.rs`'s helper: this suite only
/// needs Custos-owned tables, so it stops before `acta_new()` (see that
/// file's doc comment for why running past it requires `pgvector`).
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

/// Bulk-inserts `count` direct-grant rows for `noise_user_id`, each with a
/// distinct `resource_ref` so the unique index is never violated. Raw SQL
/// (not the repo's one-row-at-a-time `upsert`) so seeding 2,000 rows stays
/// fast.
async fn seed_noise_grants(db: &TestDb, noise_user_id: UserId, offset: i64, count: i64) {
    let sql = format!(
        "INSERT INTO custos.permission_grants \
         (id, workspace_id, user_id, api_key_id, group_id, resource_ref, role, \
          created_by_user_id, created_by_api_key_id, created_at, updated_at) \
         SELECT gen_random_uuid(), gen_random_uuid(), '{noise_user_id}'::uuid, NULL, NULL, \
                'acta::project::noise-' || (i + {offset}), 'viewer', \
                '{noise_user_id}'::uuid, NULL, now(), now() \
         FROM generate_series(1, {count}) AS i",
        noise_user_id = noise_user_id.0,
    );
    db.conn()
        .execute_unprepared(&sql)
        .await
        .expect("seed noise grants");
}

/// Runs plain-text `EXPLAIN (ANALYZE)` on `sql` with one `Uuid` parameter and
/// returns the concatenated plan text (one line per row Postgres returns).
async fn explain_text(db: &TestDb, sql: &str, param: Uuid) -> String {
    let stmt = Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        format!("EXPLAIN (ANALYZE) {sql}"),
        [param.into()],
    );
    let rows = db.conn().query_all_raw(stmt).await.expect("run EXPLAIN");

    rows.iter()
        .map(|row| {
            row.try_get::<String>("", "QUERY PLAN")
                .expect("QUERY PLAN column")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_no_seq_scan_on_permission_grants(plan: &str, sql_label: &str) {
    for line in plan.lines() {
        let lower = line.to_ascii_lowercase();
        if lower.contains("seq scan") {
            assert!(
                !lower.contains("permission_grants") && !lower.contains(" pg "),
                "{sql_label}: found a Seq Scan touching permission_grants in the plan:\n{plan}"
            );
        }
    }
    assert!(
        plan.to_ascii_lowercase().contains("index"),
        "{sql_label}: expected at least one Index/Bitmap Index Scan in the plan:\n{plan}"
    );
}

#[tokio::test]
async fn the_user_and_group_union_query_never_seq_scans_permission_grants() {
    let db = create_custos_only_db().await;

    // Noise: 2,000 grants spread across 4 unrelated users, so the planner
    // sees a representative table size rather than a handful of rows.
    for i in 0..4u32 {
        let noise_user_id = seed_user(&db, &format!("noise-{i}")).await;
        seed_noise_grants(&db, noise_user_id, i64::from(i) * 500, 500).await;
    }

    // The principal under test: one direct grant, one group-derived grant.
    let owner_id = seed_user(&db, "owner").await;
    let member_id = seed_user(&db, "member").await;
    let workspace_id = WorkspaceScope(Uuid::now_v7());

    let grant_repo = PgPermissionGrantRepo {
        conn: db.conn().clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id,
            user_id: Some(member_id),
            api_key_id: None,
            group_id: None,
            resource_ref: "acta::workspace::direct-ws".parse().unwrap(),
            role: ResourceRole::Viewer,
            created_by_user_id: Some(member_id),
            created_by_api_key_id: None,
        })
        .await
        .expect("upsert direct grant");

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

    db.conn()
        .execute_unprepared("ANALYZE custos.permission_grants")
        .await
        .expect("ANALYZE permission_grants");
    db.conn()
        .execute_unprepared("ANALYZE custos.group_members")
        .await
        .expect("ANALYZE group_members");

    let principal = DiscoveryPrincipal {
        user_id: Some(member_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };
    let (sql, param) = build_query(&principal).expect("user principal builds a query");

    let plan = explain_text(&db, &sql, param).await;
    assert_no_seq_scan_on_permission_grants(&plan, "user+group UNION query");

    db.teardown().await.expect("teardown");
}

#[tokio::test]
async fn the_api_key_query_never_seq_scans_permission_grants() {
    let db = create_custos_only_db().await;

    for i in 0..4u32 {
        let noise_user_id = seed_user(&db, &format!("apikey-noise-{i}")).await;
        seed_noise_grants(&db, noise_user_id, i64::from(i) * 500, 500).await;
    }

    let owner_id = seed_user(&db, "keyowner").await;
    let api_key_repo = atlas_custos_postgres::repos::identity::PgApiKeyRepo {
        conn: db.conn().clone(),
    };
    let key = {
        use atlas_custos::entities::identity::{ApiKeyType, NewApiKey};
        use atlas_custos::ports::identity::ApiKeyRepo as ApiKeyRepoTrait;

        api_key_repo
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
            .expect("create api key")
    };

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

    db.conn()
        .execute_unprepared("ANALYZE custos.permission_grants")
        .await
        .expect("ANALYZE permission_grants");

    let principal = DiscoveryPrincipal {
        user_id: None,
        api_key_id: Some(key.id),
        is_root: false,
        is_system_admin: false,
    };
    let (sql, param) = build_query(&principal).expect("api key principal builds a query");

    let plan = explain_text(&db, &sql, param).await;
    assert_no_seq_scan_on_permission_grants(&plan, "api-key query");

    db.teardown().await.expect("teardown");
}

/// Review CRITICAL (resilience): a principal over `MAX_DISCOVERY_SCOPES` is
/// capped, reported `truncated`, and the capped SQL stays index-driven.
#[tokio::test]
async fn a_principal_over_the_cap_is_capped_truncated_and_still_index_driven() {
    let db = create_custos_only_db().await;

    for i in 0..4u32 {
        let noise_user_id = seed_user(&db, &format!("cap-noise-{i}")).await;
        seed_noise_grants(&db, noise_user_id, i64::from(i) * 500, 500).await;
    }

    let owner_id = seed_user(&db, "cap-owner").await;
    let member_id = seed_user(&db, "cap-member").await;
    let workspace_id = WorkspaceScope(Uuid::now_v7());

    let over_cap = (MAX_DISCOVERY_SCOPES + 50) as i64;
    seed_noise_grants(&db, member_id, 0, over_cap).await;

    let group_repo = PgGroupRepo {
        conn: db.conn().clone(),
    };
    let group = group_repo
        .create(NewGroup {
            workspace_id,
            name: "cap-engineers".to_string(),
            created_by: owner_id,
        })
        .await
        .expect("create group");
    group_repo
        .add_member(group.id, member_id)
        .await
        .expect("add member");
    PgPermissionGrantRepo {
        conn: db.conn().clone(),
    }
    .upsert(NewPermissionGrant {
        workspace_id,
        user_id: None,
        api_key_id: None,
        group_id: Some(group.id),
        resource_ref: "acta::project::cap-group-project".parse().unwrap(),
        role: ResourceRole::Editor,
        created_by_user_id: Some(owner_id),
        created_by_api_key_id: None,
    })
    .await
    .expect("upsert group grant");

    db.conn()
        .execute_unprepared("ANALYZE custos.permission_grants")
        .await
        .expect("ANALYZE permission_grants");
    db.conn()
        .execute_unprepared("ANALYZE custos.group_members")
        .await
        .expect("ANALYZE group_members");

    let principal = DiscoveryPrincipal {
        user_id: Some(member_id),
        api_key_id: None,
        is_root: false,
        is_system_admin: false,
    };

    let discovery = PgDiscoveryRepo {
        conn: db.conn().clone(),
    };
    let scopes = discovery
        .granted_scopes(&principal)
        .await
        .expect("granted_scopes");
    assert_eq!(
        scopes.for_product("acta").expect("acta present").len(),
        MAX_DISCOVERY_SCOPES
    );
    assert!(scopes.truncated);

    let (sql, param) = build_query(&principal).expect("user principal builds a query");
    let plan = explain_text(&db, &sql, param).await;
    assert_no_seq_scan_on_permission_grants(&plan, "over-the-cap user+group UNION query");

    db.teardown().await.expect("teardown");
}
