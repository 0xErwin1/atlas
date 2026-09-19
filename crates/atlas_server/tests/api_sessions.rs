//! Self-service session management for the authenticated user
//! (`v2-e4-s3a-sessions`): list your own sessions, revoke one, revoke all
//! except the current one. No migration — `custos.sessions` already carries
//! every column these routes need.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_client::AtlasClient;
use atlas_server::persistence::repos::{NewSession, NewUser, SessionRepo, UserRepo};
use chrono::Utc;
use sea_orm::ConnectionTrait;

/// Returns `(status, body)` of `GET /api/v2/custos/sessions` presented with
/// the given raw bearer token.
async fn list_sessions_with_bearer(
    server: &support::TestServer,
    raw_token: &str,
) -> (u16, serde_json::Value) {
    let http = reqwest::Client::new();
    let resp = http
        .get(support::path::api_url(
            server.base_url(),
            "custos",
            "/sessions",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("list sessions request");
    let status = resp.status().as_u16();
    let body = resp.json().await.unwrap_or(serde_json::Value::Null);
    (status, body)
}

/// Creates an activated user with a throwaway password hash (never logged in
/// through; the session rows are seeded directly through the repo).
async fn seed_user(db: &support::TestDb, username: &str) -> atlas_server::persistence::repos::User {
    let repo = db.user_repo();
    let user = repo
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$test$hash".into()),
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("seed user");
    support::activate_user_in_db(db, user.id.0).await;
    user
}

/// Creates a session for `user_id` and then stamps a fixed `created_at`, so
/// the newest-first ordering assertion never depends on insert-speed timing.
async fn seed_session(
    db: &support::TestDb,
    user_id: uuid::Uuid,
    token_hash: &str,
    created_at: &str,
) -> uuid::Uuid {
    let repo = db.session_repo();
    let session = repo
        .create(NewSession {
            user_id: atlas_core::principal::UserId(user_id),
            token_hash: token_hash.to_string(),
            expires_at: Utc::now() + chrono::Duration::days(30),
        })
        .await
        .expect("seed session");

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE custos.sessions SET created_at = '{created_at}' WHERE id = '{}'",
            session.id.0
        ))
        .await
        .expect("stamp created_at");

    session.id.0
}

#[tokio::test]
async fn list_for_user_returns_only_that_users_sessions_newest_first() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let user_a = seed_user(&db, "sessions-list-a").await;
    let user_b = seed_user(&db, "sessions-list-b").await;

    let a_older = seed_session(&db, user_a.id.0, "hash-a-older", "2026-01-01 00:00:00+00").await;
    let a_newer = seed_session(&db, user_a.id.0, "hash-a-newer", "2026-02-01 00:00:00+00").await;
    let _b_session = seed_session(&db, user_b.id.0, "hash-b", "2026-01-15 00:00:00+00").await;

    let sessions = db
        .session_repo()
        .list_for_user(user_a.id)
        .await
        .expect("list_for_user must succeed");

    let listed_ids: Vec<uuid::Uuid> = sessions.iter().map(|s| s.id.0).collect();

    assert_eq!(
        listed_ids,
        vec![a_newer, a_older],
        "list_for_user must return only the caller's own sessions, newest first"
    );

    db.teardown().await;
}

#[tokio::test]
async fn list_for_user_for_a_user_without_sessions_is_empty() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let user = seed_user(&db, "sessions-list-empty").await;

    let sessions = db
        .session_repo()
        .list_for_user(user.id)
        .await
        .expect("list_for_user must succeed");

    assert!(
        sessions.is_empty(),
        "a user with no sessions must list none, got {}",
        sessions.len()
    );

    db.teardown().await;
}

#[tokio::test]
async fn sessions_list_exposes_no_token_hash_and_reports_activity_flags() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "sessions-list-dto").await;

    let revoked = seed_session(&db, _user.id.0, "hash-revoked", "2026-01-01 00:00:00+00").await;
    let expired = seed_session(&db, _user.id.0, "hash-expired", "2026-01-02 00:00:00+00").await;
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE custos.sessions SET revoked_at = now() WHERE id = '{revoked}'"
        ))
        .await
        .expect("revoke seeded session");
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE custos.sessions SET expires_at = now() - interval '1 hour' WHERE id = '{expired}'"
        ))
        .await
        .expect("expire seeded session");

    let raw_token = client.token().expect("logged-in client holds a token");
    let (status, body) = list_sessions_with_bearer(&server, raw_token).await;

    assert_eq!(status, 200, "listing own sessions must succeed: {body}");
    let items = body.as_array().expect("list response is a JSON array");
    assert_eq!(items.len(), 3, "the user's three sessions must be listed");

    let expected_keys = [
        "id",
        "created_at",
        "last_used_at",
        "expires_at",
        "revoked_at",
        "active",
    ];
    for item in items {
        let obj = item.as_object().expect("each item is an object");
        let mut keys: Vec<String> = obj.keys().map(|k| k.to_string()).collect();
        keys.sort();
        let mut expected: Vec<String> = expected_keys.iter().map(|k| k.to_string()).collect();
        expected.sort();
        assert_eq!(
            keys, expected,
            "a session projection must never carry the token hash or any other field: {obj:?}"
        );
    }

    let find = |id: &uuid::Uuid| {
        items
            .iter()
            .find(|item| item["id"] == *id.to_string())
            .unwrap_or_else(|| panic!("session {id} must be in the list"))
            .clone()
    };

    assert_eq!(
        find(&revoked)["active"],
        false,
        "a revoked session is inactive"
    );
    assert_eq!(
        find(&expired)["active"],
        false,
        "an expired session is inactive"
    );
    assert_eq!(
        items.iter().filter(|i| i["active"] == true).count(),
        1,
        "only the caller's current (login) session is active"
    );

    db.teardown().await;
}

/// Returns the HTTP status of `DELETE /api/v2/custos/sessions` (revoke all
/// except the current session) presented with the given raw bearer token.
async fn revoke_other_sessions_with_bearer(server: &support::TestServer, raw_token: &str) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .delete(support::path::api_url(
            server.base_url(),
            "custos",
            "/sessions",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("revoke-other-sessions request");
    resp.status().as_u16()
}

/// Returns the HTTP status of `DELETE /api/v2/custos/sessions/{id}` presented
/// with the given raw bearer token.
async fn revoke_session_with_bearer(
    server: &support::TestServer,
    raw_token: &str,
    session_id: uuid::Uuid,
) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .delete(support::path::api_url(
            server.base_url(),
            "custos",
            &format!("/sessions/{session_id}"),
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("revoke-session request");
    resp.status().as_u16()
}

/// Returns the HTTP status of `GET /api/v2/custos/sessions` presented with
/// the given raw bearer token.
async fn list_sessions_status_with_bearer(server: &support::TestServer, raw_token: &str) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .get(support::path::api_url(
            server.base_url(),
            "custos",
            "/sessions",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("list sessions request");
    resp.status().as_u16()
}

#[tokio::test]
async fn revoking_one_of_your_own_sessions_marks_it_inactive() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "sessions-revoke-own").await;
    let raw_token = client.token().expect("logged-in client holds a token");

    let other = seed_session(&db, user.id.0, "hash-other", "2026-01-01 00:00:00+00").await;

    let status = revoke_session_with_bearer(&server, raw_token, other).await;
    assert_eq!(status, 204, "revoking your own session must return 204");

    let (_, body) = list_sessions_with_bearer(&server, raw_token).await;
    let items = body.as_array().expect("list response is a JSON array");
    assert_eq!(items.len(), 2);
    let revoked_item = items
        .iter()
        .find(|i| i["id"] == other.to_string())
        .expect("the revoked session is still listed (with active = false)");
    assert_eq!(revoked_item["active"], false);

    db.teardown().await;
}

#[tokio::test]
async fn revoking_an_already_revoked_session_is_idempotent() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "sessions-revoke-twice").await;
    let raw_token = client.token().expect("logged-in client holds a token");

    let target = seed_session(&db, user.id.0, "hash-target", "2026-01-01 00:00:00+00").await;

    assert_eq!(
        revoke_session_with_bearer(&server, raw_token, target).await,
        204
    );
    assert_eq!(
        revoke_session_with_bearer(&server, raw_token, target).await,
        204,
        "re-revoking a revoked session must stay a 204 no-op"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoking_a_foreign_session_returns_404_and_leaves_it_alive() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user_a) = support::login_user(&server, &db, "sessions-foreign-a").await;
    let raw_token = client.token().expect("logged-in client holds a token");

    let user_b = seed_user(&db, "sessions-foreign-b").await;
    let b_session = seed_session(&db, user_b.id.0, "hash-b", "2026-01-01 00:00:00+00").await;

    let status = revoke_session_with_bearer(&server, raw_token, b_session).await;
    assert_eq!(
        status, 404,
        "a session id the caller cannot see must not exist for them (non-disclosure)"
    );

    let b_sessions = db
        .session_repo()
        .list_for_user(user_b.id)
        .await
        .expect("list user B's sessions");
    let target = b_sessions
        .iter()
        .find(|s| s.id.0 == b_session)
        .expect("user B's session is still listed");
    assert!(
        target.revoked_at.is_none(),
        "the foreign session must survive the rejected revoke"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoking_the_current_session_by_id_behaves_like_logout() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "sessions-revoke-current").await;
    let raw_token = client.token().expect("logged-in client holds a token");

    // The current session is the caller's only active one.
    let (_, body) = list_sessions_with_bearer(&server, raw_token).await;
    let items = body.as_array().expect("list response is a JSON array");
    let current = items
        .iter()
        .find(|i| i["active"] == true)
        .expect("the login session is active")["id"]
        .as_str()
        .expect("session id is a string")
        .to_string();

    let status = revoke_session_with_bearer(&server, raw_token, current.parse().unwrap()).await;
    assert_eq!(status, 204, "revoking the current session by id is allowed");

    let result = client.custos().me().await;
    assert!(
        result.is_err(),
        "after revoking the current session the token must be dead, like logout"
    );

    db.teardown().await;
}

#[tokio::test]
async fn revoke_other_sessions_keeps_the_current_session_and_kills_the_rest() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "sessions-revoke-others").await;
    let raw_token = client.token().expect("logged-in client holds a token");

    let _a = seed_session(&db, user.id.0, "hash-a", "2026-01-01 00:00:00+00").await;
    let _b = seed_session(&db, user.id.0, "hash-b", "2026-01-02 00:00:00+00").await;

    let status = revoke_other_sessions_with_bearer(&server, raw_token).await;
    assert_eq!(status, 204);

    let (_, body) = list_sessions_with_bearer(&server, raw_token).await;
    let items = body.as_array().expect("list response is a JSON array");
    assert_eq!(items.len(), 3, "the listing still shows every own session");
    assert_eq!(
        items.iter().filter(|i| i["active"] == true).count(),
        1,
        "exactly the caller's current session survives"
    );

    // The token still works afterwards — the current session is alive.
    assert_eq!(
        list_sessions_status_with_bearer(&server, raw_token).await,
        200
    );

    db.teardown().await;
}

#[tokio::test]
async fn api_key_principal_cannot_list_or_revoke_sessions() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "sessions-api-key").await;

    let created = client
        .custos()
        .create_personal_api_key(atlas_api::dtos::CreatePersonalApiKeyRequest {
            name: "sessions-agent".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: None,
            scopes: None,
        })
        .await
        .expect("create agent key");
    let agent = AtlasClient::new(server.base_url()).with_token(created.secret);
    let raw_token = agent
        .token()
        .expect("agent client holds a token")
        .to_string();

    // An own session of the owning user, for the per-id revoke attempt.
    let target = seed_session(&db, user.id.0, "hash-target", "2026-01-01 00:00:00+00").await;

    let (list_status, body) = list_sessions_with_bearer(&server, &raw_token).await;
    assert_eq!(list_status, 403, "API keys cannot list sessions: {body}");
    let problem_type = body["type"].as_str().unwrap_or_default().to_string();
    assert_eq!(problem_type, "urn:atlas:error:forbidden");
    let detail = body["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("API keys") || detail.contains("human"),
        "the 403 must carry an actionable hint: {detail}"
    );

    assert_eq!(
        revoke_session_with_bearer(&server, &raw_token, target).await,
        403,
        "API keys cannot revoke a session"
    );
    assert_eq!(
        revoke_other_sessions_with_bearer(&server, &raw_token).await,
        403,
        "API keys cannot revoke-all-except-current"
    );

    let sessions = db
        .session_repo()
        .list_for_user(user.id)
        .await
        .expect("list the owner's sessions");
    let target = sessions
        .iter()
        .find(|s| s.id.0 == target)
        .expect("the target session is still listed");
    assert!(
        target.revoked_at.is_none(),
        "the target session must be untouched"
    );

    db.teardown().await;
}
