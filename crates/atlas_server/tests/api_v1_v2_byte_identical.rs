#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

//! T5.5/T5.6/T7.10 (`v2-e3-s4` PR5/PR7, D1/D10, spec AMENDMENT "V2 prefix
//! per component"): for every declared route of component `C`, the same
//! request sent to `/api/<rel>` and `/api/v2/<C>/<rel>` produces the same
//! behavior at both mounts. Exhaustive over `route_matrix()`
//! (INV-DATA-DRIVEN), not sampled.
//!
//! What is compared, per route:
//!
//! - the status code, exactly;
//! - every response header except `x-request-id`, `content-length`, and any
//!   date-like header (see `is_excluded_header` for why each is dropped);
//! - the body. A non-problem body is compared as raw bytes. A
//!   `application/problem+json` body is NOT byte-identical by construction:
//!   `problem_stamp` runs at the composition root (`lib.rs::apply_layers`)
//!   and stamps `instance` with the full request path, so the `/api` mount
//!   carries `"instance":"/api/<rel>"` and the `/api/v2/<C>` mount carries
//!   `"instance":"/api/v2/<C>/<rel>"` (which is also why `content-length` is
//!   excluded above). For those bodies the test parses both as JSON,
//!   asserts each mount's `instance` equals that mount's own path (an
//!   `instance` that names the wrong mount fails here, naming the route and
//!   namespace), removes `instance` from both, and compares the remaining
//!   JSON values. Everything else in a problem body (`type`, `title`,
//!   `status`, `detail`, `request_id`, every extension field) must match.
//!
//! Every request in this sweep is unauthenticated and carries no body: for
//! a protected route this is a deterministic 401 from `require_authn`,
//! which runs before any per-route layer (including the `Idempotency-Key`
//! middleware) — no idempotent route's dedup store is ever touched here.
//! (The store keys on the canonical `/api` form of the path, so the two
//! mounts dedup each other; `tests/idempotency_middleware.rs`'s
//! `the_same_key_replays_across_namespaces` proves that with an
//! authenticated request.) The registry declares 9 public routes:
//! 5 root-level (`router_audit::ROOT_LEVEL_PATHS`, served unprefixed and
//! therefore outside both mounts — skipped here, since `mounted_path`
//! returns the bare path for every namespace and a comparison of one URL
//! against itself proves nothing) and 4 namespaced. For the namespaced
//! public routes, the same no-body/no-session request is issued at both
//! mounts and compared only against itself, never against a fixed expected
//! value: whatever the handler actually returns (a real 200, a validation
//! 4xx) is exactly what this test proves is namespace-independent, so it
//! needs no per-route provocation table of its own.

mod support;

use std::collections::{BTreeMap, BTreeSet};

use atlas_core::registry::HttpMethod;
use atlas_server::router_audit::{ROOT_LEVEL_PATHS, mounted_path};
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

/// Replaces every `{param}` path segment with a fixed, syntactically valid
/// placeholder. The substituted value never matters for this test: the same
/// substituted path is sent to both mount points, so whatever status or body
/// the substitution provokes is compared against itself, never against a
/// fixed expectation.
fn substitute_path_params(template: &str) -> String {
    let mut result = String::new();
    let mut chars = template.chars();
    while let Some(c) = chars.by_ref().next() {
        if c == '{' {
            for c2 in chars.by_ref() {
                if c2 == '}' {
                    break;
                }
            }
            result.push_str("mount-fixture");
        } else {
            result.push(c);
        }
    }
    result
}

/// Header names excluded from the cross-namespace comparison below. The
/// request id is pinned to the same value on both calls this test issues
/// (so a server that only echoes the caller's value already matches without
/// this exclusion), but the exclusion also covers a route that generates
/// its own id rather than echoing the caller's. Date-like headers carry
/// wall-clock time, which legitimately differs between the two sequential
/// requests this test issues for the same route. `content-length` is
/// excluded because a problem body's `instance` field names the mount it
/// was served from (see the module doc), so the two bodies differ in length
/// by exactly the `/v2/<component>` segment; the body comparison below still
/// proves the bodies agree on everything but that field.
fn is_excluded_header(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower == "x-request-id" || lower == "content-length" || lower.contains("date")
}

fn normalized_headers(response: &reqwest::Response) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (name, value) in response.headers() {
        if is_excluded_header(name.as_str()) {
            continue;
        }
        map.entry(name.as_str().to_ascii_lowercase())
            .or_default()
            .push(value.to_str().unwrap_or("<non-utf8>").to_string());
    }
    for values in map.values_mut() {
        values.sort();
    }
    map
}

fn is_problem_json(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.contains("application/problem+json"))
}

/// One mount's observed response, reduced to what the cross-namespace
/// comparison looks at. `Problem` carries the body with `instance` already
/// verified against `mounted` and removed; `Raw` carries the exact bytes.
enum ComparableBody {
    Problem(serde_json::Value),
    Raw(bytes::Bytes),
}

struct MountResponse {
    namespace: String,
    mounted: String,
    status: u16,
    headers: BTreeMap<String, Vec<String>>,
    body: ComparableBody,
}

/// Parses a `application/problem+json` body, asserts its `instance` names
/// exactly the mount it was served from, and returns the body without
/// `instance` so the two mounts' bodies can be compared for everything
/// else. Panics name the route, the namespace, and the offending value.
fn strip_verified_instance(
    body: &[u8],
    method: HttpMethod,
    template: &str,
    namespace: &str,
    mounted: &str,
) -> serde_json::Value {
    let mut value: serde_json::Value = serde_json::from_slice(body).unwrap_or_else(|e| {
        panic!(
            "{method} {template}: {namespace} problem+json body must parse as JSON: {e}; body: \
             {:?}",
            String::from_utf8_lossy(body)
        )
    });

    let object = value.as_object_mut().unwrap_or_else(|| {
        panic!("{method} {template}: {namespace} problem+json body must be a JSON object")
    });

    let instance = object.remove("instance").unwrap_or_else(|| {
        panic!("{method} {template}: {namespace} problem+json body must carry `instance`")
    });

    assert_eq!(
        instance.as_str(),
        Some(mounted),
        "{method} {template}: {namespace} problem+json `instance` must equal the mounted path \
         {mounted}, got {instance}"
    );

    value
}

fn assert_bodies_match(
    first: &MountResponse,
    other: &MountResponse,
    method: HttpMethod,
    template: &str,
) {
    match (&first.body, &other.body) {
        (ComparableBody::Problem(first_body), ComparableBody::Problem(other_body)) => {
            assert_eq!(
                other_body, first_body,
                "{method} {template}: problem+json body (with `instance` removed) differs between \
                 {} and {}",
                first.namespace, other.namespace
            );
        }
        (ComparableBody::Raw(first_body), ComparableBody::Raw(other_body)) => {
            assert_eq!(
                other_body, first_body,
                "{method} {template}: body differs between {} and {}",
                first.namespace, other.namespace
            );
        }
        _ => panic!(
            "{method} {template}: one mount answered problem+json and the other did not ({} vs \
             {}); headers already compared equal, so content-type disagreement is a test bug",
            first.namespace, other.namespace
        ),
    }
}

#[tokio::test]
async fn every_declared_route_answers_identically_at_both_mounts() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    let mut skipped_root_level: BTreeSet<String> = BTreeSet::new();

    for entry in route_matrix() {
        if ROOT_LEVEL_PATHS.contains(&entry.path_template.as_str()) {
            skipped_root_level.insert(entry.path_template.clone());
            continue;
        }

        let relative = substitute_path_params(&entry.path_template);
        let request_id = format!("dual-mount-{}-{}", entry.method, relative);

        let namespaces = entry.namespaces();
        let mut responses = Vec::with_capacity(namespaces.len());
        for namespace in namespaces {
            let mounted = mounted_path(&namespace, &relative);
            let url = format!("{}{}", server.base_url(), mounted);

            let response = http
                .request(reqwest_method(entry.method), &url)
                .header("x-request-id", request_id.clone())
                .send()
                .await
                .unwrap_or_else(|e| {
                    panic!(
                        "request to {namespace} {} must not error: {e}",
                        entry.path_template
                    )
                });

            let status = response.status().as_u16();
            let headers = normalized_headers(&response);
            let problem = is_problem_json(&response);
            let bytes = response.bytes().await.unwrap_or_else(|e| {
                panic!(
                    "body for {namespace} {} must be readable: {e}",
                    entry.path_template
                )
            });

            let body = if problem {
                ComparableBody::Problem(strip_verified_instance(
                    &bytes,
                    entry.method,
                    &entry.path_template,
                    &namespace,
                    &mounted,
                ))
            } else {
                ComparableBody::Raw(bytes)
            };

            responses.push(MountResponse {
                namespace,
                mounted,
                status,
                headers,
                body,
            });
        }

        let first = &responses[0];
        for other in &responses[1..] {
            assert_eq!(
                other.status,
                first.status,
                "{} {}: status differs between {} ({}) and {} ({})",
                entry.method,
                entry.path_template,
                first.namespace,
                first.status,
                other.namespace,
                other.status
            );
            assert_eq!(
                other.headers,
                first.headers,
                "{} {}: headers (other than request id / content-length / date-like) differ \
                 between {} ({}) and {} ({})",
                entry.method,
                entry.path_template,
                first.namespace,
                first.mounted,
                other.namespace,
                other.mounted
            );
            assert_bodies_match(first, other, entry.method, &entry.path_template);
        }
    }

    let expected_root_level: BTreeSet<String> = ROOT_LEVEL_PATHS
        .iter()
        .map(|path| (*path).to_string())
        .collect();
    assert_eq!(
        skipped_root_level, expected_root_level,
        "the set of routes skipped as root-level must be exactly ROOT_LEVEL_PATHS: a registry \
         route can only be skipped here by joining ROOT_LEVEL_PATHS, and every ROOT_LEVEL_PATHS \
         member must still be declared by the registry"
    );

    db.teardown().await;
}
