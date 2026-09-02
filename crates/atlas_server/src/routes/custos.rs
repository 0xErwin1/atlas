//! `custos` component router (design D1/D5/D6, `v2-e3-s2-router-audit`
//! PR3).
//!
//! Custos's 35 routes split the same way they do in today's `lib.rs`: `login`
//! and `activate` (GET+POST) sit in the `public` router, unauthenticated,
//! each carrying its own route-specific `tower_governor` rate limiter
//! (`lib.rs:137-146` and `lib.rs:768-778` pre-PR3); every other custos route
//! sits in the `protected` router behind `require_authn` →
//! `require_rate_limit` → CSRF-for-cookie-mutations (`lib.rs:752-765`
//! pre-PR3), the same stack `platform`'s protected split already reproduces
//! (PR2).
//!
//! ## The login/activate governor and why they sit outside `component_routes!`
//!
//! `component_routes!` folds every method on one path into a single
//! `.route(path, method_router)` call and has no grammar for a per-route
//! `.layer(...)` — every other route in this crate is layered at the
//! sub-router level (the public/protected split, D6), never per-route.
//! `login` and `activate` are the only two custos routes that need a
//! layer at the individual `.route()` granularity, matching exactly what
//! `lib.rs` does today: `axum::routing::post(login).layer(GovernorLayer::new(..))`.
//!
//! Design D5's PR3 instructions offer two ways to handle this: extend the
//! macro's grammar, or keep the route outside the macro and hand-declare its
//! `AuditedRoute` entry so the bidirectional audit stays exhaustive. This
//! module takes the second path for both routes (design named only `login`
//! explicitly, but `activate` carries the identical per-route-governor shape
//! in the current `lib.rs` — verified by reading `lib.rs` directly rather than
//! assuming symmetry with `login`, so it gets the same treatment). Growing
//! `component_routes!`'s grammar for two routes out of 35 would add optional,
//! nested macro captures around the existing repetition for a case this
//! module's `public::router()` already expresses directly and readably as
//! plain axum calls — the same shape `lib.rs` has today, just relocated.
//!
//! The tradeoff, stated plainly: for every OTHER custos route,
//! `router()`/`declared_routes()` are two outputs of the same
//! `component_routes!` invocation, so a route cannot exist in one without a
//! matching entry in the other (D1, SH10). For `login` and `activate`,
//! `public::declared_routes()` is a hand-written `Vec` that a future edit to
//! `public::router()` is NOT compiler-enforced to stay in sync with — the
//! same class of drift D1 exists to prevent, now re-introduced for exactly
//! two routes. This does not create a NEW category of audit gap: per D2 (see
//! `routes::platform`'s own module doc), no per-component audit in this slice
//! ever probes the live `axum::Router` — the bidirectional test compares
//! `declared_routes()` against the registry, never against a live router. So
//! `login`/`activate`'s hand-declared entries carry exactly the coverage
//! every other route already has (declared-vs-registry, not
//! declared-vs-router), not less.
#![allow(
    unreachable_pub,
    reason = "component_routes! always emits `pub fn router` to match the real \
              per-component contract (D1); this module's own path is `pub(crate)` \
              (`routes::custos` in `routes/mod.rs`), so the `pub` here is never \
              reachable from outside the crate, which is by design, not an oversight"
)]

use axum::Router;

use crate::router_audit::AuditedRoute;
use crate::state::AppState;

/// `login` and `activate` — unauthenticated, each carrying its own
/// route-specific rate limiter (pre-PR3: `lib.rs:137-146`, `lib.rs:768-778`).
/// See the module doc for why these two sit outside `component_routes!`.
mod public {
    use std::sync::Arc;

    use atlas_core::registry::HttpMethod;
    use axum::Router;
    use tower_governor::{GovernorLayer, governor::GovernorConfigBuilder};

    use crate::router_audit::{AuditedRoute, DeclaredScope};
    use crate::routes::{activate, auth};
    use crate::state::AppState;

    pub fn router(state: AppState) -> Router {
        // burst_size(5) and per_second(1) are non-zero, so finish() always
        // returns Some here (identical construction to pre-PR3 `lib.rs:139-146`).
        #[allow(clippy::expect_used)]
        let login_config = {
            let mut b = GovernorConfigBuilder::default();
            let cfg = b
                .per_second(1)
                .burst_size(5)
                .finish()
                .expect("governor config");
            Arc::new(cfg)
        };

        // burst_size(5) and per_second(1) are non-zero, so finish() always
        // returns Some here (identical construction to pre-PR3 `lib.rs:770-777`).
        #[allow(clippy::expect_used)]
        let activate_config = {
            let mut b = GovernorConfigBuilder::default();
            let cfg = b
                .per_second(1)
                .burst_size(5)
                .finish()
                .expect("governor config");
            Arc::new(cfg)
        };

        Router::new()
            .route(
                "/auth/login",
                axum::routing::post(auth::login).layer(GovernorLayer::new(login_config)),
            )
            .route(
                "/activate/{token}",
                axum::routing::get(activate::get_activation_info)
                    .post(activate::post_activate)
                    .layer(GovernorLayer::new(activate_config)),
            )
            .with_state(state)
    }

    /// Hand-declared (see module doc): neither route takes an
    /// `Authorized<...>` extractor at all, so both are capability-extraction
    /// exempt (D5), matching the registry's `action: None` for both.
    pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
        vec![
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/auth/login",
                scope: DeclaredScope::Unauthenticated,
                // D8: no Principal exists before login succeeds — the
                // mechanism has no principal_id to scope dedup by.
                idempotent: false,
                one_shot: false,
            },
            AuditedRoute {
                method: HttpMethod::Get,
                path: "/activate/{token}",
                scope: DeclaredScope::Unauthenticated,
                idempotent: false,
                one_shot: false,
            },
            AuditedRoute {
                method: HttpMethod::Post,
                path: "/activate/{token}",
                scope: DeclaredScope::Unauthenticated,
                // D8: token-in-path auth, not a Principal from require_authn.
                idempotent: false,
                one_shot: false,
            },
        ]
    }
}

/// Every other custos route: behind `require_authn` → `require_rate_limit` →
/// CSRF-for-cookie-mutations today (`lib.rs`'s `protected` router, pre-PR3).
///
/// Exemption scaffold (D5) applies to every route here whose handler takes a
/// gate OTHER than `Authorized<R, M, S>` — `RequireUserAdmin`, `RequireRoot`,
/// `WorkspaceOwnerOrAdmin`, or a bare `Extension<Principal>` /
/// `Extension<AuthPrincipal>` — verified by reading each handler's real
/// signature directly (`routes::users`, `routes::auth`, `routes::api_keys`,
/// `routes::groups`, `routes::audit`), not inferred from the registry's
/// `action: None`. None of these types are `Authorized<...>`, so
/// `declared_scope()` has no `S` to extract — the same exemption category PR2
/// established for `health`/`ready`/`version`/`meta`, extended here to every
/// non-`Authorized` gate, not just "no gate at all". Only the six
/// grants-family routes below (`*_project_grant`, `*_workspace_grant`) take
/// `Authorized<R, M, S>` and go through real capability extraction — the
/// first `Some(_)` exercise of `capability_from_action_id` (two of the six:
/// `list_project_grants`, `list_workspace_grants`, both
/// `custos::grants::read`).
mod protected {
    use crate::routes::{api_keys, audit, auth, grants, groups, users};
    use crate::state::AppState;

    crate::component_routes! {
        state: AppState;
        "/auth/logout" => [ post(auth::logout, exempt) ];
        "/auth/me" => [ get(auth::me, exempt) ];
        "/auth/change-password" => [ post(auth::change_password, exempt) ];
        "/users/me" => [ patch(auth::update_me, exempt) ];
        "/users" => [
            post(users::create_user, exempt),
            get(users::list_users, exempt)
        ];
        "/users/{user_id}/disable" => [ post(users::disable_user, exempt) ];
        "/users/{user_id}/enable" => [ post(users::enable_user, exempt) ];
        "/users/{user_id}/reset-password" => [ post(users::reset_password, exempt) ];
        "/users/{user_id}/activation-link" => [
            post(users::regenerate_activation_link, exempt)
        ];
        "/users/{user_id}/system-admin" => [ post(users::set_system_admin, exempt) ];
        "/users/{user_id}/memberships" => [ get(users::list_user_memberships, exempt) ];
        "/admin/audit" => [ get(audit::list_platform_audit, exempt) ];
        "/api-keys" => [
            post(api_keys::create_user_api_key, exempt),
            get(api_keys::list_user_api_keys, exempt)
        ];
        "/api-keys/{key_id}" => [
            delete(api_keys::revoke_user_api_key, exempt),
            patch(api_keys::update_user_api_key, exempt)
        ];
        "/api-keys/{key_id}/grants" => [ get(api_keys::list_api_key_grants, exempt) ];
        "/api-keys/{key_id}/grants/{grant_id}" => [
            delete(api_keys::delete_api_key_grant, exempt)
        ];
        "/workspaces/{ws}/projects/{project_slug}/grants" => [
            post(grants::create_project_grant, idempotent),
            get(grants::list_project_grants)
        ];
        "/workspaces/{ws}/projects/{project_slug}/grants/{grant_id}" => [
            delete(grants::delete_project_grant)
        ];
        "/workspaces/{ws}/grants" => [
            post(grants::create_workspace_grant, idempotent),
            get(grants::list_workspace_grants)
        ];
        "/workspaces/{ws}/grants/{grant_id}" => [ delete(grants::delete_workspace_grant) ];
        "/workspaces/{ws}/groups" => [
            post(groups::create_group, exempt, idempotent),
            get(groups::list_groups, exempt)
        ];
        "/workspaces/{ws}/groups/{group_id}" => [ delete(groups::delete_group, exempt) ];
        "/workspaces/{ws}/groups/{group_id}/members" => [
            post(groups::add_group_member, exempt, idempotent),
            get(groups::list_group_members, exempt)
        ];
        "/workspaces/{ws}/groups/{group_id}/members/{user_id}" => [
            delete(groups::remove_group_member, exempt)
        ];
        "/workspaces/{ws}/audit" => [ get(audit::list_workspace_audit, exempt) ];
    }
}

/// Builds custos's router, reproducing today's exact public/protected split
/// and layer stack (D6): no layer added, removed, or reordered — only the
/// `.route()` call sites moved out of `lib.rs::app()` into this module. The
/// login/activate governors move at the same per-route granularity they have
/// today (see the module doc).
pub fn router(state: AppState) -> Router {
    let public = public::router(state.clone());
    let protected = super::protection::protect(protected::router(state.clone()), state);

    public.merge(protected)
}

/// The union of both sub-routers' declared routes. Order-independent: the
/// audits below compare sets, never sequences. Only this module's own
/// bidirectional/declare-and-verify audit tests call the full union today
/// (T4.9's mount assertion deliberately uses the narrower
/// `public_declared_routes()` below instead); PR5's T5.7 crate-wide sweep is
/// this function's next production caller.
#[allow(
    dead_code,
    reason = "only this module's own #[cfg(test)] audit tests call the full \
              declared_routes() union outside a test build; PR5's T5.7 crate-wide \
              sweep is its next production caller"
)]
pub(crate) fn declared_routes() -> Vec<AuditedRoute> {
    let mut routes = public::declared_routes();
    routes.extend(protected::declared_routes());
    routes
}

/// Just `login`/`activate` (`public`, above), read by
/// `router_audit::custos_route_paths()` for T4.9's per-component mount
/// assertion: unlike the full `declared_routes()` union, every route here is
/// safe to probe with an unauthenticated request and a foreign method,
/// since neither sits behind `require_authn` — a probe against a
/// `protected` route would get 401 from that layer before ever reaching the
/// method dispatch that would otherwise answer 405.
pub(crate) fn public_declared_routes() -> Vec<AuditedRoute> {
    public::declared_routes()
}

/// `custos`'s own OpenAPI fragment (D4): exactly the paths/schemas this
/// component's own `router()` mounts, nothing else — the `custos`-owned
/// subset of what used to be `openapi::ApiDoc`'s single 401-path list.
#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        crate::routes::activate::get_activation_info,
        crate::routes::activate::post_activate,
        crate::routes::auth::login,
        crate::routes::auth::logout,
        crate::routes::auth::me,
        crate::routes::auth::change_password,
        crate::routes::auth::update_me,
        crate::routes::users::list_users,
        crate::routes::users::create_user,
        crate::routes::users::disable_user,
        crate::routes::users::enable_user,
        crate::routes::users::reset_password,
        crate::routes::users::set_system_admin,
        crate::routes::users::regenerate_activation_link,
        crate::routes::users::list_user_memberships,
        crate::routes::audit::list_workspace_audit,
        crate::routes::audit::list_platform_audit,
        crate::routes::api_keys::create_user_api_key,
        crate::routes::api_keys::list_user_api_keys,
        crate::routes::api_keys::revoke_user_api_key,
        crate::routes::api_keys::update_user_api_key,
        crate::routes::api_keys::list_api_key_grants,
        crate::routes::api_keys::delete_api_key_grant,
        crate::routes::grants::create_project_grant,
        crate::routes::grants::list_project_grants,
        crate::routes::grants::delete_project_grant,
        crate::routes::grants::create_workspace_grant,
        crate::routes::grants::list_workspace_grants,
        crate::routes::grants::delete_workspace_grant,
        crate::routes::groups::create_group,
        crate::routes::groups::list_groups,
        crate::routes::groups::delete_group,
        crate::routes::groups::add_group_member,
        crate::routes::groups::remove_group_member,
        crate::routes::groups::list_group_members,
    ),
    components(schemas(
        atlas_api::dtos::audit::AuditEntryDto,
        atlas_api::dtos::groups::AddGroupMemberRequest,
        atlas_api::dtos::groups::CreateGroupRequest,
        atlas_api::dtos::groups::GroupDto,
        atlas_api::dtos::groups::GroupMemberDto,
        atlas_api::dtos::ActivatePasswordRequest,
        atlas_api::dtos::ActivationInfoDto,
        atlas_api::dtos::ActivationLinkResponse,
        atlas_api::dtos::AgentIdentityDto,
        atlas_api::dtos::ApiKeyCreated,
        atlas_api::dtos::ApiKeyDto,
        atlas_api::dtos::ApiKeyGrantDto,
        atlas_api::dtos::ApiKeyScope,
        atlas_api::dtos::ChangePasswordRequest,
        atlas_api::dtos::CreateGrantRequest,
        atlas_api::dtos::CreateUserApiKeyRequest,
        atlas_api::dtos::CreateUserRequest,
        atlas_api::dtos::CreateUserResponse,
        atlas_api::dtos::GrantDto,
        atlas_api::dtos::GrantPrincipal,
        atlas_api::dtos::GrantedByDto,
        atlas_api::dtos::InitialGrantRequest,
        atlas_api::dtos::LoginRequest,
        atlas_api::dtos::LoginResponse,
        atlas_api::dtos::MeResponse,
        atlas_api::dtos::ResetPasswordRequest,
        atlas_api::dtos::SetSystemAdminRequest,
        atlas_api::dtos::UpdateApiKeyRequest,
        atlas_api::dtos::UpdateMeRequest,
        atlas_api::dtos::UserDto,
        atlas_api::dtos::UserMembershipDto,
    )),
    tags(
        (name = "audit", description = "Security audit log"),
        (name = "auth", description = "Authentication and session management"),
        (name = "users", description = "User management (root-only)"),
        (name = "api-keys", description = "Workspace API key management"),
        (name = "grants", description = "Permission grant management"),
        (name = "groups", description = "Workspace principal groups"),
    )
)]
struct CustosOpenApi;

/// `custos`'s registry `stable_id` (matches `reg5.rs`'s
/// `component("custos")`), read once here — every operation-tag/extension
/// stamp this module produces goes through
/// [`crate::routes::openapi::stamp_component_ownership`] with this single
/// constant, never a second hand-typed `"custos"` literal (T2.33).
pub(crate) const OPENAPI_STABLE_ID: &str = "custos";

/// Builds `custos`'s OpenAPI fragment, tagged with component ownership
/// (D4). Consumed only by `routes::openapi::document()`'s fixed merge order.
pub(crate) fn openapi() -> utoipa::openapi::OpenApi {
    crate::routes::openapi::stamp_component_ownership(
        <CustosOpenApi as utoipa::OpenApi>::openapi(),
        OPENAPI_STABLE_ID,
    )
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use atlas_core::registry::{ComponentId, HttpMethod, build};

    use super::*;
    use crate::reg5::{StorageBackend, reg5_component_entries};
    use crate::router_audit::{
        DeclaredScope, capability_from_action_id, diff_declared_and_enforced, diff_route_sets,
    };

    fn component(value: &str) -> ComponentId {
        ComponentId::new(value).expect("valid component id")
    }

    /// Builds the same registry the live server assembles at startup. See
    /// `routes::platform`'s test module for why the storage backend choice is
    /// irrelevant to this component's own routes.
    fn build_registry() -> atlas_core::registry::Registry {
        build(reg5_component_entries(StorageBackend::Filesystem))
            .expect("REG-5 entries must satisfy every registry::build() validator")
    }

    /// T3.2/T3.4 (bidirectional audit, D2/INV-SET): `custos::declared_routes()`
    /// must equal the registry's `custos.api.routes` set exactly, in both
    /// directions.
    #[test]
    fn custos_router_and_registry_route_sets_match_exactly() {
        let registry = build_registry();
        let entry = registry
            .get(&component("custos"))
            .expect("custos is a REG-5 component");

        let router_set: std::collections::HashSet<(HttpMethod, String)> = declared_routes()
            .iter()
            .map(|route| (route.method, route.path.to_string()))
            .collect();

        let registry_set: std::collections::HashSet<(HttpMethod, String)> = entry
            .api
            .routes
            .iter()
            .map(|route| (route.method, route.path.as_str().to_string()))
            .collect();

        let diff = diff_route_sets(&router_set, &registry_set);

        assert!(
            diff.is_empty(),
            "custos's router and registry route sets must match exactly: {diff:?}"
        );
        assert_eq!(
            router_set.len(),
            35,
            "custos owns exactly 35 routes per docs/registry-route-ownership.md"
        );
    }

    /// T3.7 (declare-and-verify, D5): exhaustive over every one of custos's
    /// 35 routes, not a curated subset (R4). Two routes
    /// (`list_project_grants`, `list_workspace_grants`) declare
    /// `action: Some(custos::grants::read)` — the first real, non-degenerate
    /// exercise of `capability_from_action_id` (platform declared zero
    /// `Some(_)` actions in PR2).
    #[test]
    fn custos_declared_actions_match_enforced_capabilities() {
        let registry = build_registry();
        let entry = registry
            .get(&component("custos"))
            .expect("custos is a REG-5 component");

        let some_action_paths: Vec<&str> = entry
            .api
            .routes
            .iter()
            .filter(|route| route.action.is_some())
            .map(|route| route.path.as_str())
            .collect();
        assert_eq!(
            some_action_paths.len(),
            2,
            "custos declares exactly two Some(_) actions (both grants::read reads): {some_action_paths:?}"
        );

        let routes = declared_routes();

        let declared_actions: HashMap<(HttpMethod, &'static str), Option<_>> = routes
            .iter()
            .map(|route| {
                let action = entry
                    .api
                    .routes
                    .iter()
                    .find(|declared| {
                        declared.method == route.method && declared.path.as_str() == route.path
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "registry has no declaration for {:?} {}",
                            route.method, route.path
                        )
                    })
                    .action
                    .as_ref()
                    .map(capability_from_action_id);
                ((route.method, route.path), action)
            })
            .collect();

        let mismatches = diff_declared_and_enforced(&routes, &declared_actions);

        assert!(
            mismatches.is_empty(),
            "declared vs enforced must agree for every custos route: {mismatches:?}"
        );

        // Confirm the positive case is real, not a coincidence of both sides
        // being `None`: the two grants-read routes must have actually
        // extracted `Some(_)`, not degenerated to the exemption arm.
        let grants_read_routes: Vec<_> = routes
            .iter()
            .filter(|route| {
                route.path == "/workspaces/{ws}/grants"
                    || route.path == "/workspaces/{ws}/projects/{project_slug}/grants"
            })
            .filter(|route| route.method == HttpMethod::Get)
            .collect();
        assert_eq!(grants_read_routes.len(), 2);
        for route in grants_read_routes {
            assert!(
                matches!(route.scope, DeclaredScope::Extracted(Some(_))),
                "{:?} {} must extract a real Some(_) capability, got {:?}",
                route.method,
                route.path,
                route.scope
            );
        }
    }

    /// D3/T3.8: the four `Module` entries (no HTTP surface) assert
    /// `namespace: None` and `routes: []` explicitly, as their own fact — not
    /// inferred from the per-component `[platform, custos, acta]` loop's
    /// absence of failures, since Modules are never in that loop and could
    /// otherwise be silently skipped and still show green (A3).
    ///
    /// `reg5_component_entries` includes exactly one storage backend per call
    /// (`storage.filesystem` and `storage.s3` both provide `storage.blob` and
    /// cannot coexist in one `build()`, per `reg5.rs`'s own doc comment), so
    /// this test asserts the fact for both storage backends explicitly —
    /// building the registry twice, once per `StorageBackend` variant —
    /// rather than only exercising whichever backend `build_registry()`
    /// happens to default to.
    #[test]
    fn module_entries_declare_zero_routes_as_an_explicit_fact() {
        for backend in [StorageBackend::Filesystem, StorageBackend::S3] {
            let registry = build(reg5_component_entries(backend))
                .expect("REG-5 entries must satisfy every registry::build() validator");

            let storage_module_id = match backend {
                StorageBackend::Filesystem => "storage.filesystem",
                StorageBackend::S3 => "storage.s3",
            };

            for module_id in [
                storage_module_id,
                "search.postgres_fts",
                "search.pgvector_embeddings",
            ] {
                let entry = registry
                    .get(&component(module_id))
                    .unwrap_or_else(|| panic!("{module_id} must be a registered REG-5 entry"));

                assert!(
                    entry.api.namespace.is_none(),
                    "{module_id} must declare no HTTP namespace (Module entries carry none)"
                );
                assert!(
                    entry.api.routes.is_empty(),
                    "{module_id} must declare zero routes (Module entries carry none)"
                );
            }
        }
    }
}
