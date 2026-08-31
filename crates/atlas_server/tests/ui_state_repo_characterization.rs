#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! S4 PR9: `user_ui_state` moves to `platform.ui_state` (design §D4).
//!
//! T9.1 pins `PgUiStateRepo`'s pre-move query shape and row data — keyed by
//! `user_id`, `find`/`upsert` round trip, upsert-overwrites-on-conflict
//! semantics — against the post-move entity/repo before the migration lands,
//! same discipline as the S4 PR6/PR7/PR8 repo characterization tests. Must
//! keep passing unmodified once the migration lands (T9.6): query semantics
//! unchanged, key unchanged, no re-key to `principal_id`.
//!
//! T9.4 is the `information_schema` proof that `platform.ui_state` exists
//! with the same primary key and column set as `public.user_ui_state` had,
//! mirroring `m20260830_000051_custos_set_schema`'s FK-intact proof pattern
//! (applied here to a primary key, since a bare `SET SCHEMA` + `RENAME`
//! never touches `pg_constraint`/`pg_class` row identity, only their
//! `relnamespace`/`relname`).

mod support;

use atlas_server::persistence::repos::PgUiStateRepo;
use atlas_server::platform::UiStateRepo;
use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
use serde_json::json;
use support::{TestDb, seed_workspace};

#[tokio::test]
async fn ui_state_repo_find_and_upsert_round_trip_keyed_by_user_id() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "ui-state-repo").await;
    let repo = PgUiStateRepo {
        conn: db.conn().clone(),
    };

    let missing = repo.find(user.id).await.expect("find must not error");
    assert!(
        missing.is_none(),
        "no row must exist before the first upsert"
    );

    let first_state = json!({ "collapsedFolders": ["a"] });
    let created = repo
        .upsert(user.id, first_state.clone())
        .await
        .expect("upsert must succeed");
    assert_eq!(created.user_id, user.id);
    assert_eq!(created.state, first_state);

    let found = repo
        .find(user.id)
        .await
        .expect("find must not error")
        .expect("row must exist after upsert");
    assert_eq!(found.user_id, user.id);
    assert_eq!(found.state, first_state);

    let second_state = json!({ "collapsedFolders": ["b", "c"], "sidebarWidth": 240 });
    let updated = repo
        .upsert(user.id, second_state.clone())
        .await
        .expect("second upsert must succeed");
    assert_eq!(
        updated.state, second_state,
        "upsert must overwrite the previous state on conflict, not merge it"
    );

    let refound = repo
        .find(user.id)
        .await
        .expect("find must not error")
        .expect("row must still exist");
    assert_eq!(refound.state, second_state);

    db.teardown().await;
}

/// Two different users each get their own row, keyed by `user_id` alone —
/// pinning the key shape before the migration (no re-key to a workspace- or
/// principal-scoped key).
#[tokio::test]
async fn ui_state_rows_are_scoped_per_user() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws_a, user_a) = seed_workspace(&db, "ui-state-user-a").await;
    let (_ws_b, user_b) = seed_workspace(&db, "ui-state-user-b").await;
    let repo = PgUiStateRepo {
        conn: db.conn().clone(),
    };

    repo.upsert(user_a.id, json!({ "owner": "a" }))
        .await
        .expect("upsert for user a");
    repo.upsert(user_b.id, json!({ "owner": "b" }))
        .await
        .expect("upsert for user b");

    let a = repo
        .find(user_a.id)
        .await
        .expect("find a")
        .expect("row a must exist");
    let b = repo
        .find(user_b.id)
        .await
        .expect("find b")
        .expect("row b must exist");

    assert_eq!(a.state, json!({ "owner": "a" }));
    assert_eq!(b.state, json!({ "owner": "b" }));

    db.teardown().await;
}

/// T9.4: `platform.ui_state` exists with the same primary key column
/// (`user_id`) `public.user_ui_state` had, proving the move preserved the
/// constraint rather than dropping and recreating the table.
#[tokio::test]
async fn platform_ui_state_keeps_its_primary_key_on_user_id() {
    let db = TestDb::create().await.expect("TestDb::create");

    #[derive(Debug, FromQueryResult)]
    struct Row {
        relnamespace: String,
        column_name: String,
    }

    let rows = Row::find_by_statement(Statement::from_string(
        sea_orm::DatabaseBackend::Postgres,
        r#"
        SELECT n.nspname AS relnamespace, a.attname AS column_name
        FROM pg_constraint c
        JOIN pg_class t ON t.oid = c.conrelid
        JOIN pg_namespace n ON n.oid = t.relnamespace
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = ANY(c.conkey)
        WHERE c.contype = 'p'
          AND t.relname = 'ui_state'
        "#
        .to_string(),
    ))
    .all(db.conn())
    .await
    .expect("query pg_constraint for platform.ui_state's primary key");

    assert_eq!(
        rows.len(),
        1,
        "expected exactly one primary-key column on platform.ui_state, got {rows:?}"
    );
    assert_eq!(rows[0].relnamespace, "platform");
    assert_eq!(rows[0].column_name, "user_id");

    let old_name_gone = db
        .conn()
        .query_one_raw(Statement::from_string(
            sea_orm::DatabaseBackend::Postgres,
            "SELECT EXISTS(\
                SELECT 1 FROM information_schema.tables \
                WHERE table_name = 'user_ui_state'\
            ) AS still_exists"
                .to_string(),
        ))
        .await
        .expect("query information_schema.tables")
        .expect("query must return a row");
    let still_exists: bool = old_name_gone
        .try_get("", "still_exists")
        .expect("still_exists column");
    assert!(
        !still_exists,
        "expected no table named user_ui_state to remain under any schema"
    );

    db.teardown().await;
}
