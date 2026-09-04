#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod support;

use std::collections::HashSet;

use atlas_core::registry::HttpMethod;
use support::route_matrix::route_matrix;

/// See `tests/api_401_sweep.rs`'s identical helper. Exhaustive by
/// construction: a method added to `HttpMethod` fails to compile here.
fn reqwest_method(method: HttpMethod) -> reqwest::Method {
    match method {
        HttpMethod::Get => reqwest::Method::GET,
        HttpMethod::Post => reqwest::Method::POST,
        HttpMethod::Put => reqwest::Method::PUT,
        HttpMethod::Patch => reqwest::Method::PATCH,
        HttpMethod::Delete => reqwest::Method::DELETE,
        HttpMethod::Head => reqwest::Method::HEAD,
        HttpMethod::Options => reqwest::Method::OPTIONS,
    }
}

/// T1.8's per-route-class provocation strategy. Every entry in
/// `route_matrix()` (data-driven, INV-DATA-DRIVEN — never a curated list)
/// falls into exactly one of these classes:
///
/// - `Unauthenticated401` — every protected route (203 of 210): no session,
///   no API key. The router's `require_authn` layer rejects before the
///   handler runs.
/// - `LoginBadCredentials401` — `POST /api/auth/login`: a well-formed but
///   unrecognized username/password pair. The handler itself renders 401 —
///   a malformed JSON body would be rejected by axum's `Json` extractor
///   before `ApiError` ever runs, which is not this sweep's subject.
/// - `UnknownActivationToken404` — `GET`/`POST /api/activate/{token}`: a
///   syntactically valid but nonexistent token. `POST` additionally needs a
///   password meeting the minimum-strength check so the handler reaches the
///   token lookup instead of returning 422 first.
/// - `MissingWebhookSignature401` — `POST
///   /api/workspaces/{ws}/integrations/{integration}/events`: the GitHub
///   ingest webhook answers its own 401 when `x-hub-signature-256` is
///   absent (`routes/integrations_ingest.rs:65-66`), by its own contract
///   rather than the `require_authn` layer (it is public, INV-LIVE-PROOF).
/// - `Excluded` — `GET /health`, `/ready`, `/version`, and (`v2-e3-s4` D3)
///   `/openapi.json`, `/scalar`: no auth requirement, no path parameter, no
///   request body. Nothing about the request can be varied to provoke a
///   4xx/5xx, so these five are recorded here rather than silently skipped
///   (T1.8). `/openapi.json`/`/scalar` only became visible to `route_matrix()`
///   once `RoutePath` was widened to accept their `.` and they were
///   registered as ordinary `platform` routes — before that they were
///   entirely absent from the registry, not merely unclassified.
enum Provocation {
    Unauthenticated401,
    LoginBadCredentials401,
    UnknownActivationToken404,
    MissingWebhookSignature401,
    Excluded { reason: &'static str },
}

fn classify(method: HttpMethod, path: &str, is_public: bool) -> Provocation {
    match (method, path) {
        (HttpMethod::Get, "/health")
        | (HttpMethod::Get, "/ready")
        | (HttpMethod::Get, "/version")
        | (HttpMethod::Get, "/openapi.json")
        | (HttpMethod::Get, "/scalar") => Provocation::Excluded {
            reason: "unauthenticated GET, no path parameter, no request body — nothing in \
                         the request can be varied to provoke a 4xx/5xx",
        },
        (HttpMethod::Post, "/auth/login") => Provocation::LoginBadCredentials401,
        (_, p) if p.starts_with("/activate/") => Provocation::UnknownActivationToken404,
        (HttpMethod::Post, p) if p.ends_with("/integrations/{integration}/events") => {
            Provocation::MissingWebhookSignature401
        }
        _ if !is_public => Provocation::Unauthenticated401,
        (m, p) => panic!(
            "route {m} {p} is public but the sweep has no provocation strategy for it — \
             classify it in `classify()` before trusting this sweep"
        ),
    }
}

/// T1.8/T1.9 (`v2-e3-s3` PR1): the RFC 9457 live sweep. Data-driven off
/// every declared route (`route_matrix()`, INV-DATA-DRIVEN), provoking a
/// 4xx per the route's class (`classify`) and asserting every provoked
/// response is `application/problem+json` carrying `type`/`title`/`status`/
/// `request_id`/`instance`, with `request_id` byte-equal to the
/// `x-request-id` header the test itself sent (proving `problem_stamp`
/// actually threads the two together, not two independently-generated
/// values that happen to both exist).
/// T5.11/T5.12/T7.12 (`v2-e3-s4` PR5/PR7, D1/D10), collapsed to one mount by
/// `v2-e3-s7` (D1/U2): data-driven over `route_matrix()`, each entry probed
/// at its own `mounted()` (`/api/v2/<component>`) mount; `INV-NONVACUOUS`
/// (gate 7) asserts the matrix examined is non-empty, so a degenerate
/// collapse that iterated zero routes would fail loudly.
#[tokio::test]
async fn every_declared_route_answers_with_a_conformant_problem_body() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let ws_slug = "rfc9457-sweep-no-such-workspace";
    let matrix = route_matrix();
    assert!(
        !matrix.is_empty(),
        "the sweep must examine at least one declared route, or its assertions pass vacuously"
    );

    {
        let mut excluded = Vec::new();
        let mut provoked_or_excluded: HashSet<(HttpMethod, String)> = HashSet::new();

        for entry in &matrix {
            let namespace = atlas_server::router_audit::v2_namespace(&entry.component);
            let path = entry.path_template.replace("{ws}", ws_slug);
            let url = format!(
                "{}{}",
                server.base_url(),
                atlas_server::router_audit::mounted_path(&namespace, &path)
            );

            let provocation = classify(entry.method, &path, entry.is_public);

            let (expected_status, request_id, response) = match provocation {
                Provocation::Excluded { reason } => {
                    excluded.push((entry.method, path.clone(), reason));
                    provoked_or_excluded.insert((entry.method, entry.path_template.clone()));
                    continue;
                }
                Provocation::Unauthenticated401 => {
                    let request_id = format!("sweep-{namespace}-{}-{}", entry.method, path);
                    let response = http
                        .request(reqwest_method(entry.method), &url)
                        .header("x-request-id", request_id.clone())
                        .send()
                        .await
                        .unwrap_or_else(|e| {
                            panic!("request to {} {} must not error: {e}", entry.method, path)
                        });
                    (401u16, request_id, response)
                }
                Provocation::LoginBadCredentials401 => {
                    let request_id = format!("sweep-{namespace}-login-bad-credentials");
                    let response = http
                        .post(&url)
                        .header("x-request-id", request_id.clone())
                        .json(&atlas_api::dtos::LoginRequest {
                            username: "rfc9457-sweep-no-such-user".to_string(),
                            password: "definitely-wrong-password".to_string(),
                        })
                        .send()
                        .await
                        .expect("login request must not error");
                    (401, request_id, response)
                }
                Provocation::UnknownActivationToken404 => {
                    let request_id = format!("sweep-{namespace}-activate-{}", entry.method);
                    let mut request = http
                        .request(reqwest_method(entry.method), &url)
                        .header("x-request-id", request_id.clone());
                    if entry.method == HttpMethod::Post {
                        request = request.json(&atlas_api::dtos::ActivatePasswordRequest {
                            password: "SweepTest1234!".to_string(),
                        });
                    }
                    let response = request
                        .send()
                        .await
                        .expect("activate request must not error");
                    (404, request_id, response)
                }
                Provocation::MissingWebhookSignature401 => {
                    let request_id = format!("sweep-{namespace}-ingest-missing-signature");
                    let response = http
                        .post(&url)
                        .header("x-request-id", request_id.clone())
                        .header("x-github-delivery", uuid::Uuid::new_v4().to_string())
                        .header("content-type", "application/json")
                        .body("{}")
                        .send()
                        .await
                        .expect("ingest request must not error");
                    (401, request_id, response)
                }
            };
            // Recorded only once a real provocation was issued for this route, so
            // the closing set comparison can fail when a route falls through
            // classification without being provoked or explicitly excluded.
            provoked_or_excluded.insert((entry.method, entry.path_template.clone()));

            let status = response.status().as_u16();
            assert_eq!(
                status, expected_status,
                "namespace {namespace}: expected {expected_status} for {} {} but got {status}",
                entry.method, path
            );

            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned);
            assert_eq!(
                content_type.as_deref(),
                Some("application/problem+json"),
                "namespace {namespace}: expected application/problem+json for {} {} but got \
             {content_type:?}",
                entry.method,
                path
            );

            let body: serde_json::Value = response.json().await.unwrap_or_else(|e| {
                panic!(
                    "namespace {namespace}: body for {} {} must be JSON: {e}",
                    entry.method, path
                )
            });

            for field in ["type", "title", "status", "request_id", "instance"] {
                assert!(
                    body.get(field).is_some(),
                    "namespace {namespace}: problem body for {} {} is missing required field \
                 `{field}`: {body:?}",
                    entry.method,
                    path
                );
            }

            assert_eq!(
                body.get("request_id").and_then(|v| v.as_str()),
                Some(request_id.as_str()),
                "namespace {namespace}: request_id in the problem body for {} {} does not match the \
             x-request-id header sent",
                entry.method,
                path
            );
        }

        assert_eq!(
            excluded,
            vec![
                (
                    HttpMethod::Get,
                    "/health".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/ready".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/version".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/openapi.json".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/scalar".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/health".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/ready".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/health".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
                (
                    HttpMethod::Get,
                    "/ready".to_string(),
                    "unauthenticated GET, no path parameter, no request body — nothing in the request can be varied to provoke a 4xx/5xx"
                ),
            ],
            "the excluded-route set drifted from the expected, hand-justified \
         nine (platform's five root probes, then custos's and acta's own \
         `/health` and `/ready`) — a new exclusion must be justified here, \
         not silently added"
        );
        // INV-SET: never a `.len()` comparison — assert set equality in both
        // directions and name the offending `(method, path)` entries on failure.
        // `provoked_or_excluded` is only populated inside the provocation arms
        // above, after classification, so a route that classification drops
        // without provoking or excluding shows up as declared-but-not-covered.
        let declared: HashSet<(HttpMethod, String)> = matrix
            .iter()
            .map(|entry| (entry.method, entry.path_template.clone()))
            .collect();

        let declared_but_not_provoked_or_excluded: Vec<_> = declared
            .difference(&provoked_or_excluded)
            .cloned()
            .collect();
        let provoked_or_excluded_but_not_declared: Vec<_> = provoked_or_excluded
            .difference(&declared)
            .cloned()
            .collect();

        assert!(
            declared_but_not_provoked_or_excluded.is_empty()
                && provoked_or_excluded_but_not_declared.is_empty(),
            "the declared route set and the provoked-or-excluded route set \
         disagree; declared but not provoked or excluded: \
         {declared_but_not_provoked_or_excluded:?}; provoked or excluded but not declared: \
         {provoked_or_excluded_but_not_declared:?}"
        );
    }

    db.teardown().await;
}
