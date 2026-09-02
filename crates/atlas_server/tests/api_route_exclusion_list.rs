#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod support;

use atlas_core::registry::HttpMethod;

/// T5.8, direction (b): proves every entry in
/// `router_audit::ROUTE_SET_EXCLUSIONS` is actually served by the real
/// assembled `atlas_server::app()`.
///
/// `router_audit::tests::route_set_exclusions_are_not_declared_anywhere`
/// (crate-internal, no DB) proves the opposite direction: no exclusion entry
/// is secretly a real declared route. Together the two prove the list is
/// exhaustive in the two directions that CAN be checked — see
/// `ROUTE_SET_EXCLUSIONS`'s own doc for the one direction that cannot be
/// checked from outside the router at all (axum 0.8 has no route
/// enumeration), and why the declarative audits plus SH10 cover that gap
/// structurally instead.
///
/// **Currently a zero-iteration loop** (`v2-e3-s4`, D3): `ROUTE_SET_EXCLUSIONS`
/// is empty as of this slice (`/openapi.json` and `/scalar`, its only two
/// former entries, are now ordinary `platform` `RouteDeclaration`s). Kept,
/// not deleted, as a live, checked escape hatch for a future genuinely-
/// inexpressible route — a `#[test]` that runs zero assertions and passes is
/// a known-empty state, not a silently-vacuous one, since
/// `router_audit::tests::route_set_exclusions_is_empty` asserts the
/// emptiness explicitly.
///
/// Mount proof never uses "not 404" (see `api_router_mount_assertion.rs`'s
/// doc for why): each excluded path is hit with a method it does not
/// register. This mirrors T4.9's mechanism, kept here for whatever entries
/// this list holds in the future.
#[tokio::test]
async fn every_route_set_exclusion_is_actually_served() {
    let db = support::TestDb::create().await.expect("TestDb::create");
    let server = support::TestServer::spawn(&db).await;
    let http = reqwest::Client::new();

    for &(method, path) in atlas_server::router_audit::ROUTE_SET_EXCLUSIONS {
        let foreign_method = foreign_method_for(method);

        let status = http
            .request(
                foreign_method.clone(),
                format!("{}{}", server.base_url(), path),
            )
            .send()
            .await
            .expect("request must not error")
            .status()
            .as_u16();

        assert_eq!(
            status, 405,
            "{path} is in ROUTE_SET_EXCLUSIONS but a foreign method ({foreign_method}) got \
             {status}, not 405 — it does not appear to be mounted in app()",
        );
    }

    db.teardown().await;
}

/// Any method other than the excluded entry's own declared method works as
/// the foreign probe. `POST` is used unconditionally rather than building a
/// full candidate-order search like `api_router_mount_assertion.rs::pick_probe`
/// needs for routes with several declared methods on the same path; this
/// function never runs today (the list is empty, `v2-e3-s4` D3), and the
/// assertion below keeps it honest for whatever entry it holds next.
fn foreign_method_for(declared: HttpMethod) -> reqwest::Method {
    assert_eq!(
        declared,
        HttpMethod::Get,
        "ROUTE_SET_EXCLUSIONS grew a non-GET entry; foreign_method_for must widen its \
         candidate set to keep probing with a genuinely foreign method"
    );

    reqwest::Method::POST
}
