#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_api::dtos::LoginRequest;

/// T3.9 (`v2-e3-s2-router-audit` PR3): `login`'s `tower_governor` limiter
/// (`burst_size(5)`/`per_second(1)`, moved from `lib.rs` into
/// `routes::custos::router`'s `public` sub-module) must still apply at
/// exactly the same threshold post-move, and it must apply ONLY to `login` —
/// this is the one route-specific (not router-wide) layer in the whole
/// conversion, and therefore the highest-risk single layering change in this
/// PR.
///
/// The request bodies use wrong credentials on purpose: a rejected login
/// (401) still consumes the governor's per-IP bucket the same way a
/// successful one would (the limiter sits in front of the handler), and using
/// wrong credentials means the test does not depend on any seeded user
/// existing.
#[tokio::test]
async fn login_burst_is_rate_limited_and_activate_is_not() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let base_url = server.base_url().to_string();
    let http = reqwest::Client::new();

    // Fire concurrently so every request reaches the governor's gate before
    // any token-bucket refill can happen (burst_size(5), per_second(1)).
    let login_futures: Vec<_> = (0..10)
        .map(|_| {
            http.post(format!("{base_url}/api/auth/login"))
                .json(&LoginRequest {
                    username: "no-such-user".to_string(),
                    password: "wrong-password".to_string(),
                })
                .send()
        })
        .collect();

    let login_responses = futures::future::join_all(login_futures).await;

    let mut saw_429 = false;
    let mut saw_401 = false;
    let mut retry_after_present = false;
    let mut statuses = Vec::new();

    for result in login_responses {
        let response = result.expect("login request must not error transport-wise");
        let status = response.status().as_u16();
        statuses.push(status);

        match status {
            429 => {
                saw_429 = true;
                if response
                    .headers()
                    .get(reqwest::header::RETRY_AFTER)
                    .is_some()
                {
                    retry_after_present = true;
                }
            }
            401 => saw_401 = true,
            other => panic!("unexpected status {other} from /api/auth/login burst: {statuses:?}"),
        }
    }

    assert!(
        saw_429,
        "a burst of 10 requests past burst_size(5) must produce at least one 429; got: {statuses:?}"
    );
    assert!(
        saw_401,
        "at least the first requests (within the burst) must still reach the handler and get \
         401 for wrong credentials; got: {statuses:?}"
    );
    assert!(
        retry_after_present,
        "the 429 response must carry a Retry-After header, matching the pre-move governor's \
         behavior"
    );

    // Scope check: `activate` (a sibling public, unauthenticated custos
    // route with its OWN separate governor instance) must be completely
    // unaffected by login's exhausted bucket — it is keyed by a different
    // route entirely, not the same limiter instance, and definitely not a
    // router-wide layer.
    let activate_response = http
        .get(format!("{base_url}/api/activate/bogus-token"))
        .send()
        .await
        .expect("activate request must not error transport-wise");
    assert_eq!(
        activate_response.status().as_u16(),
        404,
        "an unrelated public custos route must not be rate-limited by login's exhausted bucket"
    );

    db.teardown().await;
}
