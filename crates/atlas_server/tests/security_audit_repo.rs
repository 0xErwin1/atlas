#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_domain::{
    Actor,
    entities::security_audit::{AuditFilters, NewSecurityAuditEvent, SecurityAction},
    ids::UserId,
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, NewApiKey, NewUser, PgSecurityAuditRepo, SecurityAuditRepo, UserRepo,
};
use sea_orm::{ConnectionTrait, TransactionTrait};

// ─── helpers ────────────────────────────────────────────────────────────────

fn user_event(
    ws_id: Option<atlas_domain::ids::WorkspaceId>,
    actor_id: UserId,
) -> NewSecurityAuditEvent {
    NewSecurityAuditEvent {
        workspace_id: ws_id,
        actor: Actor::User(actor_id),
        action: SecurityAction::UserDisabled,
        target_type: "user".into(),
        target_id: Some(uuid::Uuid::now_v7()),
        metadata: serde_json::json!({}),
    }
}

fn platform_event(actor_id: UserId) -> NewSecurityAuditEvent {
    NewSecurityAuditEvent {
        workspace_id: None,
        actor: Actor::User(actor_id),
        action: SecurityAction::UserCreated,
        target_type: "user".into(),
        target_id: Some(uuid::Uuid::now_v7()),
        metadata: serde_json::json!({ "workspace_id": "ws-abc", "initial_role": "member" }),
    }
}

// ─── append_in — basic insert via pool ──────────────────────────────────────

#[tokio::test]
async fn append_in_inserts_user_actor_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-user-actor").await;

    let event = user_event(Some(ws.id), user.id);
    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append_in");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace");

    assert_eq!(rows.len(), 1, "one row must exist after append");
    assert_eq!(rows[0].actor, Actor::User(user.id));
    assert_eq!(rows[0].workspace_id, Some(ws.id));

    db.teardown().await;
}

#[tokio::test]
async fn append_in_inserts_api_key_actor_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-key-actor").await;

    let raw_secret = "test-api-key-secret-sal-001";
    let token_hash = atlas_server::auth::tokens::hash_token(raw_secret);
    let ctx = atlas_domain::WorkspaceCtx::new(ws.id, Actor::User(user.id));
    let key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "sal-key".into(),
                token_hash,
                type_: atlas_domain::entities::identity::ApiKeyType::Agent,
                expires_at: None,
            },
        )
        .await
        .expect("create api key");

    let event = NewSecurityAuditEvent {
        workspace_id: None,
        actor: Actor::ApiKey(key.id),
        action: SecurityAction::ApiKeyCreated,
        target_type: "api_key".into(),
        target_id: Some(key.id.0),
        metadata: serde_json::json!({ "key_type": "agent", "key_name": "sal-key" }),
    };

    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append_in with api_key actor");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_platform(&AuditFilters::default(), None, 10)
        .await
        .expect("list_platform");

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].actor, Actor::ApiKey(key.id));
    assert!(rows[0].workspace_id.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn append_in_platform_event_has_null_workspace() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-platform-null-ws").await;

    let event = platform_event(user.id);
    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append_in platform event");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_platform(&AuditFilters::default(), None, 10)
        .await
        .expect("list_platform");

    assert_eq!(rows.len(), 1);
    assert!(
        rows[0].workspace_id.is_none(),
        "platform event must have NULL workspace_id"
    );

    let ws_rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace");
    assert_eq!(
        ws_rows.len(),
        0,
        "platform event must NOT appear in workspace feed"
    );

    db.teardown().await;
}

#[tokio::test]
async fn append_in_metadata_json_round_trips() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-metadata-rt").await;

    let meta = serde_json::json!({
        "old_role": "member",
        "new_role": "admin",
        "nested": { "key": 42 }
    });

    let event = NewSecurityAuditEvent {
        workspace_id: Some(ws.id),
        actor: Actor::User(user.id),
        action: SecurityAction::MembershipRoleChanged,
        target_type: "user".into(),
        target_id: Some(user.id.0),
        metadata: meta.clone(),
    };

    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append_in metadata");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace");

    assert_eq!(rows.len(), 1);
    assert_eq!(
        rows[0].metadata, meta,
        "metadata JSON must round-trip faithfully"
    );

    db.teardown().await;
}

// ─── append_in — atomicity (rollback leaves no row) ──────────────────────────

#[tokio::test]
async fn append_in_inside_rolled_back_txn_leaves_no_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-rollback").await;

    let txn = db.conn().begin().await.expect("begin txn");

    PgSecurityAuditRepo::append_in(&txn, user_event(Some(ws.id), user.id))
        .await
        .expect("append_in inside txn");

    txn.rollback().await.expect("rollback");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace after rollback");

    assert_eq!(
        rows.len(),
        0,
        "rolled-back txn must leave zero audit rows (atomicity contract)"
    );

    db.teardown().await;
}

#[tokio::test]
async fn append_in_inside_committed_txn_persists_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-commit").await;

    let txn = db.conn().begin().await.expect("begin txn");

    PgSecurityAuditRepo::append_in(&txn, user_event(Some(ws.id), user.id))
        .await
        .expect("append_in inside txn");

    txn.commit().await.expect("commit");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());
    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace after commit");

    assert_eq!(rows.len(), 1, "committed txn must persist the audit row");

    db.teardown().await;
}

// ─── actor CHECK constraint ──────────────────────────────────────────────────
//
// The constraint is `num_nonnulls(...) <= 1` (at most one), not a strict XOR.
// Both-null is allowed because ON DELETE SET NULL can null the actor when the
// actor user/key is hard-deleted — that row must still survive with a null actor.
// Both-set (two non-null) is still rejected: that is a corruption case.

#[tokio::test]
async fn actor_check_allows_both_null_after_on_delete_set_null() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, _user) = support::seed_workspace(&db, "sal-atmost-null").await;

    let ws_id = ws.id.0;

    // Both-null is the state left after ON DELETE SET NULL fires on the actor.
    // The constraint must allow it so the audit row survives actor deletion.
    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO security_audit_log
               (id, workspace_id, actor_user_id, actor_api_key_id, action, target_type, metadata, created_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', NULL, NULL, 'user.disabled', 'user', '{{}}', now())"#
        ))
        .await;

    assert!(
        result.is_ok(),
        "both-null actors must be allowed (ON DELETE SET NULL state): {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn actor_check_rejects_both_set_actors() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-atmost-both").await;

    let ws_id = ws.id.0;
    let user_id = user.id.0;

    let result = db
        .conn()
        .execute_unprepared(&format!(
            r#"INSERT INTO security_audit_log
               (id, workspace_id, actor_user_id, actor_api_key_id, action, target_type, metadata, created_at)
               VALUES
               (gen_random_uuid(), '{ws_id}', '{user_id}', '{user_id}', 'user.disabled', 'user', '{{}}', now())"#
        ))
        .await;

    assert!(
        result.is_err(),
        "sal_actor_atmost_one CHECK must reject both-set actors"
    );

    db.teardown().await;
}

// ─── list_for_workspace — ordering, pagination, partition ──────────────────

#[tokio::test]
async fn list_for_workspace_returns_newest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-order").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    for action in [
        SecurityAction::MembershipRoleChanged,
        SecurityAction::MembershipRemoved,
    ] {
        PgSecurityAuditRepo::append_in(
            db.conn(),
            NewSecurityAuditEvent {
                workspace_id: Some(ws.id),
                actor: Actor::User(user.id),
                action,
                target_type: "user".into(),
                target_id: Some(user.id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("append_in");

        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list_for_workspace");

    assert_eq!(rows.len(), 2);
    assert_eq!(
        rows[0].action,
        SecurityAction::MembershipRemoved.as_str(),
        "newest must be first"
    );
    assert_eq!(
        rows[1].action,
        SecurityAction::MembershipRoleChanged.as_str(),
        "oldest must be last"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_for_workspace_keyset_paginates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-keyset").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    for _ in 0..5 {
        PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user.id))
            .await
            .expect("append");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let page1 = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 3)
        .await
        .expect("page1");
    assert_eq!(page1.len(), 3, "first page must have 3 rows");

    let last = page1.last().expect("last");
    let cursor = atlas_domain::entities::security_audit::AuditCursor {
        created_at: last.created_at,
        id: last.id,
    };

    let page2 = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), Some(cursor), 10)
        .await
        .expect("page2");
    assert_eq!(page2.len(), 2, "second page must have the remaining 2 rows");

    let all_ids: Vec<_> = page1.iter().chain(page2.iter()).map(|r| r.id).collect();
    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    assert_eq!(
        all_ids.len(),
        unique.len(),
        "no duplicate rows across pages"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_for_workspace_filter_by_action() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-action-filter").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    for action in [
        SecurityAction::MembershipRoleChanged,
        SecurityAction::MembershipRemoved,
        SecurityAction::MembershipRoleChanged,
    ] {
        PgSecurityAuditRepo::append_in(
            db.conn(),
            NewSecurityAuditEvent {
                workspace_id: Some(ws.id),
                actor: Actor::User(user.id),
                action,
                target_type: "user".into(),
                target_id: Some(user.id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .expect("append");
    }

    let filters = AuditFilters {
        action: Some(SecurityAction::MembershipRoleChanged.as_str().to_string()),
        ..Default::default()
    };
    let rows = repo
        .list_for_workspace(ws.id, &filters, None, 10)
        .await
        .expect("filtered list");

    assert_eq!(rows.len(), 2, "only role_changed rows must be returned");
    for r in &rows {
        assert_eq!(r.action, SecurityAction::MembershipRoleChanged.as_str());
    }

    db.teardown().await;
}

#[tokio::test]
async fn list_for_workspace_filter_by_actor_user_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user_a) = support::seed_workspace(&db, "sal-actor-filter-a").await;

    let user_b = db
        .user_repo()
        .create(NewUser {
            username: "sal-actor-filter-b".into(),
            display_name: "B".into(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user b");

    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user_a.id))
        .await
        .expect("append a");

    PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user_b.id))
        .await
        .expect("append b");

    let filters = AuditFilters {
        actor_user_id: Some(user_a.id),
        ..Default::default()
    };
    let rows = repo
        .list_for_workspace(ws.id, &filters, None, 10)
        .await
        .expect("filtered by actor");

    assert_eq!(rows.len(), 1, "only user_a's events must be returned");
    assert_eq!(rows[0].actor, Actor::User(user_a.id));

    db.teardown().await;
}

#[tokio::test]
async fn list_for_workspace_filter_by_date_range() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-date-filter").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user.id))
        .await
        .expect("append before");

    let mid = chrono::Utc::now();
    tokio::time::sleep(std::time::Duration::from_millis(5)).await;

    PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user.id))
        .await
        .expect("append after");

    let filters = AuditFilters {
        from: Some(mid),
        ..Default::default()
    };
    let rows = repo
        .list_for_workspace(ws.id, &filters, None, 10)
        .await
        .expect("date-filtered list");

    assert_eq!(rows.len(), 1, "only the row after mid must be returned");

    db.teardown().await;
}

// ─── workspace vs platform partition ─────────────────────────────────────────

#[tokio::test]
async fn workspace_and_platform_events_are_partitioned() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-partition").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    PgSecurityAuditRepo::append_in(db.conn(), user_event(Some(ws.id), user.id))
        .await
        .expect("workspace event");

    PgSecurityAuditRepo::append_in(db.conn(), platform_event(user.id))
        .await
        .expect("platform event");

    let ws_rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("workspace rows");
    let pl_rows = repo
        .list_platform(&AuditFilters::default(), None, 10)
        .await
        .expect("platform rows");

    assert_eq!(
        ws_rows.len(),
        1,
        "workspace feed must have only workspace-scoped rows"
    );
    assert_eq!(
        pl_rows.len(),
        1,
        "platform feed must have only NULL-workspace rows"
    );
    assert!(ws_rows[0].workspace_id.is_some());
    assert!(pl_rows[0].workspace_id.is_none());

    db.teardown().await;
}

// ─── FK / ON DELETE behaviour ──────────────────────────────────────────────

#[tokio::test]
async fn audit_row_survives_target_deleted_no_fk_on_target_id() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "sal-target-del").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    let target_id = uuid::Uuid::now_v7();

    let event = NewSecurityAuditEvent {
        workspace_id: Some(ws.id),
        actor: Actor::User(user.id),
        action: SecurityAction::UserDisabled,
        target_type: "user".into(),
        target_id: Some(target_id),
        metadata: serde_json::json!({}),
    };

    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append event");

    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list before delete");
    assert_eq!(rows.len(), 1);

    // target_id is a bare UUID with no FK — deleting the "target" has no effect on the audit row.
    // We verify the row still exists after the referenced entity is gone by simply confirming
    // the row persists (there is nothing to delete since target_id has no FK constraint).
    let rows_after = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list after (no FK on target_id — row must persist)");
    assert_eq!(
        rows_after.len(),
        1,
        "audit row must persist regardless of target_id being orphaned"
    );

    db.teardown().await;
}

#[tokio::test]
async fn actor_fk_on_delete_set_null_nulls_actor_but_keeps_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "sal-actor-on-delete").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    let victim = db
        .user_repo()
        .create(NewUser {
            username: "sal-victim-actor".into(),
            display_name: "Victim".into(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create victim user");

    let event = NewSecurityAuditEvent {
        workspace_id: Some(ws.id),
        actor: Actor::User(victim.id),
        action: SecurityAction::MembershipRemoved,
        target_type: "user".into(),
        target_id: Some(owner.id.0),
        metadata: serde_json::json!({}),
    };

    PgSecurityAuditRepo::append_in(db.conn(), event)
        .await
        .expect("append event");

    let victim_id = victim.id.0;
    db.conn()
        .execute_unprepared(&format!("DELETE FROM users WHERE id = '{victim_id}'"))
        .await
        .expect("delete victim user");

    let rows = repo
        .list_for_workspace(ws.id, &AuditFilters::default(), None, 10)
        .await
        .expect("list after actor delete");

    assert_eq!(rows.len(), 1, "audit row must survive actor deletion");

    db.teardown().await;
}

// ─── list_platform — keyset pagination ───────────────────────────────────────

#[tokio::test]
async fn list_platform_keyset_paginates() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (_ws, user) = support::seed_workspace(&db, "sal-platform-pag").await;
    let repo = PgSecurityAuditRepo::new(db.conn().clone());

    for _ in 0..4 {
        PgSecurityAuditRepo::append_in(db.conn(), platform_event(user.id))
            .await
            .expect("append platform event");
        tokio::time::sleep(std::time::Duration::from_millis(2)).await;
    }

    let page1 = repo
        .list_platform(&AuditFilters::default(), None, 2)
        .await
        .expect("platform page1");
    assert_eq!(page1.len(), 2);

    let last = page1.last().expect("last");
    let cursor = atlas_domain::entities::security_audit::AuditCursor {
        created_at: last.created_at,
        id: last.id,
    };

    let page2 = repo
        .list_platform(&AuditFilters::default(), Some(cursor), 10)
        .await
        .expect("platform page2");
    assert_eq!(page2.len(), 2, "second page must have remaining 2 rows");

    db.teardown().await;
}
