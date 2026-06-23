#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_api::dtos::LoginRequest;
use atlas_server::persistence::repos::UserRepo;

/// Extracts the `atlas_session` cookie value from a `Set-Cookie` response header.
fn extract_session_cookie(response: &reqwest::Response) -> Option<String> {
    for value in response.headers().get_all("set-cookie") {
        if let Ok(s) = value.to_str() {
            for part in s.split(';') {
                let part = part.trim();
                if let Some(val) = part.strip_prefix("atlas_session=") {
                    return Some(val.to_owned());
                }
            }
        }
    }
    None
}

/// Performs a raw login and returns (token, session_cookie_value).
async fn raw_login(base_url: &str, username: &str, password: &str) -> (String, Option<String>) {
    let client = reqwest::Client::builder()
        .build()
        .expect("build reqwest client");

    let resp = client
        .post(format!("{base_url}/v1/auth/login"))
        .json(&LoginRequest {
            username: username.to_string(),
            password: password.to_string(),
        })
        .send()
        .await
        .expect("login request");

    let cookie = extract_session_cookie(&resp);
    let body: serde_json::Value = resp.json().await.expect("login body");
    let token = body["token"].as_str().unwrap_or("").to_string();

    (token, cookie)
}

#[tokio::test]
async fn csrf_cookie_mutation_without_header_is_rejected() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let password = "TestPassword1!";
    let hash = atlas_server::auth::password::hash(password.to_string())
        .await
        .expect("hash");
    db.user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: "csrf-test-user".to_string(),
            display_name: "csrf-test-user".to_string(),
            email: None,
            password_hash: hash,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let (_token, maybe_cookie) = raw_login(server.base_url(), "csrf-test-user", password).await;
    let session_cookie = maybe_cookie.expect("login must set atlas_session cookie");

    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{}/v1/auth/logout", server.base_url()))
        .header("Cookie", format!("atlas_session={session_cookie}"))
        .send()
        .await
        .expect("logout request");

    assert_eq!(
        resp.status().as_u16(),
        403,
        "cookie-authenticated mutation without X-Atlas-CSRF must be rejected with 403"
    );

    db.teardown().await;
}

#[tokio::test]
async fn csrf_cookie_mutation_with_header_succeeds() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let password = "TestPassword1!";
    let hash = atlas_server::auth::password::hash(password.to_string())
        .await
        .expect("hash");
    db.user_repo()
        .create(atlas_server::persistence::repos::NewUser {
            username: "csrf-ok-user".to_string(),
            display_name: "csrf-ok-user".to_string(),
            email: None,
            password_hash: hash,
            is_root: false,
            is_system_admin: false,
        })
        .await
        .expect("create user");

    let (_token, maybe_cookie) = raw_login(server.base_url(), "csrf-ok-user", password).await;
    let session_cookie = maybe_cookie.expect("login must set atlas_session cookie");

    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{}/v1/auth/logout", server.base_url()))
        .header("Cookie", format!("atlas_session={session_cookie}"))
        .header("X-Atlas-CSRF", "1")
        .send()
        .await
        .expect("logout request");

    assert_eq!(
        resp.status().as_u16(),
        204,
        "cookie-authenticated mutation WITH X-Atlas-CSRF must succeed"
    );

    db.teardown().await;
}

#[tokio::test]
async fn csrf_bearer_mutation_is_exempt() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let (client, _user) = support::login_user(&server, &db, "csrf-bearer-user").await;

    let token = client.token().expect("client must have token");
    let http = reqwest::Client::new();

    let resp = http
        .post(format!("{}/v1/auth/logout", server.base_url()))
        .header("Authorization", format!("Bearer {token}"))
        .send()
        .await
        .expect("logout request");

    assert_eq!(
        resp.status().as_u16(),
        204,
        "bearer-authenticated request must not require CSRF header"
    );

    db.teardown().await;
}

#[tokio::test]
async fn login_rate_limit_returns_429_after_quota_exceeded() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;

    let base_url = server.base_url().to_string();
    let http = reqwest::Client::new();

    // Send all requests concurrently so they all hit the rate-limiter gate
    // before any handler has time to complete. With burst_size(5) the first 5
    // pass and the rest are rejected with 429 without waiting for argon2.
    let futures: Vec<_> = (0..10)
        .map(|_| {
            http.post(format!("{base_url}/v1/auth/login"))
                .json(&LoginRequest {
                    username: "nonexistent".to_string(),
                    password: "wrong".to_string(),
                })
                .send()
        })
        .collect();

    let statuses: Vec<u16> = futures::future::join_all(futures)
        .await
        .into_iter()
        .map(|r| r.expect("login attempt").status().as_u16())
        .collect();

    let saw_429 = statuses.contains(&429);
    assert!(
        saw_429,
        "at least one response must be 429 after burst exhausted; got: {statuses:?}"
    );

    db.teardown().await;
}
