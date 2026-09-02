#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! S3 PR3 (T3.11–T3.15): store-level (no middleware) proof of
//! `PgIdempotencyRepo`'s D2 branch table, D7's lazy expiry, and the T3.15
//! bounded opportunistic cleanup.
//!
//! Every test drives `now`/`retention` explicitly (never `Utc::now()` plus a
//! sleep), so the 30s in-flight staleness window and the 24h response TTL
//! are both provable without waiting real time.

mod support;

use atlas_server::persistence::repos::{
    CLEANUP_BATCH_LIMIT, CompleteOutcome, IN_FLIGHT_TTL, IdempotencyScope, InsertOutcome,
    PgIdempotencyRepo,
};
use chrono::{Duration, Utc};
use support::{TestDb, seed_workspace};

const RETENTION: Duration = Duration::hours(24);

fn scope(principal_id: uuid::Uuid, key: &str) -> IdempotencyScope {
    IdempotencyScope {
        principal_id,
        method: "POST".to_string(),
        path: "/api/workspaces/ws/tasks".to_string(),
        key: key.to_string(),
    }
}

/// T3.11/T3.12: two concurrent `insert_in_flight` calls for the same scope —
/// exactly one succeeds as fresh, the other observes the pre-existing
/// `in_flight` row (D2's second branch), not blocked by a lock.
#[tokio::test]
async fn concurrent_insert_for_the_same_scope_yields_exactly_one_fresh_and_one_in_flight() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-race").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let now = Utc::now();
    let sc = scope(user.id.0, "race-key");

    // Both inserts are driven concurrently on the shared pool, so the two
    // statements genuinely interleave against the unique index instead of
    // one completing before the other is issued.
    let (first, second) = tokio::join!(
        repo.insert_in_flight(&sc, b"fingerprint-a", now, RETENTION),
        repo.insert_in_flight(&sc, b"fingerprint-a", now, RETENTION),
    );
    let first = first.expect("first insert must not error");
    let second = second.expect("second insert must not error");

    let outcomes = [first, second];
    let fresh_count = outcomes
        .iter()
        .filter(|o| matches!(o, InsertOutcome::Fresh { .. }))
        .count();
    let in_flight_count = outcomes
        .iter()
        .filter(|o| matches!(o, InsertOutcome::InFlight))
        .count();

    assert_eq!(fresh_count, 1, "exactly one call must win as Fresh");
    assert_eq!(
        in_flight_count, 1,
        "exactly one call must observe the pre-existing in_flight row"
    );

    db.teardown().await;
}

/// T3.13: a row left `in_flight` past the fixed 30s window is treated as
/// abandoned — a subsequent `insert_in_flight` for the same scope succeeds
/// as Fresh (reclaimed), not blocked by the stale row.
#[tokio::test]
async fn stale_in_flight_row_past_30s_is_reclaimed_as_fresh() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-stale").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let t0 = Utc::now();
    let sc = scope(user.id.0, "stale-key");

    let first = repo
        .insert_in_flight(&sc, b"fingerprint-a", t0, RETENTION)
        .await
        .expect("first insert must not error");
    assert!(matches!(first, InsertOutcome::Fresh { .. }));

    let still_within_window = t0 + Duration::seconds(29);
    let blocked = repo
        .insert_in_flight(&sc, b"fingerprint-a", still_within_window, RETENTION)
        .await
        .expect("second insert must not error");
    assert_eq!(
        blocked,
        InsertOutcome::InFlight,
        "within the 30s window the row must still block as in-flight"
    );

    let past_window = t0 + IN_FLIGHT_TTL + Duration::seconds(1);
    let reclaimed = repo
        .insert_in_flight(&sc, b"fingerprint-b", past_window, RETENTION)
        .await
        .expect("reclaim insert must not error");
    assert!(
        matches!(reclaimed, InsertOutcome::Fresh { .. }),
        "past the 30s window the stale row must be reclaimed as fresh, got {reclaimed:?}"
    );

    db.teardown().await;
}

/// F1: the exact boundary tick — `now == created_at + IN_FLIGHT_TTL` for an
/// in-flight row, and separately `now == expires_at` for a completed row —
/// must terminate with a sane outcome instead of recursing forever.
/// `branch_on_existing`'s staleness checks and `reclaim`'s `UPDATE ... WHERE`
/// must agree exactly at this tick (both inclusive), otherwise
/// `branch_on_existing` routes into a reclaim that matches zero rows and
/// falls back into itself indefinitely.
#[tokio::test]
async fn exact_boundary_tick_terminates_instead_of_looping() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-boundary").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };

    // in-flight boundary: now - created_at == IN_FLIGHT_TTL exactly.
    let t0 = Utc::now();
    let in_flight_scope = scope(user.id.0, "boundary-in-flight");
    let started = repo
        .insert_in_flight(&in_flight_scope, b"fp-a", t0, RETENTION)
        .await
        .expect("first insert must not error");
    assert!(matches!(started, InsertOutcome::Fresh { .. }));

    let exact_ttl_boundary = t0 + IN_FLIGHT_TTL;
    let at_boundary = repo
        .insert_in_flight(&in_flight_scope, b"fp-b", exact_ttl_boundary, RETENTION)
        .await
        .expect("boundary insert must terminate and not error");
    assert!(
        matches!(at_boundary, InsertOutcome::Fresh { .. }),
        "age == IN_FLIGHT_TTL exactly must reclaim as fresh, got {at_boundary:?}"
    );

    // completed boundary: now == expires_at exactly.
    let completed_scope = scope(user.id.0, "boundary-completed");
    let t1 = Utc::now();
    let completed_started = repo
        .insert_in_flight(&completed_scope, b"fp-a", t1, RETENTION)
        .await
        .expect("insert must not error");
    let InsertOutcome::Fresh { id, generation } = completed_started else {
        panic!("expected Fresh, got {completed_started:?}");
    };
    repo.complete(id, generation, 201, b"{}", None, t1, RETENTION)
        .await
        .expect("complete must not error");

    let exact_expiry_boundary = t1 + RETENTION;
    let at_expiry_boundary = repo
        .insert_in_flight(&completed_scope, b"fp-b", exact_expiry_boundary, RETENTION)
        .await
        .expect("boundary insert must terminate and not error");
    assert!(
        matches!(at_expiry_boundary, InsertOutcome::Fresh { .. }),
        "expires_at == now exactly must reclaim as fresh, got {at_expiry_boundary:?}"
    );

    db.teardown().await;
}

/// T3.14: a `completed` row whose `expires_at` has passed is not returned by
/// `lookup`, and a subsequent `insert_in_flight` for the same scope proceeds
/// as fresh (D7's lazy-expiry contract).
#[tokio::test]
async fn completed_row_past_ttl_is_invisible_to_lookup_and_reclaimed_on_insert() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-ttl").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let t0 = Utc::now();
    let sc = scope(user.id.0, "ttl-key");

    let started = repo
        .insert_in_flight(&sc, b"fingerprint-a", t0, RETENTION)
        .await
        .expect("insert must not error");
    let InsertOutcome::Fresh { id, generation } = started else {
        panic!("expected Fresh, got {started:?}");
    };

    let outcome = repo
        .complete(id, generation, 201, b"{\"ok\":true}", None, t0, RETENTION)
        .await
        .expect("complete must not error");
    assert_eq!(outcome, CompleteOutcome::Stored);

    let before_expiry = t0 + Duration::hours(1);
    let replay = repo
        .lookup(&sc, before_expiry)
        .await
        .expect("lookup must not error");
    assert!(
        replay.is_some(),
        "a completed, non-expired row must be visible to lookup"
    );

    let after_expiry = t0 + RETENTION + Duration::seconds(1);
    let expired = repo
        .lookup(&sc, after_expiry)
        .await
        .expect("lookup must not error");
    assert!(
        expired.is_none(),
        "a completed row past its TTL must be invisible to lookup"
    );

    let reclaimed = repo
        .insert_in_flight(&sc, b"fingerprint-b", after_expiry, RETENTION)
        .await
        .expect("reclaim insert must not error");
    assert!(
        matches!(reclaimed, InsertOutcome::Fresh { .. }),
        "an insert past the completed row's TTL must proceed as fresh, got {reclaimed:?}"
    );

    db.teardown().await;
}

/// D1/D2: a `completed`, non-expired row with a different fingerprint
/// answers `Mismatch`, carrying the existing fingerprint for the caller to
/// build the `IdempotencyKeyConflict` hint from (PR4).
#[tokio::test]
async fn completed_row_with_different_fingerprint_yields_mismatch() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-mismatch").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let t0 = Utc::now();
    let sc = scope(user.id.0, "mismatch-key");

    let started = repo
        .insert_in_flight(&sc, b"fingerprint-a", t0, RETENTION)
        .await
        .expect("insert must not error");
    let InsertOutcome::Fresh { id, generation } = started else {
        panic!("expected Fresh, got {started:?}");
    };
    repo.complete(id, generation, 201, b"{}", None, t0, RETENTION)
        .await
        .expect("complete must not error");

    let mismatch = repo
        .insert_in_flight(&sc, b"fingerprint-different", t0, RETENTION)
        .await
        .expect("mismatch insert must not error");

    match mismatch {
        InsertOutcome::Mismatch {
            existing_fingerprint,
        } => assert_eq!(existing_fingerprint, b"fingerprint-a"),
        other => panic!("expected Mismatch, got {other:?}"),
    }

    let replay = repo
        .insert_in_flight(&sc, b"fingerprint-a", t0, RETENTION)
        .await
        .expect("replay insert must not error");
    match replay {
        InsertOutcome::Replay(stored) => {
            assert_eq!(stored.status, 201);
            assert_eq!(stored.body, b"{}");
        }
        other => panic!("expected Replay for the matching fingerprint, got {other:?}"),
    }

    db.teardown().await;
}

/// T3.15 (ORCHESTRATOR DECISION 2026-09-02): 150 expired rows for one
/// principal, then one fresh insert — at most `CLEANUP_BATCH_LIMIT` (100)
/// expired rows are deleted by that single insert, leaving at least 50
/// behind, and the fresh row is present.
#[tokio::test]
async fn insert_bounds_opportunistic_cleanup_of_expired_rows_for_the_principal() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-cleanup").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let t0 = Utc::now();

    for i in 0..150 {
        let sc = scope(user.id.0, &format!("expired-{i}"));
        let outcome = repo
            .insert_in_flight(&sc, b"fp", t0 - Duration::hours(48), Duration::hours(1))
            .await
            .expect("seed insert must not error");
        assert!(matches!(outcome, InsertOutcome::Fresh { .. }));
    }

    let after_seeding = t0;
    let count_before = count_rows_for_principal(&db, user.id.0).await;
    assert_eq!(
        count_before, 150,
        "all 150 seed rows must exist before cleanup"
    );

    let fresh_scope = scope(user.id.0, "fresh-key");
    let fresh = repo
        .insert_in_flight(&fresh_scope, b"fp", after_seeding, RETENTION)
        .await
        .expect("fresh insert must not error");
    assert!(matches!(fresh, InsertOutcome::Fresh { .. }));

    let count_after = count_rows_for_principal(&db, user.id.0).await;
    let expired_remaining = count_after.saturating_sub(1);

    assert_eq!(
        expired_remaining,
        150 - CLEANUP_BATCH_LIMIT,
        "one bounded cleanup must delete exactly CLEANUP_BATCH_LIMIT expired rows, leaving the rest"
    );

    let second_insert_for_fresh_scope = repo
        .insert_in_flight(&fresh_scope, b"fp", after_seeding, RETENTION)
        .await
        .expect("second insert for the fresh scope must not error");
    assert_eq!(
        second_insert_for_fresh_scope,
        InsertOutcome::InFlight,
        "the fresh row must be present and still in_flight"
    );

    db.teardown().await;
}

/// A stale generation (reclaimed by a later caller) and a deleted `id`
/// both answer `Superseded`, never silent `Ok`; the successor stays
/// untouched.
#[tokio::test]
async fn complete_supersedes_a_reclaimed_generation_and_a_deleted_row() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-superseded").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };
    let t0 = Utc::now();
    let sc = scope(user.id.0, "superseded-key");

    let first = repo
        .insert_in_flight(&sc, b"fp-a", t0, RETENTION)
        .await
        .expect("first insert must not error");
    let InsertOutcome::Fresh {
        id,
        generation: stale_generation,
    } = first
    else {
        panic!("expected Fresh, got {first:?}");
    };

    let past_window = t0 + IN_FLIGHT_TTL + Duration::seconds(1);
    let reclaimed = repo
        .insert_in_flight(&sc, b"fp-b", past_window, RETENTION)
        .await
        .expect("reclaim insert must not error");
    let InsertOutcome::Fresh {
        id: reclaimed_id,
        generation: reclaimed_generation,
    } = reclaimed
    else {
        panic!("expected Fresh, got {reclaimed:?}");
    };
    assert_eq!(reclaimed_id, id);
    assert_ne!(
        stale_generation, reclaimed_generation,
        "reclaim must mint a fresh generation token distinct from the superseded claim"
    );

    // The generation guard is a minted token, not a timestamp: reusing the
    // exact same `now` (`t0`) as the original stale claim for this `complete`
    // call must still supersede correctly, since `stale_generation` no
    // longer matches the row's current (reclaimed) generation regardless of
    // what `now` is passed.
    let outcome = repo
        .complete(id, stale_generation, 201, b"stale", None, t0, RETENTION)
        .await
        .expect("complete must not error");
    assert_eq!(outcome, CompleteOutcome::Superseded);

    let still_in_flight = repo
        .insert_in_flight(&sc, b"fp-b", past_window, RETENTION)
        .await
        .expect("in_flight probe insert must not error");
    assert_eq!(
        still_in_flight,
        InsertOutcome::InFlight,
        "the successor's row must stay in_flight, untouched by the superseded complete"
    );

    let deleted_key_sc = scope(user.id.0, "deleted-key");
    let started = repo
        .insert_in_flight(&deleted_key_sc, b"fp-a", t0, RETENTION)
        .await
        .expect("insert must not error");
    let InsertOutcome::Fresh {
        id: deleted_id,
        generation,
    } = started
    else {
        panic!("expected Fresh, got {started:?}");
    };
    delete_row(db.conn(), &deleted_key_sc).await;
    let deleted_outcome = repo
        .complete(deleted_id, generation, 201, b"gone", None, t0, RETENTION)
        .await
        .expect("complete must not error");
    assert_eq!(deleted_outcome, CompleteOutcome::Superseded);

    db.teardown().await;
}

/// R3: a row vanishing between the losing insert and the reclaim update
/// (e.g. concurrent cleanup) must recover with a bounded re-insert, never
/// `DomainError::Internal`. Not forceable deterministically from one
/// sequential call (no seam between the internal statements), so this
/// races a real delete against the reclaim and asserts no error.
#[tokio::test]
async fn stale_reclaim_racing_a_concurrent_delete_never_errors() {
    let db = TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = seed_workspace(&db, "idem-race-delete").await;
    let repo = PgIdempotencyRepo {
        conn: db.conn().clone(),
    };

    for i in 0..10 {
        let sc = scope(user.id.0, &format!("vanish-{i}"));
        let t0 = Utc::now();
        repo.insert_in_flight(&sc, b"fp", t0, RETENTION)
            .await
            .expect("seed insert must not error");

        let past_window = t0 + IN_FLIGHT_TTL + Duration::seconds(1);
        let (reclaim_result, ()) = tokio::join!(
            repo.insert_in_flight(&sc, b"fp-2", past_window, RETENTION),
            delete_row(db.conn(), &sc),
        );

        assert!(
            reclaim_result.is_ok(),
            "a stale reclaim racing a concurrent delete must not error, got {reclaim_result:?}"
        );
    }

    db.teardown().await;
}

async fn delete_row(conn: &sea_orm::DatabaseConnection, sc: &IdempotencyScope) {
    use sea_orm::{ConnectionTrait, DatabaseBackend, Statement};

    let _ = conn
        .execute_raw(Statement::from_sql_and_values(
            DatabaseBackend::Postgres,
            "DELETE FROM platform.idempotency_keys \
             WHERE principal_id = $1 AND method = $2 AND path = $3 AND key = $4",
            [
                sc.principal_id.into(),
                sc.method.clone().into(),
                sc.path.clone().into(),
                sc.key.clone().into(),
            ],
        ))
        .await;
}

async fn count_rows_for_principal(db: &TestDb, principal_id: uuid::Uuid) -> i64 {
    use sea_orm::{FromQueryResult, Statement};

    #[derive(Debug, FromQueryResult)]
    struct Row {
        count: i64,
    }

    Row::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT COUNT(*) AS count FROM platform.idempotency_keys WHERE principal_id = $1",
        [principal_id.into()],
    ))
    .one(db.conn())
    .await
    .expect("count query")
    .expect("count row")
    .count
}
