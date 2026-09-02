//! The one protected layer stack every component router and the root
//! fallback share. Defined once so the three component routers and
//! `lib.rs::app()` cannot drift from each other by a layer or an order.

use axum::Router;

use crate::state::AppState;

/// Wraps `router` in the protected middleware stack, applied in exactly this
/// order (outermost first at request time):
///
/// 1. `require_authn` — rejects the request with 401 before anything else.
/// 2. `require_rate_limit` — per-principal rate limiting.
/// 3. `require_csrf_for_cookie_mutations` — CSRF check for cookie-backed
///    mutating requests.
///
/// The `.layer(...)` calls below are listed innermost first, which is why
/// the source order is the reverse of the request-time order above.
pub(crate) fn protect(router: Router, state: AppState) -> Router {
    router
        .layer(axum::middleware::from_fn(
            crate::auth::csrf::require_csrf_for_cookie_mutations,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::middleware::rate_limit::require_rate_limit,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state,
            crate::auth::middleware::require_authn,
        ))
}
