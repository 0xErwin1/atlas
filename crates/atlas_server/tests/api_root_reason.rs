//! `v2-e4-s3c-root-reason`: root becomes break-glass that must justify itself.
//!
//! Coverage:
//! - root login requires a non-empty `reason`; a non-root login does not;
//! - a successful root login stores the reason on the session and appends a
//!   `root.login` audit row carrying it;
//! - a single middleware site appends one `root.action` audit row per
//!   state-changing (POST/PUT/PATCH/DELETE) request under a root session,
//!   carrying the method, path, and the session's reason; reads are not
//!   audited and non-root sessions are not root-audited;
//! - a root session without a stored reason fails closed on state-changing
//!   requests instead of running unaudited;
//! - root callers cannot create personal API keys (a key has no session and
//!   therefore no reason).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{CreatePersonalApiKeyRequest, LoginRequest};
use atlas_client::{AtlasClient, ClientError};
use atlas_server::auth::password;
use atlas_server::persistence::repos::{NewUser, User, UserRepo};
use sea_orm::{ConnectionTrait, DatabaseBackend, FromQueryResult, Statement};

const PASSWORD: &str = "TestPassword1!";

/// Seeds an activated user (optionally root) with a real password hash.
async fn seed_user(db: &support::TestDb, username: &str, is_root: bool) -> User {
    let password_hash = password::hash(PASSWORD.to_string())
        .await
        .expect("hash password");

    let user = db
        .user_repo()
        .create(NewUser {
            username: username.to_string(),
            display_name: username.to_string(),
            email: None,
            password_hash: Some(password_hash),
            is_root,
            is_system_admin: false,
        })
        .await
        .expect("seed user");

    db.conn()
        .execute_unprepared(&format!(
            "UPDATE custos.users SET activated_at = now() WHERE id = '{}'",
            user.id.0
        ))
        .await
        .expect("activate user");

    user
}

async fn login(
    server: &support::TestServer,
    username: &str,
    reason: Option<&str>,
) -> Result<AtlasClient, ClientError> {
    let mut client = AtlasClient::new(server.base_url().to_string());
    client
        .login(LoginRequest {
            username: username.to_string(),
            password: PASSWORD.to_string(),
            reason: reason.map(str::to_string),
        })
        .await?;
    Ok(client)
}

/// The HTTP status of an API error response, or `None` for any other outcome.
fn api_status<T>(result: &Result<T, ClientError>) -> Option<u16> {
    match result {
        Err(ClientError::Api(p)) => Some(p.status),
        _ => None,
    }
}

#[derive(Debug, FromQueryResult)]
struct SessionRow {
    root_reason: Option<String>,
    revoked_at: Option<chrono::DateTime<chrono::Utc>>,
}

async fn latest_session_for(db: &support::TestDb, user_id: uuid::Uuid) -> SessionRow {
    SessionRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT root_reason, revoked_at FROM custos.sessions \
             WHERE user_id = '{}' ORDER BY created_at DESC LIMIT 1",
            user_id
        ),
    ))
    .one(db.conn())
    .await
    .expect("query session")
    .expect("session row must exist")
}

#[derive(Debug, FromQueryResult)]
struct AuditRow {
    action: String,
    target_type: String,
    metadata: serde_json::Value,
}

/// Every `root.*` audit row attributed to the user, oldest first.
async fn root_audit_rows(db: &support::TestDb, user_id: uuid::Uuid) -> Vec<AuditRow> {
    AuditRow::find_by_statement(Statement::from_string(
        DatabaseBackend::Postgres,
        format!(
            "SELECT action, target_type, metadata FROM custos.security_audit_log \
             WHERE actor_user_id = '{}' AND action LIKE 'root.%' \
             ORDER BY created_at ASC, id ASC",
            user_id
        ),
    ))
    .all(db.conn())
    .await
    .expect("query root audit rows")
}

// ---------------------------------------------------------------------------
// W2: the login boundary.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn root_login_without_a_reason_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-no-reason", true).await;

    let result = login(&server, &user.username, None).await;

    assert_eq!(
        api_status(&result),
        Some(422),
        "root login without a reason must be rejected with an actionable 422, got error: {:?}",
        result.as_ref().err()
    );

    db.teardown().await;
}

#[tokio::test]
async fn root_login_with_a_blank_reason_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-blank-reason", true).await;

    let result = login(&server, &user.username, Some("   ")).await;

    assert_eq!(
        api_status(&result),
        Some(422),
        "a whitespace-only reason must be treated as missing, got error: {:?}",
        result.as_ref().err()
    );

    db.teardown().await;
}

#[tokio::test]
async fn root_login_with_a_reason_succeeds_stores_it_and_writes_the_login_audit_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-with-reason", true).await;

    let client = login(&server, &user.username, Some("prod incident 4711"))
        .await
        .expect("root login with a reason must succeed");

    let session = latest_session_for(&db, user.id.0).await;
    assert_eq!(
        session.root_reason.as_deref(),
        Some("prod incident 4711"),
        "the session must carry the stated reason"
    );
    assert!(session.revoked_at.is_none(), "the session must be active");
    assert!(
        client.token().is_some(),
        "the client must hold the session token"
    );

    let rows = root_audit_rows(&db, user.id.0).await;
    assert_eq!(rows.len(), 1, "exactly one root audit row after login");
    assert_eq!(rows[0].action, "root.login");
    assert_eq!(rows[0].target_type, "session");
    assert_eq!(
        rows[0].metadata.get("reason").and_then(|v| v.as_str()),
        Some("prod incident 4711"),
        "the login audit row must carry the reason"
    );

    db.teardown().await;
}

#[tokio::test]
async fn non_root_login_still_needs_no_reason() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "plain-no-reason", false).await;

    login(&server, &user.username, None)
        .await
        .expect("a non-root login must not require a reason");

    let session = latest_session_for(&db, user.id.0).await;
    assert_eq!(
        session.root_reason, None,
        "a non-root session carries no reason"
    );
    assert!(
        root_audit_rows(&db, user.id.0).await.is_empty(),
        "a non-root login writes no root audit row"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// W3: the per-action audit, one middleware site.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn root_state_changing_request_writes_one_action_row_carrying_method_path_reason() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-action", true).await;
    let client = login(&server, &user.username, Some("incident response"))
        .await
        .expect("root login");

    client.custos().logout().await.expect("logout must succeed");

    let rows = root_audit_rows(&db, user.id.0).await;
    assert_eq!(
        rows.len(),
        2,
        "one root.login row and one root.action row expected, got: {rows:?}"
    );
    assert_eq!(rows[1].action, "root.action");
    assert_eq!(
        rows[1].metadata.get("method").and_then(|v| v.as_str()),
        Some("POST"),
        "the action row must carry the HTTP method"
    );
    assert_eq!(
        rows[1].metadata.get("path").and_then(|v| v.as_str()),
        Some("/api/v2/custos/auth/logout"),
        "the action row must carry the request path"
    );
    assert_eq!(
        rows[1].metadata.get("reason").and_then(|v| v.as_str()),
        Some("incident response"),
        "the action row must carry the session's reason"
    );

    let session = latest_session_for(&db, user.id.0).await;
    assert!(
        session.revoked_at.is_some(),
        "the audited logout must still have taken effect"
    );

    db.teardown().await;
}

#[tokio::test]
async fn root_read_request_writes_no_action_row() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-read", true).await;
    let client = login(&server, &user.username, Some("inspection"))
        .await
        .expect("root login");

    client.custos().me().await.expect("read must succeed");

    let rows = root_audit_rows(&db, user.id.0).await;
    assert!(
        rows.iter().all(|r| r.action == "root.login"),
        "reads must not produce root.action rows, got: {rows:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn non_root_state_changing_request_is_not_root_audited() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "plain-action", false).await;
    let client = login(&server, &user.username, None)
        .await
        .expect("non-root login");

    client.custos().logout().await.expect("logout must succeed");

    assert!(
        root_audit_rows(&db, user.id.0).await.is_empty(),
        "a non-root session must not be root-audited"
    );

    db.teardown().await;
}

#[tokio::test]
async fn root_session_without_a_stored_reason_fails_closed_on_state_changes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-null-reason", true).await;
    let client = login(&server, &user.username, Some("will be wiped"))
        .await
        .expect("root login");

    // Simulate the only way a root session can lack a reason after the
    // migration + login gate: out-of-band tampering. The middleware must fail
    // closed instead of running the handler unaudited.
    db.conn()
        .execute_unprepared(&format!(
            "UPDATE custos.sessions SET root_reason = NULL WHERE user_id = '{}'",
            user.id.0
        ))
        .await
        .expect("wipe session reason");

    let result = client.custos().logout().await;
    assert!(
        result.is_err(),
        "a root session without a stored reason must be rejected on state changes"
    );

    let session = latest_session_for(&db, user.id.0).await;
    assert!(
        session.revoked_at.is_none(),
        "fail closed means the handler never ran: the session must not be revoked"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// W4: root may not hold personal API keys.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn root_caller_cannot_create_a_personal_api_key() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let user = seed_user(&db, "root-personal-key", true).await;
    let client = login(&server, &user.username, Some("credential attempt"))
        .await
        .expect("root login");

    let result = client
        .custos()
        .create_personal_api_key(CreatePersonalApiKeyRequest {
            name: "root standing credential".to_string(),
            r#type: None,
            expires_at: None,
            scopes: None,
            initial_grant: None,
        })
        .await;

    assert_eq!(
        api_status(&result),
        Some(403),
        "root must be rejected from personal key creation with an actionable 403, got error: {:?}",
        result.as_ref().err()
    );

    db.teardown().await;
}
