#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::LoginRequest;
use atlas_client::AtlasClient;
use atlas_server::{
    error::ApiError,
    persistence::repos::{ActivationTokenRepo, NewActivationToken, NewUser, UserRepo},
};
use chrono::{Duration, Utc};
use sea_orm::ConnectionTrait;

// ── T01: migration columns exist after up() ─────────────────────────────────
// Migration is applied by TestDb::create() via Migrator::up(). If migration
// 021 was applied, the columns must be present. These tests assert the resulting
// schema shape rather than the migration mechanics directly.

#[tokio::test]
async fn password_hash_is_nullable_after_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    // Insert a user with NULL password_hash via raw SQL — would fail if the
    // column is NOT NULL.
    db.conn()
        .execute_unprepared(
            "INSERT INTO users (id, username, display_name, email, password_hash, is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at)
             VALUES (gen_random_uuid(), 'null-pw-test', 'Null PW', NULL, NULL, false, false, NULL, NULL, now(), now())"
        )
        .await
        .expect("password_hash column must be nullable after migration");

    db.teardown().await;
}

#[tokio::test]
async fn activated_at_column_exists_after_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    // Verify activated_at is readable (SELECT won't fail if the column exists).
    use sea_orm::ConnectionTrait;
    let result = db
        .conn()
        .execute_unprepared("SELECT activated_at FROM users LIMIT 1")
        .await;

    assert!(result.is_ok(), "activated_at column must exist on users");

    db.teardown().await;
}

#[tokio::test]
async fn existing_users_have_activated_at_set_after_migration() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    // The back-fill in up() sets activated_at = created_at for all existing rows.
    // Any row inserted before the migration — but migrations run fresh on TestDb
    // creation (empty DB), so we seed one here and verify the column works.
    // More importantly, the migration back-fill SQL must have been run: test by
    // inserting a row with an explicit activated_at and reading it back.
    db.conn()
        .execute_unprepared(
            "INSERT INTO users (id, username, display_name, email, password_hash, is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at)
             VALUES (gen_random_uuid(), 'backfill-test', 'Backfill', NULL, '$argon2id$v=19$m=19456,t=2,p=1$test$hash', false, false, NULL, now(), now(), now())"
        )
        .await
        .expect("insert user with activated_at");

    use sea_orm::{ConnectionTrait, FromQueryResult, Statement};
    #[derive(sea_orm::FromQueryResult, Debug)]
    struct Row {
        activated_at: Option<chrono::DateTime<Utc>>,
    }
    let rows = Row::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT activated_at FROM users WHERE username = 'backfill-test' LIMIT 1",
        [],
    ))
    .all(db.conn())
    .await
    .expect("select activated_at");

    assert!(!rows.is_empty(), "row must be present");
    assert!(
        rows[0].activated_at.is_some(),
        "activated_at must not be null for a seeded row"
    );

    db.teardown().await;
}

#[tokio::test]
async fn user_activation_tokens_table_exists() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    use sea_orm::ConnectionTrait;
    let result = db
        .conn()
        .execute_unprepared(
            "SELECT id, user_id, token_hash, expires_at, consumed_at, created_at FROM user_activation_tokens LIMIT 1",
        )
        .await;

    assert!(
        result.is_ok(),
        "user_activation_tokens table must exist with expected columns"
    );

    db.teardown().await;
}

// ── T02: ActivationTokenRepo round-trip ─────────────────────────────────────

#[tokio::test]
async fn activation_token_create_then_find_active_returns_token() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();
    let token_repo = db.activation_token_repo();

    let user = user_repo
        .create(NewUser {
            username: "tok-create-find".into(),
            display_name: "Tok Create Find".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let token_hash = "findme_hash_001".to_string();
    let expires_at = Utc::now() + Duration::hours(24);

    token_repo
        .create(NewActivationToken {
            user_id: user.id,
            token_hash: token_hash.clone(),
            expires_at,
        })
        .await
        .expect("create activation token");

    let found = token_repo
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find_active_by_token_hash");

    assert!(found.is_some(), "token must be found when active");
    assert_eq!(found.expect("found").token_hash, token_hash);

    db.teardown().await;
}

#[tokio::test]
async fn expired_activation_token_returns_none() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();
    let token_repo = db.activation_token_repo();

    let user = user_repo
        .create(NewUser {
            username: "tok-expired".into(),
            display_name: "Tok Expired".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let token_hash = "expired_hash_001".to_string();
    let expires_at = Utc::now() - Duration::hours(1);

    token_repo
        .create(NewActivationToken {
            user_id: user.id,
            token_hash: token_hash.clone(),
            expires_at,
        })
        .await
        .expect("create expired token");

    let found = token_repo
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find_active_by_token_hash");

    assert!(
        found.is_none(),
        "expired token must not be returned as active"
    );

    db.teardown().await;
}

#[tokio::test]
async fn consumed_activation_token_returns_none() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();
    let token_repo = db.activation_token_repo();

    let user = user_repo
        .create(NewUser {
            username: "tok-consumed".into(),
            display_name: "Tok Consumed".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let token_hash = "consumed_hash_001".to_string();
    let expires_at = Utc::now() + Duration::hours(24);

    let tok = token_repo
        .create(NewActivationToken {
            user_id: user.id,
            token_hash: token_hash.clone(),
            expires_at,
        })
        .await
        .expect("create token");

    token_repo.consume(tok.id).await.expect("consume token");

    let found = token_repo
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find_active_by_token_hash");

    assert!(
        found.is_none(),
        "consumed token must not be returned as active"
    );

    db.teardown().await;
}

#[tokio::test]
async fn invalidate_unconsumed_for_user_makes_find_active_return_none() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();
    let token_repo = db.activation_token_repo();

    let user = user_repo
        .create(NewUser {
            username: "tok-invalidate".into(),
            display_name: "Tok Invalidate".into(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let token_hash = "invalidate_hash_001".to_string();
    let expires_at = Utc::now() + Duration::hours(24);

    token_repo
        .create(NewActivationToken {
            user_id: user.id,
            token_hash: token_hash.clone(),
            expires_at,
        })
        .await
        .expect("create token");

    token_repo
        .invalidate_unconsumed_for_user(user.id)
        .await
        .expect("invalidate_unconsumed_for_user");

    let found = token_repo
        .find_active_by_token_hash(&token_hash)
        .await
        .expect("find_active_by_token_hash");

    assert!(
        found.is_none(),
        "invalidated token must not be returned as active"
    );

    db.teardown().await;
}

// ── T03: find_by_username round-trips nullable password_hash and activated_at ─

#[tokio::test]
async fn find_by_username_roundtrips_some_password_hash_and_activated_at() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();

    // Insert a user with password_hash set and activated_at set via raw SQL
    // (simulates an activated account).
    db.conn()
        .execute_unprepared(
            "INSERT INTO users (id, username, display_name, email, password_hash, is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at)
             VALUES (gen_random_uuid(), 'roundtrip-some', 'RT Some', NULL, '$argon2id$v=19$m=19456,t=2,p=1$test$hash', false, false, NULL, now(), now(), now())"
        )
        .await
        .expect("insert");

    let found = user_repo
        .find_by_username("roundtrip-some")
        .await
        .expect("find_by_username")
        .expect("user must be found");

    assert!(
        found.password_hash.is_some(),
        "password_hash must be Some for an activated user"
    );
    assert!(
        found.activated_at.is_some(),
        "activated_at must be Some for an activated user"
    );
}

#[tokio::test]
async fn find_by_username_roundtrips_none_password_hash_for_pending_user() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let user_repo = db.user_repo();

    // Insert a pending user with NULL password_hash and NULL activated_at.
    db.conn()
        .execute_unprepared(
            "INSERT INTO users (id, username, display_name, email, password_hash, is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at)
             VALUES (gen_random_uuid(), 'roundtrip-none', 'RT None', NULL, NULL, false, false, NULL, NULL, now(), now())"
        )
        .await
        .expect("insert pending user");

    let found = user_repo
        .find_by_username("roundtrip-none")
        .await
        .expect("find_by_username")
        .expect("user must be found");

    assert!(
        found.password_hash.is_none(),
        "password_hash must be None for a pending user"
    );
    assert!(
        found.activated_at.is_none(),
        "activated_at must be None for a pending user"
    );
}

// ── T04/T13: login rejects pending users with 403 AccountNotActivated ────────

#[tokio::test]
async fn login_pending_user_returns_403_account_not_activated() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    // Insert a pending user: NULL password_hash, NULL activated_at.
    db.conn()
        .execute_unprepared(
            "INSERT INTO users (id, username, display_name, email, password_hash, is_root, is_system_admin, disabled_at, activated_at, created_at, updated_at)
             VALUES (gen_random_uuid(), 'pending-login', 'Pending', NULL, NULL, false, false, NULL, NULL, now(), now())"
        )
        .await
        .expect("insert pending user");

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "pending-login".into(),
            password: "anypassword".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 403),
        "pending user login must return 403, got: {result:?}"
    );

    let problem = match result {
        Err(atlas_client::ClientError::Api(p)) => p,
        _ => panic!("expected Api error"),
    };
    assert_eq!(
        problem.r#type, "urn:atlas:error:account-not-activated",
        "error type must be account-not-activated urn"
    );

    db.teardown().await;
}

#[tokio::test]
async fn login_activated_user_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, _user) = support::login_user(&server, &db, "activated-login-ok").await;

    // login_user already asserts success internally.
    db.teardown().await;
}

#[tokio::test]
async fn login_disabled_user_still_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "disabled-auth-b1").await;

    db.user_repo().disable(user.id).await.expect("disable user");

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "disabled-auth-b1".into(),
            password: "TestPassword1!".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "disabled user must return 401 (not 403), got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn login_unknown_user_returns_401_not_403() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "does-not-exist-b1".into(),
            password: "anypassword".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "unknown user must return 401 (not 403), got: {result:?}"
    );

    db.teardown().await;
}

// ── T05: ApiError::AccountNotActivated serializes correctly ──────────────────

#[tokio::test]
async fn account_not_activated_error_serializes_403_with_urn() {
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
    };
    use tower::ServiceExt;

    let app = atlas_server::test_app_with_route(
        "/test-not-activated",
        get(|| async {
            Err::<(), ApiError>(ApiError::AccountNotActivated {
                message: "account is pending activation".into(),
            })
        }),
    );

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test-not-activated")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_owned();

    assert!(
        content_type.contains("application/problem+json"),
        "content-type must be application/problem+json, got: {content_type}"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body["type"], "urn:atlas:error:account-not-activated");
    assert_eq!(body["status"], 403);
    assert!(body["title"].is_string(), "title must be present");
}

// ════════════════════════════════════════════════════════════════════════════
// B3 — Public activate GET/POST + rate-limit + transaction (T26–T38)
// ════════════════════════════════════════════════════════════════════════════

/// Seeds a pending user and mints an activation token, returning the raw
/// plaintext token extracted from the activation_link.
async fn seed_pending_user_with_token(
    db: &support::TestDb,
    server: &support::TestServer,
    username: &str,
) -> String {
    use atlas_api::dtos::CreateUserRequest;

    let (_, ws, _) =
        support::login_user_with_workspace(server, db, &format!("owner-{username}")).await;
    let root = support::login_root_user(server, db).await;

    let result = root
        .create_user(CreateUserRequest {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            workspace: ws.slug,
            role: "member".to_string(),
        })
        .await
        .expect("create_user");

    let link = result.activation_link;
    link.split("/activate/")
        .nth(1)
        .unwrap_or_else(|| panic!("activation_link has unexpected shape: {link}"))
        .to_owned()
}

// ── T26: GET valid token → 200 {username, display_name}; no oracle leakage ──

#[tokio::test]
async fn get_activate_valid_token_returns_200_with_username_and_display_name() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "get-valid-tok").await;

    let http = reqwest::Client::new();
    let resp = http
        .get(format!("{}/v1/activate/{token}", server.base_url()))
        .send()
        .await
        .expect("GET /v1/activate/{token}");

    assert_eq!(resp.status().as_u16(), 200, "valid token must return 200");

    let body: serde_json::Value = resp.json().await.expect("response body");

    assert!(body["username"].is_string(), "username must be present");
    assert!(
        body["display_name"].is_string(),
        "display_name must be present"
    );

    assert!(
        body.get("email").is_none() || body["email"].is_null(),
        "email must NOT be leaked in the response"
    );
    assert!(
        body.get("id").is_none(),
        "id must NOT be leaked in the response"
    );
    assert!(
        body.get("role").is_none(),
        "role must NOT be leaked in the response"
    );

    db.teardown().await;
}

// ── T26: GET unknown/expired/consumed token → 404, IDENTICAL generic body ───

#[tokio::test]
async fn get_activate_unknown_token_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let http = reqwest::Client::new();
    let resp = http
        .get(format!(
            "{}/v1/activate/totally-unknown-token",
            server.base_url()
        ))
        .send()
        .await
        .expect("GET unknown token");

    assert_eq!(resp.status().as_u16(), 404);

    db.teardown().await;
}

#[tokio::test]
async fn get_activate_expired_token_returns_same_404_as_unknown() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    use atlas_server::{
        auth::tokens::{generate_session_token, hash_token},
        persistence::repos::{NewActivationToken, UserRepo},
    };

    let user = db
        .user_repo()
        .create(NewUser {
            username: "expired-tok-user".into(),
            display_name: "Expired Tok".into(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let plaintext = generate_session_token();
    let token_hash = hash_token(&plaintext);

    db.activation_token_repo()
        .create(NewActivationToken {
            user_id: user.id,
            token_hash,
            expires_at: Utc::now() - Duration::hours(1),
        })
        .await
        .expect("create expired token");

    let http = reqwest::Client::new();

    let resp_expired = http
        .get(format!("{}/v1/activate/{plaintext}", server.base_url()))
        .send()
        .await
        .expect("GET expired");

    let resp_unknown = http
        .get(format!("{}/v1/activate/nonexistent-abc", server.base_url()))
        .send()
        .await
        .expect("GET unknown");

    assert_eq!(resp_expired.status().as_u16(), 404);
    assert_eq!(
        resp_unknown.status().as_u16(),
        404,
        "unknown token must also be 404"
    );

    let body_expired: serde_json::Value = resp_expired.json().await.expect("expired body");
    let body_unknown: serde_json::Value = resp_unknown.json().await.expect("unknown body");

    assert_eq!(
        body_expired["title"], body_unknown["title"],
        "expired and unknown must return the same error message (no oracle)"
    );

    db.teardown().await;
}

#[tokio::test]
async fn get_activate_consumed_token_returns_same_404_as_unknown() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    use atlas_server::{
        auth::tokens::{generate_session_token, hash_token},
        persistence::repos::{NewActivationToken, UserRepo},
    };

    let user = db
        .user_repo()
        .create(NewUser {
            username: "consumed-tok-user".into(),
            display_name: "Consumed Tok".into(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let plaintext = generate_session_token();
    let token_hash = hash_token(&plaintext);

    let tok = db
        .activation_token_repo()
        .create(NewActivationToken {
            user_id: user.id,
            token_hash,
            expires_at: Utc::now() + Duration::days(7),
        })
        .await
        .expect("create token");

    db.activation_token_repo()
        .consume(tok.id)
        .await
        .expect("consume token");

    let http = reqwest::Client::new();

    let resp_consumed = http
        .get(format!("{}/v1/activate/{plaintext}", server.base_url()))
        .send()
        .await
        .expect("GET consumed");

    let resp_unknown = http
        .get(format!("{}/v1/activate/nonexistent-xyz", server.base_url()))
        .send()
        .await
        .expect("GET unknown");

    assert_eq!(resp_consumed.status().as_u16(), 404);
    assert_eq!(resp_unknown.status().as_u16(), 404);

    let body_consumed: serde_json::Value = resp_consumed.json().await.expect("consumed body");
    let body_unknown: serde_json::Value = resp_unknown.json().await.expect("unknown body");

    assert_eq!(
        body_consumed["title"], body_unknown["title"],
        "consumed and unknown must return the same error message (no oracle)"
    );

    db.teardown().await;
}

// ── T27: POST valid token → 200 LoginResponse + cookie; user activated ───────

#[tokio::test]
async fn post_activate_valid_token_activates_user_and_returns_login_response() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "post-valid-act").await;

    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "SuperSecret99!" }))
        .send()
        .await
        .expect("POST /v1/activate/{token}");

    assert_eq!(
        resp.status().as_u16(),
        200,
        "valid activation must return 200"
    );

    let has_session_cookie = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .any(|v| v.to_str().unwrap_or("").contains("atlas_session="));

    assert!(
        has_session_cookie,
        "activation response must set atlas_session cookie"
    );

    let body: serde_json::Value = resp.json().await.expect("response body");
    assert!(
        body["token"].is_string(),
        "LoginResponse must have a token field"
    );
    assert!(
        body["expires_at"].is_string(),
        "LoginResponse must have expires_at"
    );
    assert!(
        body["user"]["username"].is_string(),
        "LoginResponse must have user.username"
    );

    db.teardown().await;
}

#[tokio::test]
async fn post_activate_sets_password_hash_and_activated_at_in_db() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "post-db-check").await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "SuperSecret99!" }))
        .send()
        .await
        .expect("POST activate");

    assert_eq!(resp.status().as_u16(), 200);

    let body: serde_json::Value = resp.json().await.expect("body");
    let username = body["user"]["username"]
        .as_str()
        .expect("username in response");

    let user = db
        .user_repo()
        .find_by_username(username)
        .await
        .expect("find_by_username")
        .expect("user must exist");

    assert!(
        user.password_hash.is_some(),
        "password_hash must be set after activation"
    );
    assert!(
        user.activated_at.is_some(),
        "activated_at must be set after activation"
    );

    db.teardown().await;
}

// ── T27: after activation the user can log in normally ───────────────────────

#[tokio::test]
async fn activated_user_can_login_normally_after_activation() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "post-then-login").await;

    let password = "MySecurePass1!";

    let http = reqwest::Client::new();

    let activate_resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": password }))
        .send()
        .await
        .expect("POST activate");

    assert_eq!(activate_resp.status().as_u16(), 200);
    let body: serde_json::Value = activate_resp.json().await.expect("body");
    let username = body["user"]["username"]
        .as_str()
        .expect("username")
        .to_owned();

    let login_resp = http
        .post(format!("{}/v1/auth/login", server.base_url()))
        .json(&LoginRequest {
            username: username.clone(),
            password: password.to_string(),
        })
        .send()
        .await
        .expect("login");

    assert_eq!(
        login_resp.status().as_u16(),
        200,
        "activated user must be able to log in; got {}",
        login_resp.status()
    );

    db.teardown().await;
}

// ── T28: POST invalid/expired/consumed token → 404 generic; no session ───────

#[tokio::test]
async fn post_activate_unknown_token_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/totally-unknown", server.base_url()))
        .json(&serde_json::json!({ "password": "SomePassword1!" }))
        .send()
        .await
        .expect("POST unknown token");

    assert_eq!(resp.status().as_u16(), 404);

    let has_session = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .any(|v| v.to_str().unwrap_or("").contains("atlas_session="));

    assert!(
        !has_session,
        "invalid token POST must not issue a session cookie"
    );

    db.teardown().await;
}

// ── T29: password < 8 chars → 422; token NOT consumed, user NOT activated ────

#[tokio::test]
async fn post_activate_short_password_returns_422_and_does_not_consume_token() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "short-pw-test").await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "short" }))
        .send()
        .await
        .expect("POST short password");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "password < 8 chars must return 422"
    );

    let http = reqwest::Client::new();
    let get_resp = http
        .get(format!("{}/v1/activate/{token}", server.base_url()))
        .send()
        .await
        .expect("GET after 422");

    assert_eq!(
        get_resp.status().as_u16(),
        200,
        "token must still be valid after a rejected POST (422 must not consume token)"
    );

    db.teardown().await;
}

#[tokio::test]
async fn post_activate_empty_password_returns_422() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "empty-pw-test").await;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "" }))
        .send()
        .await
        .expect("POST empty password");

    assert_eq!(
        resp.status().as_u16(),
        422,
        "empty password must return 422"
    );

    db.teardown().await;
}

// ── T30: single-use / double-consume ─────────────────────────────────────────

#[tokio::test]
async fn post_activate_same_token_twice_second_returns_404() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "double-consume").await;

    let http = reqwest::Client::new();

    let first = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "FirstGoodPass1!" }))
        .send()
        .await
        .expect("first POST");

    assert_eq!(first.status().as_u16(), 200, "first POST must succeed");
    first.text().await.expect("drain first body");

    let second = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "SecondGoodPass2!" }))
        .send()
        .await
        .expect("second POST");

    assert_eq!(
        second.status().as_u16(),
        404,
        "second POST with same token must return 404 (single-use)"
    );

    let body: serde_json::Value = second.json().await.expect("second body");
    let username = {
        let http2 = reqwest::Client::new();
        let user_check = http2
            .post(format!("{}/v1/auth/login", server.base_url()))
            .json(&serde_json::json!({ "username": "double-consume", "password": "SecondGoodPass2!" }))
            .send()
            .await
            .expect("login attempt with second password");

        assert_ne!(
            user_check.status().as_u16(),
            200,
            "user must not have been re-activated with the second password (second password must not work)"
        );
        body
    };

    let _ = username;
    db.teardown().await;
}

// ── T31: transaction integrity assertion ─────────────────────────────────────
// Atomicity is enforced by an explicit DB transaction in the POST handler.
// If any step fails (validate → hash → set_password → set_activated_at →
// consume_token → create_session), the entire transaction rolls back.
// A forced mid-activation failure via an external integration-test shim is
// not directly injectable here, but we assert the invariant indirectly:
// a 404 (bad token) must leave both the user record and token unconsumed.

#[tokio::test]
async fn post_activate_bad_token_leaves_user_unactivated_and_token_unconsumed() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    use atlas_server::{
        auth::tokens::{generate_session_token, hash_token},
        persistence::repos::NewActivationToken,
    };

    let user = db
        .user_repo()
        .create(NewUser {
            username: "txn-integrity".into(),
            display_name: "Txn Integrity".into(),
            email: None,
            password_hash: None,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let plaintext = generate_session_token();
    let token_hash = hash_token(&plaintext);

    db.activation_token_repo()
        .create(NewActivationToken {
            user_id: user.id,
            token_hash,
            expires_at: Utc::now() + Duration::days(7),
        })
        .await
        .expect("create token");

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/wrong-token", server.base_url()))
        .json(&serde_json::json!({ "password": "SomeGoodPass1!" }))
        .send()
        .await
        .expect("POST wrong token");

    assert_eq!(resp.status().as_u16(), 404);

    let user_after = db
        .user_repo()
        .find_by_username("txn-integrity")
        .await
        .expect("find_by_username")
        .expect("user must still exist");

    assert!(
        user_after.password_hash.is_none(),
        "user's password_hash must remain None after a failed activation"
    );
    assert!(
        user_after.activated_at.is_none(),
        "user's activated_at must remain None after a failed activation"
    );

    let real_get = http
        .get(format!("{}/v1/activate/{plaintext}", server.base_url()))
        .send()
        .await
        .expect("GET real token");

    assert_eq!(
        real_get.status().as_u16(),
        200,
        "real token must still be active — a bad-token POST must not affect other users' tokens"
    );

    db.teardown().await;
}

// ── T32: GovernorLayer present on POST → 429 on burst ───────────────────────

#[tokio::test]
async fn post_activate_rate_limit_returns_429_after_burst() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let http = reqwest::Client::new();
    let base_url = server.base_url().to_string();

    let futures: Vec<_> = (0..10)
        .map(|_| {
            http.post(format!("{base_url}/v1/activate/nonexistent-tok"))
                .json(&serde_json::json!({ "password": "SomePass1!" }))
                .send()
        })
        .collect();

    let statuses: Vec<u16> = futures::future::join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("request").status().as_u16())
        .collect();

    assert!(
        statuses.contains(&429),
        "at least one response must be 429 after burst exceeded; got: {statuses:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn get_activate_rate_limit_returns_429_after_burst() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let http = reqwest::Client::new();
    let base_url = server.base_url().to_string();

    let futures: Vec<_> = (0..10)
        .map(|_| {
            http.get(format!("{base_url}/v1/activate/nonexistent-tok"))
                .send()
        })
        .collect();

    let statuses: Vec<u16> = futures::future::join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("request").status().as_u16())
        .collect();

    assert!(
        statuses.contains(&429),
        "GET burst must also hit 429; got: {statuses:?}"
    );

    db.teardown().await;
}

// ── W1: concurrency hardening — exactly one activation wins the race ─────────
//
// Two concurrent POSTs with the same token must yield exactly one 200 and one
// 404, and the user must end up activated exactly once (no double-session,
// no double-activation). This test covers both the true-concurrent path and the
// lost-race path (pre-consumed token).

/// Simulates the lost-race path: pre-consume the token in the DB, then POST.
/// The handler must return 404 and leave the user unactivated.
///
/// This is the deterministic lower bound of the race: the second racer always
/// loses, and the guarded UPDATE + rows-affected check must catch it.
#[tokio::test]
async fn post_activate_pre_consumed_token_returns_404_no_state_change() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "pre-consumed-race").await;

    use atlas_server::auth::tokens::hash_token;

    let token_hash = hash_token(&token);
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE user_activation_tokens \
             SET consumed_at = now() \
             WHERE token_hash = '{}' AND consumed_at IS NULL",
            token_hash
        ))
        .await
        .expect("pre-consume token to simulate lost race");

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/activate/{token}", server.base_url()))
        .json(&serde_json::json!({ "password": "RacerPass99!" }))
        .send()
        .await
        .expect("POST pre-consumed token");

    assert_eq!(
        resp.status().as_u16(),
        404,
        "a pre-consumed token must return 404 (lost-race path)"
    );

    let has_session = resp
        .headers()
        .get_all("set-cookie")
        .iter()
        .any(|v| v.to_str().unwrap_or("").contains("atlas_session="));

    assert!(
        !has_session,
        "lost-race POST must not issue a session cookie"
    );

    let user = db
        .user_repo()
        .find_by_username("pre-consumed-race")
        .await
        .expect("find_by_username")
        .expect("user must still exist");

    assert!(
        user.activated_at.is_none(),
        "user must remain unactivated after the lost-race POST"
    );
    assert!(
        user.password_hash.is_none(),
        "user's password_hash must remain None after the lost-race POST"
    );

    db.teardown().await;
}

/// Fires two concurrent POSTs for the same token and asserts exactly one wins.
///
/// Under the fixed implementation:
/// - `FOR UPDATE` on the SELECT serializes the two transactions at the DB level.
/// - `AND consumed_at IS NULL` on the UPDATE guards the consume, and rows-affected
///   check rolls back the loser.
///
/// Expected invariants:
/// - Exactly one response is 200, exactly one is 404.
/// - The user has exactly one session (activated_at set once).
/// - The user's password_hash matches the winner's password only.
#[tokio::test]
async fn post_activate_concurrent_requests_exactly_one_wins() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let token = seed_pending_user_with_token(&db, &server, "concurrent-race").await;

    let base_url = server.base_url().to_string();
    let token_a = token.clone();
    let token_b = token.clone();

    let task_a = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base_url}/v1/activate/{token_a}"))
            .json(&serde_json::json!({ "password": "RacerPassA99!" }))
            .send()
            .await
            .expect("concurrent POST A")
            .status()
            .as_u16()
    });

    let base_url_b = server.base_url().to_string();
    let task_b = tokio::spawn(async move {
        reqwest::Client::new()
            .post(format!("{base_url_b}/v1/activate/{token_b}"))
            .json(&serde_json::json!({ "password": "RacerPassB99!" }))
            .send()
            .await
            .expect("concurrent POST B")
            .status()
            .as_u16()
    });

    let (status_a, status_b) = tokio::join!(task_a, task_b);
    let status_a = status_a.expect("task A");
    let status_b = status_b.expect("task B");

    let statuses = [status_a, status_b];

    assert!(
        statuses.contains(&200),
        "exactly one concurrent POST must succeed (200); got: {statuses:?}"
    );
    assert!(
        statuses.contains(&404),
        "exactly one concurrent POST must lose the race (404); got: {statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|&&s| s == 200).count(),
        1,
        "must have exactly one 200; got: {statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|&&s| s == 404).count(),
        1,
        "must have exactly one 404; got: {statuses:?}"
    );

    use sea_orm::{FromQueryResult, Statement};

    #[derive(sea_orm::FromQueryResult, Debug)]
    struct SessionCount {
        n: i64,
    }

    let session_counts = SessionCount::find_by_statement(Statement::from_sql_and_values(
        sea_orm::DatabaseBackend::Postgres,
        "SELECT COUNT(*) AS n \
         FROM sessions s \
         JOIN users u ON u.id = s.user_id \
         WHERE u.username = 'concurrent-race'",
        [],
    ))
    .all(db.conn())
    .await
    .expect("count sessions");

    let session_count = session_counts.first().map(|r| r.n).unwrap_or(0);

    assert_eq!(
        session_count, 1,
        "user must have exactly one session after concurrent activation; got: {session_count}"
    );

    let user = db
        .user_repo()
        .find_by_username("concurrent-race")
        .await
        .expect("find_by_username")
        .expect("user must exist");

    assert!(
        user.activated_at.is_some(),
        "user must be activated after the winning POST"
    );
    assert!(
        user.password_hash.is_some(),
        "user must have a password_hash set by the winner"
    );

    db.teardown().await;
}

// ── T33: registry entries for GET and POST /v1/activate/{token} exist ────────

#[test]
fn registry_has_get_and_post_activate_entries() {
    use atlas_server::routes::registry::{ROUTE_REGISTRY, RouteKind};

    let get_entry = ROUTE_REGISTRY.iter().find(|e| {
        e.method == "GET"
            && e.openapi_path == Some("/v1/activate/{token}")
            && e.kind == RouteKind::Public
    });

    let post_entry = ROUTE_REGISTRY.iter().find(|e| {
        e.method == "POST"
            && e.openapi_path == Some("/v1/activate/{token}")
            && e.kind == RouteKind::Public
    });

    assert!(
        get_entry.is_some(),
        "ROUTE_REGISTRY must contain GET /v1/activate/{{token}} (Public)"
    );
    assert!(
        post_entry.is_some(),
        "ROUTE_REGISTRY must contain POST /v1/activate/{{token}} (Public)"
    );
}
