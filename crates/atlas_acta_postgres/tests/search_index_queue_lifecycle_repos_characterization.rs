#![allow(clippy::expect_used, clippy::unwrap_used, clippy::indexing_slicing)]

//! Characterization test for `PgSearchIndexQueueRepo`/`PgPurgeOperationRepo`/
//! `PgPropertyDefinitionRepo`'s current query shapes, ported from
//! `atlas_server::persistence::repos::{search_index_queue, lifecycle,
//! workspace_core}` before that code moves into this crate (S4 PR8, T8.1).
//! Must keep passing unmodified once the move lands (T8.4).
//!
//! `search_index_queue` rows carry no FK on `resource_id` (design D1's R8
//! classification: the table is workspace-scoped only, with no `Module` type
//! to reference), so this test enqueues a synthetic document UUID with no
//! backing `documents` row — the same shape the pre-move production code
//! allows.
//!
//! Runs against a disposable Postgres named by `ATLAS_TEST_DATABASE_URL`.

use atlas_acta::actor::{Actor, UserAttributionId, WorkspaceCtx};
use atlas_acta::entities::identity::NewWorkspace;
use atlas_acta::entities::lifecycle::{RestoreTarget, SecurityAuditRef, TrashKind};
use atlas_acta::entities::workspace_core::{AppliesTo, NewPropertyDefinition, PropertyKind};
use atlas_acta::ids::{DocumentId, WorkspaceId};
use atlas_acta::ports::identity::WorkspaceRepo;
use atlas_acta::ports::workspace_core::PropertyDefinitionRepo;
use atlas_acta_postgres::repos::identity::PgWorkspaceRepo;
use atlas_acta_postgres::repos::lifecycle::{NewPurgeOperation, PgPurgeOperationRepo};
use atlas_acta_postgres::repos::search_index_queue::PgSearchIndexQueueRepo;
use atlas_acta_postgres::repos::workspace_core::PgPropertyDefinitionRepo;
use atlas_core::principal::UserId;
use atlas_custos::entities::identity::NewUser;
use atlas_custos_postgres::repos::identity::{PgUserRepo, UserRepo};
use atlas_test_db::TestDb;
use uuid::Uuid;

async fn seed_workspace(db: &TestDb, slug: &str) -> WorkspaceId {
    let workspace_repo = PgWorkspaceRepo {
        conn: db.conn().clone(),
    };
    let workspace_id = WorkspaceId(Uuid::now_v7());
    workspace_repo
        .create(NewWorkspace {
            id: workspace_id,
            name: slug.to_string(),
            slug: slug.to_string(),
        })
        .await
        .expect("workspace must be created");
    workspace_id
}

// `purge_operations.original_actor_user_id`/`property_definitions.created_by_user_id`
// are foreign keys into `custos.users`: seed a real user rather than a random
// UUID.
async fn seed_user(db: &TestDb, username: &str) -> UserId {
    let repo = PgUserRepo {
        conn: db.conn().clone(),
    };
    let user = repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user must be created");
    user.id
}

#[tokio::test]
async fn search_index_queue_repo_enqueue_claim_and_complete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "search-index-queue-test").await;
    let document_id = DocumentId(Uuid::now_v7());

    PgSearchIndexQueueRepo::enqueue_document_in(db.conn(), workspace_id, document_id)
        .await
        .expect("enqueue_document_in must succeed");

    let claimed = PgSearchIndexQueueRepo::claim_batch(db.conn(), 10, 60)
        .await
        .expect("claim_batch must not error");
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].resource_id, document_id.0);

    PgSearchIndexQueueRepo::complete(db.conn(), claimed[0].id, claimed[0].enqueued_at)
        .await
        .expect("complete must succeed");

    let remaining = PgSearchIndexQueueRepo::claim_batch(db.conn(), 10, 60)
        .await
        .expect("claim_batch must not error");
    assert!(remaining.is_empty());

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn purge_operation_repo_create_record_attempt_and_digest_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "purge-op-test").await;
    let user_id = seed_user(&db, "purge-op-actor").await;

    // `purge_operations.commit_audit_id` is a foreign key into
    // `custos.security_audit_log`: seed a real row rather than a random UUID.
    let commit_audit_id = Uuid::now_v7();
    sea_orm::ConnectionTrait::execute_unprepared(
        db.conn(),
        &format!(
            "INSERT INTO custos.security_audit_log \
             (id, workspace_id, actor_user_id, action, target_type) \
             VALUES ('{commit_audit_id}', '{}', '{}', 'purge.commit', 'project')",
            workspace_id.0, user_id.0
        ),
    )
    .await
    .expect("seed security audit log row");

    let repo = PgPurgeOperationRepo;
    let target = RestoreTarget {
        kind: TrashKind::Project,
        target_id: Uuid::now_v7(),
    };

    let created = repo
        .create_in(
            db.conn(),
            NewPurgeOperation {
                workspace_id,
                target: target.clone(),
                original_actor_user_id: user_id,
                commit_audit_id: SecurityAuditRef(commit_audit_id),
            },
        )
        .await
        .expect("purge operation must be created");
    assert_eq!(created.workspace_id, workspace_id);

    let found = repo
        .find_by_target_in(db.conn(), workspace_id, &target)
        .await
        .expect("find_by_target_in must not error")
        .expect("purge operation must exist");
    assert_eq!(found.id, created.id);

    let digest = repo
        .create_digest_in(db.conn(), created.id, "sha256:deadbeef".to_string())
        .await
        .expect("create_digest_in must succeed");
    assert_eq!(digest.digest, "sha256:deadbeef");

    let digests = repo
        .list_digests_in(db.conn(), created.id)
        .await
        .expect("list_digests_in must not error");
    assert_eq!(digests.len(), 1);

    let candidates = repo
        .list_cleanup_candidates_in(db.conn(), 10)
        .await
        .expect("list_cleanup_candidates_in must not error");
    assert!(candidates.iter().any(|op| op.id == created.id));

    db.teardown().await.expect("teardown must succeed");
}

#[tokio::test]
async fn property_definition_repo_create_find_list_and_soft_delete_round_trip() {
    let db = TestDb::create().await.expect("test db must be created");
    let workspace_id = seed_workspace(&db, "property-def-test").await;
    let user_id = seed_user(&db, "property-def-owner").await;
    let ctx = WorkspaceCtx::new(workspace_id, Actor::User(UserAttributionId(user_id.0)));
    let repo = PgPropertyDefinitionRepo {
        conn: db.conn().clone(),
    };

    let created = repo
        .create(
            &ctx,
            NewPropertyDefinition {
                key: "priority".to_string(),
                name: "Priority".to_string(),
                kind: PropertyKind::Select,
                options: None,
                applies_to: AppliesTo::Task,
            },
        )
        .await
        .expect("property definition must be created");
    assert_eq!(created.key, "priority");

    let found = repo
        .find(&ctx, created.id)
        .await
        .expect("find must not error")
        .expect("property definition must exist");
    assert_eq!(found.id, created.id);

    let listed = repo.list(&ctx).await.expect("list must not error");
    assert_eq!(listed.len(), 1);

    repo.soft_delete(&ctx, created.id)
        .await
        .expect("soft_delete must succeed");
    let listed_after_delete = repo.list(&ctx).await.expect("list must not error");
    assert!(listed_after_delete.is_empty());

    db.teardown().await.expect("teardown must succeed");
}
