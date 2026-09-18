#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::{
    ApiKeyScope, CreateUserApiKeyRequest, InitialGrantRequest, LoginRequest, MeResponse,
};
use atlas_client::AtlasClient;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_server::auth::tokens::hash_token;
use atlas_server::persistence::repos::{ApiKeyRepo, NewApiKey, UserRepo};

#[tokio::test]
async fn login_returns_body_token_and_set_cookie() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-login-user").await;

    assert!(
        client.token().is_some(),
        "client must store the session token after login"
    );

    db.teardown().await;
}

#[tokio::test]
async fn login_invalid_credentials_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "nobody".into(),
            password: "wrong".into(),
        })
        .await;

    assert!(result.is_err(), "wrong credentials must fail");

    db.teardown().await;
}

#[tokio::test]
async fn bearer_token_authenticates_me_endpoint() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "auth-me-user").await;

    let me: MeResponse = client
        .custos()
        .me()
        .await
        .expect("GET /api/auth/me must succeed");

    assert_eq!(me.username, user.username);

    db.teardown().await;
}

#[tokio::test]
async fn unauthenticated_me_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = AtlasClient::new(server.base_url()).custos().me().await;

    assert!(result.is_err(), "unauthenticated /me must fail with 401");

    db.teardown().await;
}

#[tokio::test]
async fn logout_revokes_session() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-logout-user").await;

    client.custos().logout().await.expect("logout must succeed");

    let result = client.custos().me().await;
    assert!(result.is_err(), "after logout the token must be invalid");

    db.teardown().await;
}

#[tokio::test]
async fn expired_session_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-expiry-user").await;

    support::expire_all_sessions(&db).await;

    let result = client.custos().me().await;
    assert!(result.is_err(), "expired session must be rejected with 401");

    db.teardown().await;
}

#[tokio::test]
async fn nonexistent_user_login_returns_401() {
    // Behavioral test for timing-oracle fix: both "user not found" and "wrong password"
    // paths must return 401 with the same shape. Timing itself is not unit-asserted.
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: "does-not-exist-at-all".into(),
            password: "anypassword".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "nonexistent user login must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn wrong_password_returns_401() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-wrongpw-user").await;

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: user.username.clone(),
            password: "definitelywrong".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "wrong password must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn disabled_user_with_correct_password_returns_401() {
    // Behavioral test for the disabled-account timing fix: a disabled user must
    // still receive 401 even when the correct password is supplied.
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-disabled-user").await;

    db.user_repo().disable(user.id).await.expect("disable user");

    let result = atlas_client::AtlasClient::new(server.base_url())
        .login(LoginRequest {
            username: user.username.clone(),
            password: "TestPassword1!".into(),
        })
        .await;

    assert!(
        matches!(result, Err(atlas_client::ClientError::Api(ref p)) if p.status == 401),
        "disabled user with correct password must return 401, got: {result:?}"
    );

    db.teardown().await;
}

#[tokio::test]
async fn me_for_human_user_has_no_agent_identity() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-me-human").await;

    let me: MeResponse = client
        .custos()
        .me()
        .await
        .expect("GET /api/auth/me must succeed");

    assert_eq!(me.principal_type, "user");
    assert!(
        me.agent.is_none(),
        "a human principal must not carry an agent self-identity"
    );

    db.teardown().await;
}

#[tokio::test]
async fn me_for_api_key_returns_agent_identity_with_canonical_scopes() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (owner, ws, _user) =
        support::login_user_with_workspace(&server, &db, "auth-me-agent").await;

    // Deliberately unsorted, duplicated scope set: the read path must return it
    // deduplicated and in canonical family:action order.
    let created = owner
        .custos()
        .create_user_api_key(CreateUserApiKeyRequest {
            key_kind: None,
            name: "self-identity-agent".to_string(),
            r#type: None,
            expires_at: None,
            initial_grant: Some(InitialGrantRequest {
                workspace: ws.slug.clone(),
                role: "editor".to_string(),
            }),
            scopes: Some(vec![
                ApiKeyScope::TasksUpdate,
                ApiKeyScope::TasksRead,
                ApiKeyScope::TasksRead,
                ApiKeyScope::DocsRead,
            ]),
        })
        .await
        .expect("create agent key");

    let agent_client = AtlasClient::new(server.base_url()).with_token(created.secret);

    let me: MeResponse = agent_client
        .custos()
        .me()
        .await
        .expect("GET /api/auth/me as an agent must succeed");

    assert_eq!(me.principal_type, "api_key");

    let agent = me
        .agent
        .expect("an API-key principal must carry an agent self-identity");

    assert_eq!(agent.id, created.id);
    assert_eq!(agent.name, "self-identity-agent");
    assert_eq!(
        agent.scopes,
        vec![
            ApiKeyScope::TasksRead,
            ApiKeyScope::TasksUpdate,
            ApiKeyScope::DocsRead,
        ],
        "scopes must be deduplicated and canonically ordered, matching the api-keys read path"
    );

    db.teardown().await;
}

// ---------------------------------------------------------------------------
// API key token prefixes (E4-S2C): the presented prefix must agree with the
// linked principal's kind; a bare legacy `atlas_` token is untyped and stays
// accepted for every principal kind.
// ---------------------------------------------------------------------------

/// Inserts a key for `user_id` whose plaintext bearer token is exactly
/// `raw_token` (any prefix), linked to a fresh agent principal.
async fn insert_agent_key_with_token(
    db: &support::TestDb,
    user_id: atlas_core::principal::UserId,
    name: &str,
    raw_token: &str,
) {
    let repo = db.api_key_repo();
    repo.create_for_user(
        user_id,
        NewApiKey {
            name: name.to_string(),
            token_hash: hash_token(raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes: Vec::new(),
        },
    )
    .await
    .expect("insert agent key with crafted token");
}

/// Inserts a personal key for `user_id` whose plaintext bearer token is
/// exactly `raw_token` (any prefix), linked to the owner's user principal.
async fn insert_personal_key_with_token(
    db: &support::TestDb,
    user_id: atlas_core::principal::UserId,
    name: &str,
    raw_token: &str,
) {
    use atlas_custos::entities::identity::ApiKeyKind;
    use atlas_custos_postgres::repos::identity::PgApiKeyRepo;

    PgApiKeyRepo::create_for_user_in_with_kind(
        db.conn(),
        user_id,
        ApiKeyKind::Personal,
        NewApiKey {
            name: name.to_string(),
            token_hash: hash_token(raw_token),
            type_: ApiKeyType::Agent,
            expires_at: None,
            scopes: Vec::new(),
        },
    )
    .await
    .expect("insert personal key with crafted token");
}

/// Returns the HTTP status of `GET /api/v2/custos/auth/me` presented with the
/// given raw bearer token.
async fn me_status_with_bearer(server: &support::TestServer, raw_token: &str) -> u16 {
    let http = reqwest::Client::new();
    let resp = http
        .get(support::path::api_url(
            server.base_url(),
            "custos",
            "/auth/me",
        ))
        .header("Authorization", format!("Bearer {raw_token}"))
        .send()
        .await
        .expect("me request");
    resp.status().as_u16()
}

#[tokio::test]
async fn agent_key_with_a_disagreeing_personal_prefix_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, user) = support::login_user(&server, &db, "auth-prefix-agent").await;
    let user_id = user.id;

    let raw_token = "atlas_pk_notARealPersonalBody0123456789abcdefghij";
    insert_agent_key_with_token(&db, user_id, "mismatched-key", raw_token).await;
    drop(client);

    let status = me_status_with_bearer(&server, raw_token).await;
    assert_eq!(
        status, 401,
        "a personal prefix on an agent principal must be rejected"
    );

    db.teardown().await;
}

#[tokio::test]
async fn personal_key_with_a_disagreeing_agent_prefix_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-prefix-personal").await;
    let user_id = user.id;

    // A stored personal key (user principal) whose plaintext token carries the
    // agent prefix: the hash lookup succeeds, so the rejection must come from
    // the prefix/principal-kind agreement check, not from a lookup miss.
    let raw_token = "atlas_ak_notARealAgentBody0123456789abcdefghijk";
    insert_personal_key_with_token(&db, user_id, "mismatched-personal", raw_token).await;

    let status = me_status_with_bearer(&server, raw_token).await;
    assert_eq!(
        status, 401,
        "an agent prefix on a personal (user principal) key must be rejected"
    );

    db.teardown().await;
}

#[tokio::test]
async fn bare_legacy_atlas_token_is_still_accepted() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (_client, user) = support::login_user(&server, &db, "auth-bare-legacy").await;
    let user_id = user.id;

    // V1 keys have no kind segment: a bare `atlas_` prefix declares no kind,
    // so it must authenticate regardless of the linked principal's kind.
    let raw_token = "atlas_legacyBody0123456789abcdefghij0123456789abc";
    insert_agent_key_with_token(&db, user_id, "legacy-key", raw_token).await;

    let status = me_status_with_bearer(&server, raw_token).await;
    assert_eq!(
        status, 200,
        "a bare legacy atlas_ token must keep authenticating"
    );

    db.teardown().await;
}

#[tokio::test]
async fn personal_api_key_authenticates_with_its_atlas_pk_token() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "auth-personal-me").await;

    let created = client
        .custos()
        .create_user_api_key(CreateUserApiKeyRequest {
            name: "personal-me".to_string(),
            r#type: None,
            key_kind: Some("personal".to_string()),
            expires_at: None,
            initial_grant: None,
            scopes: None,
        })
        .await
        .expect("create personal key");

    let personal = AtlasClient::new(server.base_url()).with_token(created.secret);
    let me: MeResponse = personal
        .custos()
        .me()
        .await
        .expect("a personal key must authenticate with its atlas_pk_ token");
    assert_eq!(me.principal_type, "api_key");

    db.teardown().await;
}
