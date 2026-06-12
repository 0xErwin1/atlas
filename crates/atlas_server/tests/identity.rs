#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_server::persistence::repos::{NewSession, NewUser, SessionRepo, UserRepo};
use chrono::Duration;

#[tokio::test]
async fn user_password_hash_is_not_plaintext() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let repo = db.user_repo();

    let user = repo
        .create(NewUser {
            username: "alice".into(),
            display_name: "Alice".into(),
            password_hash: "$argon2id$v=19$m=19456,t=2,p=1$test$hash".into(),
            is_root: false,
        })
        .await
        .expect("create user");

    assert_ne!(user.password_hash, "password");
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
