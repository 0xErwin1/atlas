#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

//! `v2-e3-s3` PR4 (T4.15–T4.17, D6), plus a scoped correction to
//! `R4-5xx-release-duplicates-one-shot-jobs`: the idempotency middleware
//! declare-and-verify audit. Proves the SET of routes actually wired to
//! `idempotency_middleware_release`/`idempotency_middleware_store_briefly`
//! (via `component_routes!`'s `idempotent`/`one_shot` modifiers, or a
//! hand-declared `AuditedRoute` literal for the ten routes outside the
//! macro) is EXACTLY the set of registry entries with `idempotent: true` —
//! INV-SET, bidirectional, offenders named, reused from
//! `router_audit::diff_route_sets` (S2's own bidirectional comparison, never
//! reimplemented). The bottom of this file extends the same proof to the
//! `one_shot`/`StoreBriefly` 5xx-policy split.

use atlas_core::registry::{HttpMethod, build};
use atlas_server::reg5::{StorageBackend, reg5_component_entries};
use atlas_server::router_audit::{
    ONE_SHOT_IDEMPOTENT_ROUTES, acta_idempotent_route_paths, acta_one_shot_route_paths,
    custos_idempotent_route_paths, diff_route_sets, platform_idempotent_route_paths,
};
use std::collections::HashSet;

fn wired_idempotent_set() -> HashSet<(HttpMethod, String)> {
    platform_idempotent_route_paths()
        .into_iter()
        .chain(custos_idempotent_route_paths())
        .chain(acta_idempotent_route_paths())
        .map(|(method, path)| (method, path.to_string()))
        .collect()
}

/// [`ONE_SHOT_IDEMPOTENT_ROUTES`] as a `(method, path)` set, dropping each
/// entry's reason string.
fn one_shot_truth_set() -> HashSet<(HttpMethod, String)> {
    ONE_SHOT_IDEMPOTENT_ROUTES
        .iter()
        .map(|(method, path, _reason)| (*method, path.to_string()))
        .collect()
}

/// The `(method, path)` set `component_routes!`'s `one_shot` modifier
/// actually wired to `idempotency_middleware_store_briefly`. Only `acta`
/// owns any `one_shot` route today (`platform`/`custos` contribute none).
fn wired_store_briefly_set() -> HashSet<(HttpMethod, String)> {
    acta_one_shot_route_paths()
        .into_iter()
        .map(|(method, path)| (method, path.to_string()))
        .collect()
}

fn declared_true_set() -> HashSet<(HttpMethod, String)> {
    let registry = build(reg5_component_entries(StorageBackend::Filesystem))
        .expect("REG-5 entries must satisfy every registry::build() validator");

    registry
        .entries()
        .iter()
        .flat_map(|component| component.api.routes.iter())
        .filter(|route| route.idempotent)
        .map(|route| (route.method, route.path.as_str().to_string()))
        .collect()
}

/// T4.15/T4.16: the wired set and the declared-true set must match exactly,
/// in both directions.
#[test]
fn wired_middleware_set_matches_registry_idempotent_true_set_exactly() {
    let wired = wired_idempotent_set();
    let declared = declared_true_set();

    let diff = diff_route_sets(&wired, &declared);

    assert!(
        diff.left_only.is_empty(),
        "wired to the idempotency middleware but NOT declared idempotent:true in the \
         registry: {:?}",
        diff.left_only
    );
    assert!(
        diff.right_only.is_empty(),
        "declared idempotent:true in the registry but NOT wired to the idempotency \
         middleware: {:?}",
        diff.right_only
    );
    assert_eq!(
        wired.len(),
        34,
        "expected exactly 34 wired idempotent routes"
    );
    assert_eq!(
        declared.len(),
        34,
        "expected exactly 34 declared idempotent:true routes"
    );
}

/// T4.17: the audit has teeth — a set missing one known-true route must
/// fail and name it (proven on a local copy, `reg5.rs` is never mutated).
#[test]
fn a_route_missing_from_the_wired_set_fails_the_audit_and_is_named() {
    let mut wired = wired_idempotent_set();
    let declared = declared_true_set();

    let removed = (HttpMethod::Post, "/workspaces".to_string());
    assert!(
        wired.remove(&removed),
        "sanity: create_workspace must be in the wired set before removal"
    );

    let diff = diff_route_sets(&wired, &declared);

    assert!(diff.left_only.is_empty());
    assert_eq!(
        diff.right_only,
        vec![removed],
        "removing exactly one entry must produce exactly one named offender"
    );
}

/// T4.17, the mirror direction: a set with one EXTRA route not declared
/// `idempotent: true` must also fail and name it.
#[test]
fn an_extra_wired_route_not_declared_true_fails_the_audit_and_is_named() {
    let mut wired = wired_idempotent_set();
    let declared = declared_true_set();

    // A route that IS declared but is idempotent:false (logout) — pretend
    // it got wired by mistake.
    let bogus = (HttpMethod::Post, "/auth/logout".to_string());
    assert!(
        wired.insert(bogus.clone()),
        "sanity: logout must not already be in the wired set"
    );

    let diff = diff_route_sets(&wired, &declared);

    assert_eq!(diff.left_only, vec![bogus]);
    assert!(diff.right_only.is_empty());
}

/// D6 scoped correction: `router_audit::ONE_SHOT_IDEMPOTENT_ROUTES` must be a subset
/// of the declared-`idempotent: true` set — a one-shot route that is not
/// even declared idempotent would be a contradiction the registry itself
/// should have caught first.
#[test]
fn one_shot_routes_are_a_subset_of_the_declared_idempotent_true_set() {
    let declared = declared_true_set();

    for (method, path) in one_shot_truth_set() {
        assert!(
            declared.contains(&(method, path.clone())),
            "{method:?} {path} is in ONE_SHOT_IDEMPOTENT_ROUTES but is NOT declared \
             idempotent:true in the registry"
        );
    }
}

/// D6 scoped correction (INV-SET, bidirectional): the set of routes
/// `component_routes!`'s `one_shot` modifier actually wired to
/// `idempotency_middleware_store_briefly` must equal
/// `router_audit::ONE_SHOT_IDEMPOTENT_ROUTES` exactly — same "one macro expansion,
/// one fact" property already proven for `idempotent` itself, extended to
/// the 5xx policy split.
#[test]
fn wired_store_briefly_set_matches_one_shot_idempotent_routes_exactly() {
    let wired = wired_store_briefly_set();
    let truth = one_shot_truth_set();

    let diff = diff_route_sets(&wired, &truth);

    assert!(
        diff.left_only.is_empty(),
        "wired to idempotency_middleware_store_briefly but NOT named in \
         ONE_SHOT_IDEMPOTENT_ROUTES: {:?}",
        diff.left_only
    );
    assert!(
        diff.right_only.is_empty(),
        "named in ONE_SHOT_IDEMPOTENT_ROUTES but NOT wired to \
         idempotency_middleware_store_briefly: {:?}",
        diff.right_only
    );
    assert_eq!(
        wired.len(),
        2,
        "expected exactly the two named one-shot routes"
    );
}

/// T4c: the real `purge_trash` route is a member of the one-shot set (and,
/// by the audit above, is actually wired to the `StoreBriefly` policy) —
/// `idempotency_live_sweep.rs`'s data-driven sweep already exercises it as
/// part of every declared-`idempotent: true` route and is unaffected by the
/// 5xx-policy split, since none of its three requests provoke a 5xx.
#[test]
fn purge_trash_is_a_member_of_the_one_shot_set() {
    let purge_trash = (HttpMethod::Post, "/admin/trash/purge".to_string());
    assert!(
        one_shot_truth_set().contains(&purge_trash),
        "purge_trash must be named in ONE_SHOT_IDEMPOTENT_ROUTES"
    );
    assert!(
        wired_store_briefly_set().contains(&purge_trash),
        "purge_trash must actually be wired to idempotency_middleware_store_briefly"
    );
}
