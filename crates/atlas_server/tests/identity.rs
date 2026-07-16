#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::{documents::NewDocument, identity::MemberRole},
};
use atlas_server::persistence::repos::{
    DocumentRepo, MembershipRepo, NewSession, NewUser, PgDocumentRepo, SessionRepo, UserRepo,
};
use chrono::Duration;
use sea_orm::ConnectionTrait;

#[tokio::test]
async fn user_password_hash_is_not_plaintext() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let repo = db.user_repo();

    let user = repo
        .create(NewUser {
            username: "alice".into(),
            display_name: "Alice".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    assert_ne!(user.password_hash.as_deref(), Some("password"));
    db.teardown().await;
}

#[tokio::test]
async fn session_token_hash_lookup() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, user) = support::seed_workspace(&db, "bob").await;
    let repo = db.session_repo();

    let token_hash = "abc123hash".to_string();
    let session = repo
        .create(NewSession {
            user_id: user.id,
            token_hash: token_hash.clone(),
            expires_at: chrono::Utc::now() + Duration::hours(1),
        })
        .await
        .expect("create session");

    let found = repo
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find session");
    assert!(found.is_some());
    assert_eq!(found.expect("found").id, session.id);

    let _ = ws;
    db.teardown().await;
}

#[tokio::test]
async fn update_role_changes_role_and_bumps_updated_at() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let (ws, owner_user) = support::seed_workspace(&db, "role-change-owner").await;
    let member = db
        .user_repo()
        .create(NewUser {
            username: "role-change-member".into(),
            display_name: "role-change-member".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member");

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(member.id));
    let original = db
        .membership_repo()
        .add(&ctx, member.id, MemberRole::Member)
        .await
        .expect("add membership");

    let updated = db
        .membership_repo()
        .update_role(&ctx, member.id, MemberRole::Admin)
        .await
        .expect("update_role");

    assert_eq!(updated.role, MemberRole::Admin, "role must change to Admin");
    assert_eq!(updated.id, original.id, "membership id must be preserved");
    assert_eq!(
        updated.user_id, original.user_id,
        "user_id must be preserved"
    );
    assert_eq!(
        updated.created_at, original.created_at,
        "created_at must not change"
    );
    assert!(
        updated.updated_at >= original.updated_at,
        "updated_at must be bumped"
    );

    let _ = owner_user;
    db.teardown().await;
}

#[tokio::test]
async fn update_role_on_non_member_returns_not_found() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let (ws, owner_user) = support::seed_workspace(&db, "role-not-found-owner").await;
    let stranger = db
        .user_repo()
        .create(NewUser {
            username: "role-not-found-stranger".into(),
            display_name: "role-not-found-stranger".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create stranger");

    let ctx = WorkspaceCtx::new(ws.id, Actor::User(stranger.id));
    let result = db
        .membership_repo()
        .update_role(&ctx, stranger.id, MemberRole::Admin)
        .await;

    assert!(
        matches!(result, Err(DomainError::NotFound { .. })),
        "update_role on non-member must return NotFound, got: {result:?}"
    );

    let _ = owner_user;
    db.teardown().await;
}

#[tokio::test]
async fn membership_removal_conflicts_while_the_member_has_retained_draft_state() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let (ws, owner) = support::seed_workspace(&db, "retained-draft-member-owner").await;
    let owner_ctx = WorkspaceCtx::new(ws.id, Actor::User(owner.id));
    let member = db
        .user_repo()
        .create(NewUser {
            username: "retained-draft-member".into(),
            display_name: "retained-draft-member".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create member");
    db.membership_repo()
        .add(&owner_ctx, member.id, MemberRole::Member)
        .await
        .expect("add member");
    let document = PgDocumentRepo::new(db.conn().clone(), 10)
        .create(
            &owner_ctx,
            NewDocument {
                title: "Retained draft member document".into(),
                slug: None,
                content: String::new(),
                folder_id: None,
                project_id: None,
                frontmatter: None,
            },
        )
        .await
        .expect("create document");
    let draft_id = uuid::Uuid::now_v7();
    db.conn()
        .execute_unprepared(&format!(
            "INSERT INTO comment_attachment_drafts \
             (id, workspace_id, document_id, created_by_user_id, create_token, create_digest, state, expires_at) \
             VALUES ('{draft_id}', '{}', '{}', '{}', '{draft_id}', '\\x{}', 'cancelled', now() + interval '24 hours')",
            ws.id.0,
            document.id.0,
            member.id.0,
            "01".repeat(32),
        ))
        .await
        .expect("seed retained member draft");

    let result = db.membership_repo().remove(&owner_ctx, member.id).await;
    assert!(
        matches!(result, Err(DomainError::CommentDraftConflict { .. })),
        "membership removal must preserve the retained draft state, got: {result:?}"
    );

    db.teardown().await;
}
