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
/// Mount proof never uses "not 404" (see `api_router_mount_assertion.rs`'s
/// doc for why): each excluded path is hit with a method it does not
/// register. Neither `/openapi.json` nor `/scalar` sits behind
/// `require_authn` (both are mounted in `routes::acta::public::router`), so
/// an unmounted path would answer 404 for every method while a mounted one
/// answers 405 for a foreign method regardless of what its real method
/// returns — the exact mechanism T4.9 already established.
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
/// the foreign probe; `POST` is never one of `ROUTE_SET_EXCLUSIONS`'s own
/// methods (both current entries are `GET`), so it is used unconditionally
/// rather than building a full candidate-order search like
/// `api_router_mount_assertion.rs::pick_probe` needs for routes with several
/// declared methods on the same path.
fn foreign_method_for(declared: HttpMethod) -> reqwest::Method {
    assert_eq!(
        declared,
        HttpMethod::Get,
        "ROUTE_SET_EXCLUSIONS grew a non-GET entry; foreign_method_for must widen its \
         candidate set to keep probing with a genuinely foreign method"
    );

    reqwest::Method::POST
}
