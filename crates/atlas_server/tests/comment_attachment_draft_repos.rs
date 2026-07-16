#![allow(clippy::expect_used)]

mod support;

use atlas_domain::{
    Actor, AttachmentId, DomainError, WorkspaceCtx,
    entities::{
        comments::{
            CommentAttachmentDraftState, CommentDraftMetadata, CommentOwner,
            NewCommentAttachmentDraft, NewCommentAttachmentDraftUpload,
        },
        documents::NewDocument,
        identity::{ApiKeyType, NewApiKey},
    },
    ports::CommentAttachmentDraftRepo,
};
use atlas_server::persistence::repos::{
    ApiKeyRepo, DocumentRepo, NewUser, PgCommentAttachmentDraftRepo, PgDocumentRepo, UserRepo,
};
use atlas_server::services::CommentDraftService;
use chrono::{Duration, Utc};
use migration::Migrator;
use sea_orm::ConnectionTrait;
use sea_orm_migration::prelude::MigratorTrait;
use sha2::{Digest, Sha256};
use support::TestDb;

async fn seed_document(
    db: &TestDb,
    ctx: &atlas_domain::WorkspaceCtx,
    title: &str,
) -> atlas_domain::entities::documents::Document {
    PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            ctx,
            NewDocument {
                title: title.into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("seed document")
}

fn create_request(owner: CommentOwner, token: &str, digest: u8) -> NewCommentAttachmentDraft {
    NewCommentAttachmentDraft {
        id: atlas_domain::CommentDraftId::new(),
        owner,
        create_token: token.into(),
        create_digest: vec![digest; 32],
        expires_at: Utc::now() + Duration::hours(24),
    }
}

fn proposed_create_request(
    workspace_id: uuid::Uuid,
    id: atlas_domain::CommentDraftId,
    owner: CommentOwner,
    token: &str,
) -> NewCommentAttachmentDraft {
    let create_digest = Sha256::digest(
        atlas_domain::entities::comments::comment_draft_create_digest_input(
            workspace_id,
            id.0,
            token,
        ),
    )
    .to_vec();

    NewCommentAttachmentDraft {
        id,
        owner,
        create_token: token.into(),
        create_digest,
        expires_at: Utc::now() + Duration::hours(24),
    }
}

fn upload_request(
    attachment_id: Option<AttachmentId>,
    token: &str,
    request_digest: u8,
) -> NewCommentAttachmentDraftUpload {
    NewCommentAttachmentDraftUpload {
        attachment_id,
        upload_token: token.into(),
        request_digest: vec![request_digest; 32],
        payload_digest: vec![9; 32],
        metadata: CommentDraftMetadata::normalize("report.pdf", "application/pdf")
            .expect("valid attachment metadata"),
        size_bytes: 12,
    }
}

async fn seed_draft_attachment(
    db: &TestDb,
    workspace_id: uuid::Uuid,
    user_id: uuid::Uuid,
    draft_id: uuid::Uuid,
) -> AttachmentId {
    let attachment_id = AttachmentId::new();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments (id, workspace_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{}', '{workspace_id}', '{draft_id}', 'report.pdf', 'application/pdf', 12, 'digest', '{user_id}')",
            attachment_id.0,
        ))
        .await
        .expect("seed draft attachment");
    attachment_id
}

#[tokio::test]
async fn create_replays_a_create_token_for_the_same_parent() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-replay").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());

    let first_id = atlas_domain::CommentDraftId::new();
    let replay_id = atlas_domain::CommentDraftId::new();
    let first = repo
        .create_or_replay(
            &ctx,
            proposed_create_request(
                workspace.id.0,
                first_id,
                CommentOwner::Document(document.id),
                "create-token",
            ),
        )
        .await
        .expect("create draft");
    let replay = repo
        .create_or_replay(
            &ctx,
            proposed_create_request(
                workspace.id.0,
                replay_id,
                CommentOwner::Document(document.id),
                "create-token",
            ),
        )
        .await
        .expect("replay draft");

    assert_eq!(first.id, replay.id);
    assert_eq!(first.id, first_id);
    assert_ne!(replay.id, replay_id);
    assert_eq!(first.state, CommentAttachmentDraftState::Active);

    db.teardown().await;
}

#[tokio::test]
async fn create_replays_the_persisted_winner_for_distinct_proposed_ids() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-proposed-id-replay").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let owner = CommentOwner::Document(document.id);
    let winner_id = atlas_domain::CommentDraftId::new();
    let losing_id = atlas_domain::CommentDraftId::new();

    let winner = repo
        .create_or_replay(
            &ctx,
            proposed_create_request(workspace.id.0, winner_id, owner, "create-token"),
        )
        .await
        .expect("create winner");
    let replay = repo
        .create_or_replay(
            &ctx,
            proposed_create_request(workspace.id.0, losing_id, owner, "create-token"),
        )
        .await
        .expect("replay persisted winner");

    assert_eq!(winner.id, winner_id);
    assert_eq!(replay.id, winner_id);
    assert_ne!(replay.id, losing_id);

    db.teardown().await;
}

#[tokio::test]
async fn concurrent_distinct_proposed_ids_replay_the_insert_race_winner() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-proposed-id-race").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let owner = CommentOwner::Document(document.id);

    let first = repo.create_or_replay(
        &ctx,
        proposed_create_request(
            workspace.id.0,
            atlas_domain::CommentDraftId::new(),
            owner,
            "create-token",
        ),
    );
    let second = repo.create_or_replay(
        &ctx,
        proposed_create_request(
            workspace.id.0,
            atlas_domain::CommentDraftId::new(),
            owner,
            "create-token",
        ),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first create or replay");
    let second = second.expect("second create or replay");

    assert_eq!(first.id, second.id);

    db.teardown().await;
}

#[tokio::test]
async fn service_proposes_a_reserved_id_and_create_digest() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-service-create").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let service = CommentDraftService::new(std::sync::Arc::new(PgCommentAttachmentDraftRepo::new(
        db.conn().clone(),
    )));

    let draft = service
        .create_or_replay(
            &ctx,
            CommentOwner::Document(document.id),
            "create-token".into(),
            Utc::now() + Duration::hours(24),
        )
        .await
        .expect("service creates draft");
    let expected_digest = Sha256::digest(
        atlas_domain::entities::comments::comment_draft_create_digest_input(
            workspace.id.0,
            draft.id.0,
            "create-token",
        ),
    )
    .to_vec();

    assert_eq!(draft.create_digest, expected_digest);

    db.teardown().await;
}

#[tokio::test]
async fn exact_owner_lookup_hides_a_draft_from_a_different_parent() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-owner-lookup").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");

    let hidden = repo
        .get_for_owner_and_creator(
            &ctx,
            CommentOwner::Task(atlas_domain::TaskId::new()),
            draft.id,
        )
        .await
        .expect("scoped lookup");

    assert!(
        hidden.is_none(),
        "drafts must be scoped to their exact parent"
    );

    db.teardown().await;
}

#[tokio::test]
async fn draft_resolution_conceals_api_key_and_workspace_mismatches() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-concealment").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let api_key = db
        .api_key_repo()
        .create(
            &ctx,
            NewApiKey {
                name: "draft-concealment-key".into(),
                token_hash: format!("hash-{}", uuid::Uuid::now_v7()),
                type_: ApiKeyType::Agent,
                expires_at: None,
                scopes: Vec::new(),
            },
        )
        .await
        .expect("create api key");
    let api_key_ctx = WorkspaceCtx::new(workspace.id, Actor::ApiKey(api_key.id));
    let (other_workspace, _) = support::seed_workspace(&db, "draft-other-workspace").await;
    let other_workspace_ctx = WorkspaceCtx::new(other_workspace.id, Actor::User(user.id));

    let api_key_lookup = repo
        .get_for_owner_and_creator(&api_key_ctx, CommentOwner::Document(document.id), draft.id)
        .await
        .expect("api key lookup");
    let workspace_lookup = repo
        .get_for_owner_and_creator(
            &other_workspace_ctx,
            CommentOwner::Document(document.id),
            draft.id,
        )
        .await
        .expect("workspace lookup");
    let api_key_upload = repo
        .record_upload_or_replay(
            &api_key_ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(None, "upload-token", 1),
        )
        .await;
    let workspace_upload = repo
        .record_upload_or_replay(
            &other_workspace_ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(None, "upload-token", 1),
        )
        .await;

    assert!(api_key_lookup.is_none());
    assert!(workspace_lookup.is_none());
    assert!(matches!(api_key_upload, Err(DomainError::NotFound { .. })));
    assert!(matches!(
        workspace_upload,
        Err(DomainError::NotFound { .. })
    ));

    db.teardown().await;
}

#[tokio::test]
async fn create_token_reuse_across_parents_conflicts() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-cross-parent-token").await;
    let ctx = support::ctx(&workspace, &user);
    let first_document = seed_document(&db, &ctx, "First draft parent").await;
    let second_document = seed_document(&db, &ctx, "Second draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let first = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(first_document.id), "create-token", 1),
        )
        .await
        .expect("create first draft");
    let reuse = repo
        .create_or_replay(
            &ctx,
            create_request(
                CommentOwner::Document(second_document.id),
                "create-token",
                1,
            ),
        )
        .await;
    let second = repo
        .get_for_owner_and_creator(&ctx, CommentOwner::Document(second_document.id), first.id)
        .await
        .expect("second parent lookup");

    assert!(matches!(
        reuse,
        Err(DomainError::CommentDraftConflict { .. })
    ));
    assert!(second.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn migration_enforces_digest_sizes_and_allows_drained_rollback() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-migration-guards").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");

    let live_upload_without_attachment = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, request_digest, payload_digest, file_name, content_type, size_bytes) \
              VALUES ('{}', 'upload-token', '\\x01', '\\x02', 'report.pdf', 'application/pdf', 12)",
            draft.id.0,
        ))
        .await;
    let attachment_with_two_owners = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO attachments (id, workspace_id, document_id, draft_id, file_name, content_type, size_bytes, sha256, created_by_user_id) \
             VALUES ('{}', '{}', '{}', '{}', 'report.pdf', 'application/pdf', 12, 'digest', '{}')",
            uuid::Uuid::now_v7(),
            workspace.id.0,
            document.id.0,
            draft.id.0,
            user.id.0,
        ))
        .await;
    let invalid_draft_digest = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_drafts \\
             (id, workspace_id, document_id, created_by_user_id, create_token, create_digest, state, expires_at) \\
             VALUES ('{}', '{}', '{}', '{}', 'short-digest', '\\x01', 'active', now() + interval '1 day')",
            uuid::Uuid::now_v7(), workspace.id.0, document.id.0, user.id.0,
        )
        .replace("\\\n", " "))
        .await;
    let attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;
    let invalid_upload_digest = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_draft_uploads \\
              (draft_id, upload_token, original_attachment_id, attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes) \\
              VALUES ('{}', 'short-upload-digest', '{}', '{}', '\\x01', '\\x{}', 'report.pdf', 'application/pdf', 12)",
            draft.id.0,
            attachment.0,
            attachment.0,
            "02".repeat(32),
        )
        .replace("\\\n", " "))
        .await;
    let comment_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(
            &format!(
                "INSERT INTO comments \\
                 (id, workspace_id, document_id, body, created_by_user_id, created_at, updated_at) \\
                 VALUES ('{comment_id}', '{}', '{}', 'finalized body', '{}', now(), now())",
                workspace.id.0, document.id.0, user.id.0,
            )
            .replace("\\\n", " "),
        )
        .await
        .expect("seed finalized comment");
    let missing_final_comment = db
        .conn()
        .execute_unprepared(
            &format!(
                "UPDATE comment_attachment_drafts SET finalized_comment_id = '{}' WHERE id = '{}'",
                uuid::Uuid::now_v7(),
                draft.id.0,
            )
            .replace("\\\n", " "),
        )
        .await;
    let invalid_final_digest = db
        .conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts SET final_body_digest = '\\x01' WHERE id = '{}'",
            draft.id.0,
        ))
        .await;
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts \\
             SET finalized_comment_id = '{comment_id}', final_body_digest = '\\x{}', final_request_digest = '\\x{}' \\
             WHERE id = '{}'",
            "03".repeat(32),
            "04".repeat(32),
            draft.id.0,
        )
        .replace("\\\n", ""))
        .await
        .expect("persist terminal replay identity");
    let finalized = repo
        .get_for_owner_and_creator(&ctx, CommentOwner::Document(document.id), draft.id)
        .await
        .expect("lookup finalized draft")
        .expect("draft exists");
    let tombstone_without_attachment = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_draft_uploads \
              (draft_id, upload_token, original_attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes, deleted_at) \
               VALUES ('{}', 'deleted-upload-token', '{}', '\\x{}', '\\x{}', 'report.pdf', 'application/pdf', 12, now())",
            draft.id.0,
            attachment.0,
            "01".repeat(32),
            "02".repeat(32),
        ))
        .await;
    let guarded_rollback = Migrator::down(db.conn(), Some(1)).await;

    assert!(
        live_upload_without_attachment.is_err(),
        "a live upload ledger row must reference its attachment"
    );
    assert!(
        attachment_with_two_owners.is_err(),
        "an attachment must have exactly one owner, including a draft owner"
    );
    assert!(
        tombstone_without_attachment.is_ok(),
        "tombstones must retain replay identity after their attachment is removed"
    );
    assert!(
        invalid_draft_digest.is_err(),
        "draft replay digests must be exactly 32 bytes"
    );
    assert!(
        invalid_upload_digest.is_err(),
        "upload replay digests must be exactly 32 bytes"
    );
    assert!(
        missing_final_comment.is_err(),
        "finalized comments must retain a restrict foreign key"
    );
    assert!(
        invalid_final_digest.is_err(),
        "terminal replay digests must be exactly 32 bytes"
    );
    assert_eq!(
        finalized.finalized_comment_id.map(|id| id.0),
        Some(comment_id)
    );
    assert_eq!(finalized.final_body_digest, Some(vec![3; 32]));
    assert_eq!(finalized.final_request_digest, Some(vec![4; 32]));
    assert!(
        guarded_rollback.is_err(),
        "rollback must be guarded while drafts remain"
    );

    db.conn()
        .execute_unprepared(&format!(
            "DELETE FROM comment_attachment_draft_uploads WHERE draft_id = '{}'",
            draft.id.0,
        ))
        .await
        .expect("drain upload ledger");
    db.conn()
        .execute_unprepared(&format!(
            "DELETE FROM attachments WHERE draft_id = '{}'",
            draft.id.0,
        ))
        .await
        .expect("drain draft attachment");
    db.conn()
        .execute_unprepared(&format!(
            "DELETE FROM comment_attachment_drafts WHERE id = '{}'",
            draft.id.0,
        ))
        .await
        .expect("drain draft");
    db.conn()
        .execute_unprepared(&format!("DELETE FROM comments WHERE id = '{comment_id}'",))
        .await
        .expect("drain finalized comment");

    let drained_rollback = Migrator::down(db.conn(), Some(1)).await;
    assert!(
        drained_rollback.is_ok(),
        "rollback must succeed after all draft state is drained"
    );

    db.teardown().await;
}

#[tokio::test]
async fn upload_replays_identical_token_and_rejects_changed_request() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-upload-replay").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;

    let first = repo
        .record_upload_or_replay(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(Some(attachment), "upload-token", 1),
        )
        .await
        .expect("record upload");
    let provisional = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;
    let replay = repo
        .record_upload_or_replay(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(Some(provisional), "upload-token", 1),
        )
        .await
        .expect("replay upload");
    let conflicting_provisional =
        seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;
    let conflict = repo
        .record_upload_or_replay(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(Some(conflicting_provisional), "upload-token", 2),
        )
        .await;
    let provisional_survives = db
        .conn()
        .execute_unprepared(&format!(
            "DELETE FROM attachments WHERE id IN ('{}', '{}')",
            provisional.0, conflicting_provisional.0,
        ))
        .await
        .expect("check provisional attachment cleanup");

    assert_eq!(first.attachment_id, Some(attachment));
    assert_eq!(replay.attachment_id, Some(attachment));
    assert!(matches!(
        conflict,
        Err(DomainError::CommentDraftConflict { .. })
    ));
    assert_eq!(
        provisional_survives.rows_affected(),
        0,
        "replay and conflict must remove provisional draft attachments"
    );

    db.teardown().await;
}

#[tokio::test]
async fn upload_tombstone_is_gone_and_other_principals_cannot_resolve_it() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-upload-tombstone").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;

    repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(attachment), "upload-token", 1),
    )
    .await
    .expect("record upload");
    repo.tombstone_upload(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        "upload-token",
    )
    .await
    .expect("tombstone upload");
    let tombstone = repo
        .get_upload_for_original_attachment_id(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            attachment,
        )
        .await
        .expect("find retained tombstone")
        .expect("tombstone exists");

    let replay = repo
        .record_upload_or_replay(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(None, "upload-token", 1),
        )
        .await;
    let other_user = db
        .user_repo()
        .create(NewUser {
            username: "other-draft-uploader".into(),
            display_name: "Other Draft Uploader".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create other user");
    let other_ctx = WorkspaceCtx::new(workspace.id, Actor::User(other_user.id));
    let hidden = repo
        .record_upload_or_replay(
            &other_ctx,
            CommentOwner::Document(document.id),
            draft.id,
            upload_request(None, "other-token", 1),
        )
        .await;

    assert!(matches!(replay, Err(DomainError::CommentDraftGone { .. })));
    assert!(matches!(hidden, Err(DomainError::NotFound { .. })));
    assert_eq!(tombstone.original_attachment_id, attachment);
    assert!(tombstone.attachment_id.is_none());
    assert!(tombstone.deleted_at.is_some());

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE comment_attachment_drafts SET state = 'finalized' WHERE id = '{}'",
            draft.id.0,
        ))
        .await
        .expect("finalize draft");
    let finalized_tombstone = repo
        .get_upload_for_original_attachment_id(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            attachment,
        )
        .await
        .expect("find finalized tombstone")
        .expect("finalized tombstone exists");
    let duplicate_original = db
        .conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_draft_uploads \
             (draft_id, upload_token, original_attachment_id, request_digest, payload_digest, file_name, content_type, size_bytes, deleted_at) \
             VALUES ('{}', 'duplicate-original', '{}', '\\x{}', '\\x{}', 'report.pdf', 'application/pdf', 12, now())",
            draft.id.0,
            attachment.0,
            "01".repeat(32),
            "02".repeat(32),
        ))
        .await;
    db.conn()
        .execute_unprepared(&format!(
            "DELETE FROM comment_attachment_draft_uploads WHERE draft_id = '{}' AND upload_token = 'upload-token'",
            draft.id.0,
        ))
        .await
        .expect("prune retained tombstone");
    let pruned = repo
        .get_upload_for_original_attachment_id(
            &ctx,
            CommentOwner::Document(document.id),
            draft.id,
            attachment,
        )
        .await
        .expect("lookup pruned tombstone");

    assert_eq!(finalized_tombstone.original_attachment_id, attachment);
    assert!(duplicate_original.is_err());
    assert!(pruned.is_none());

    db.teardown().await;
}

#[tokio::test]
async fn draft_attachment_then_ledger_lock_order_serializes_identical_uploads() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-upload-lock-order").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;

    let first = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(attachment), "upload-token", 1),
    );
    let second = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(attachment), "upload-token", 1),
    );
    let (first, second) = tokio::join!(first, second);

    assert_eq!(
        first.expect("first upload").attachment_id,
        second.expect("second upload replay").attachment_id
    );

    db.teardown().await;
}

#[tokio::test]
async fn concurrent_distinct_provisionals_replay_without_orphans() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-concurrent-replay").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let first_attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;
    let second_attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;

    let first = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(first_attachment), "upload-token", 1),
    );
    let second = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(second_attachment), "upload-token", 1),
    );
    let (first, second) = tokio::join!(first, second);
    let first = first.expect("first upload");
    let second = second.expect("identical replay");
    let losing_attachment = if first.attachment_id == Some(first_attachment) {
        second_attachment
    } else {
        first_attachment
    };
    let orphan = db
        .conn()
        .execute_unprepared(&format!(
            "DELETE FROM attachments WHERE id = '{losing_attachment}'"
        ))
        .await
        .expect("check losing provisional cleanup");

    assert_eq!(first.attachment_id, second.attachment_id);
    assert_eq!(
        orphan.rows_affected(),
        0,
        "the losing provisional attachment must not survive"
    );

    db.teardown().await;
}

#[tokio::test]
async fn concurrent_conflicting_provisionals_leave_only_the_winning_attachment() {
    let db = TestDb::create().await.expect("test database");
    let (workspace, user) = support::seed_workspace(&db, "draft-concurrent-conflict").await;
    let ctx = support::ctx(&workspace, &user);
    let document = seed_document(&db, &ctx, "Draft parent").await;
    let repo = PgCommentAttachmentDraftRepo::new(db.conn().clone());
    let draft = repo
        .create_or_replay(
            &ctx,
            create_request(CommentOwner::Document(document.id), "create-token", 1),
        )
        .await
        .expect("create draft");
    let first_attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;
    let second_attachment = seed_draft_attachment(&db, workspace.id.0, user.id.0, draft.id.0).await;

    let first = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(first_attachment), "upload-token", 1),
    );
    let second = repo.record_upload_or_replay(
        &ctx,
        CommentOwner::Document(document.id),
        draft.id,
        upload_request(Some(second_attachment), "upload-token", 2),
    );
    let (first, second) = tokio::join!(first, second);
    let results = [first, second];
    let winning_attachment = results
        .iter()
        .find_map(|result| result.as_ref().ok().and_then(|upload| upload.attachment_id))
        .expect("one upload must win");
    let losing_attachment = if winning_attachment == first_attachment {
        second_attachment
    } else {
        first_attachment
    };
    let orphan = db
        .conn()
        .execute_unprepared(&format!(
            "DELETE FROM attachments WHERE id = '{losing_attachment}'"
        ))
        .await
        .expect("check conflicting provisional cleanup");

    assert_eq!(
        results.iter().filter(|result| result.is_ok()).count(),
        1,
        "exactly one request must win the serialized upload token"
    );
    assert_eq!(
        results
            .iter()
            .filter(|result| matches!(result, Err(DomainError::CommentDraftConflict { .. })))
            .count(),
        1,
        "the changed digest must conflict after the winner commits"
    );
    assert_eq!(
        orphan.rows_affected(),
        0,
        "the conflicting provisional attachment must not survive"
    );

    db.teardown().await;
}
