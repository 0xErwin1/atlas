#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use atlas_server::state::AppState;

/// With the per-principal limiter enabled at burst(2), a rapid burst of
/// authenticated requests from the same principal must yield at least one 429,
/// and that 429 must carry a `Retry-After` header so clients can back off.
#[tokio::test]
async fn authenticated_burst_is_rate_limited_with_retry_after() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_rate_limit(1, 2);
    let server = support::TestServer::spawn_with_state(state).await;

    let (client, _user) = support::login_user(&server, &db, "rate-limit-user").await;
    let token = client.token().expect("client must have token").to_string();

    let base_url = server.base_url().to_string();
    let http = reqwest::Client::new();

    // Fire concurrently so all requests reach the limiter gate before any refills.
    let futures: Vec<_> = (0..8)
        .map(|_| {
            http.get(format!("{base_url}/v1/auth/me"))
                .header("Authorization", format!("Bearer {token}"))
                .send()
        })
        .collect();

    let responses = futures::future::join_all(futures).await;

    let mut saw_429 = false;
    let mut retry_after_present = false;
    let mut statuses = Vec::new();

    for result in responses {
        let response = result.expect("request");
        statuses.push(response.status().as_u16());
        if response.status().as_u16() == 429 {
            saw_429 = true;
            if response
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .is_some()
            {
                retry_after_present = true;
            }
        }
    }

    assert!(
        saw_429,
        "burst of 8 requests past burst(2) must produce at least one 429; got: {statuses:?}"
    );
    assert!(
        retry_after_present,
        "the 429 response must carry a Retry-After header"
    );

    db.teardown().await;
}

/// Regression guard for layer ordering: the limiter runs after authentication,
/// so an unauthenticated request is rejected as 401 (never consumes a bucket and
/// never turns into a 429).
#[tokio::test]
async fn unauthenticated_request_is_401_not_429() {
    let db = support::TestDb::create().await.expect("TestDb::create");

    let state = AppState::for_test(db.conn().clone())
        .await
        .expect("AppState::for_test")
        .with_rate_limit(1, 1);
    let server = support::TestServer::spawn_with_state(state).await;

    let base_url = server.base_url().to_string();
    let http = reqwest::Client::new();

    // More requests than the burst; without auth every one must be 401.
    for _ in 0..3 {
        let response = http
            .get(format!("{base_url}/v1/auth/me"))
            .send()
            .await
            .expect("request");
        assert_eq!(
            response.status().as_u16(),
            401,
            "unauthenticated request must be 401, not rate-limited"
        );
    }

    db.teardown().await;
}
