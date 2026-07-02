//! Per-principal rate limiting for the authenticated API surface.
//!
//! The limiter keys by the authenticated caller (user id or API key id), not by
//! client IP. The abuse vector this guards against is programmatic clients — the
//! MCP server and CLI — driving high request volume, and those are always
//! authenticated, so the principal is the stable, proxy-independent identity to
//! throttle. IP-based limiting still protects the unauthenticated login and
//! activation routes (see `lib.rs`).
//!
//! The middleware runs immediately after `require_authn`, so the `Principal` is
//! already in the request extensions when this layer executes.

use axum::{extract::State, middleware::Next, response::Response};
use governor::{
    Quota, RateLimiter,
    clock::{Clock, DefaultClock},
    state::keyed::DefaultKeyedStateStore,
};
use std::num::NonZeroU32;

use crate::{auth::middleware::Principal, error::ApiError, state::AppState};

/// The keyed rate-limit identity derived from an authenticated `Principal`.
///
/// A user and an API key that happen to share a UUID (which cannot occur in
/// practice) still get independent buckets because the variant is part of the
/// key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RateLimitKey {
    User(uuid::Uuid),
    ApiKey(uuid::Uuid),
}

impl From<&Principal> for RateLimitKey {
    fn from(principal: &Principal) -> Self {
        match principal {
            Principal::User(id) => RateLimitKey::User(id.0),
            Principal::ApiKey(id) => RateLimitKey::ApiKey(id.0),
        }
    }
}

/// An in-memory GCRA rate limiter keyed by authenticated principal.
///
/// Backed by `governor`'s keyed limiter (a sharded in-memory map), so no external
/// store is required. A distributed deployment with multiple replicas would need
/// a shared backend; that is a deliberate future iteration, not required for the
/// current single-instance topology.
pub struct PrincipalRateLimiter {
    limiter: RateLimiter<RateLimitKey, DefaultKeyedStateStore<RateLimitKey>, DefaultClock>,
    clock: DefaultClock,
}

impl PrincipalRateLimiter {
    /// Builds a limiter allowing `burst` requests instantaneously, refilling at
    /// `per_second` requests per second. Both parameters are clamped to a floor
    /// of 1 so a misconfigured `0` cannot construct an unsatisfiable quota.
    pub fn new(per_second: u32, burst: u32) -> Self {
        let rate = NonZeroU32::new(per_second.max(1)).unwrap_or(NonZeroU32::MIN);
        let burst = NonZeroU32::new(burst.max(1)).unwrap_or(NonZeroU32::MIN);

        let quota = Quota::per_second(rate).allow_burst(burst);

        Self {
            limiter: RateLimiter::keyed(quota),
            clock: DefaultClock::default(),
        }
    }

    /// Checks and consumes one cell for `key`.
    ///
    /// Returns `Ok(())` when the request is permitted, or `Err(retry_after_secs)`
    /// with the number of whole seconds (minimum 1) the caller should wait before
    /// retrying when the quota is exhausted.
    pub fn check(&self, key: &RateLimitKey) -> Result<(), u64> {
        match self.limiter.check_key(key) {
            Ok(()) => Ok(()),
            Err(not_until) => {
                let wait = not_until.wait_time_from(self.clock.now());
                Err(wait.as_secs().max(1))
            }
        }
    }
}

/// Middleware that enforces the per-principal rate limit on protected routes.
///
/// A no-op when the limiter is disabled (`AppState::rate_limiter` is `None`) or
/// when no `Principal` is present (which should not happen after `require_authn`,
/// but is treated as pass-through rather than a hard failure).
pub async fn require_rate_limit(
    State(state): State<AppState>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, ApiError> {
    let Some(limiter) = state.rate_limiter.as_ref() else {
        return Ok(next.run(request).await);
    };

    let Some(principal) = request.extensions().get::<Principal>() else {
        return Ok(next.run(request).await);
    };

    let key = RateLimitKey::from(principal);

    match limiter.check(&key) {
        Ok(()) => Ok(next.run(request).await),
        Err(retry_after_secs) => Err(ApiError::TooManyRequests { retry_after_secs }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_key() -> RateLimitKey {
        RateLimitKey::User(uuid::Uuid::now_v7())
    }

    #[test]
    fn allows_up_to_burst_then_rejects() {
        let limiter = PrincipalRateLimiter::new(1, 2);
        let key = user_key();

        assert!(limiter.check(&key).is_ok(), "first request within burst");
        assert!(limiter.check(&key).is_ok(), "second request within burst");

        let third = limiter.check(&key);
        assert!(third.is_err(), "third request must exceed burst of 2");
        assert!(
            third.unwrap_err() >= 1,
            "retry-after must be at least one second"
        );
    }

    #[test]
    fn buckets_are_independent_per_key() {
        let limiter = PrincipalRateLimiter::new(1, 1);
        let a = user_key();
        let b = user_key();

        assert!(limiter.check(&a).is_ok());
        assert!(limiter.check(&a).is_err(), "key a exhausted its burst");
        assert!(
            limiter.check(&b).is_ok(),
            "key b has its own independent bucket"
        );
    }

    #[test]
    fn user_and_api_key_variants_do_not_collide() {
        let id = uuid::Uuid::now_v7();
        let limiter = PrincipalRateLimiter::new(1, 1);

        assert!(limiter.check(&RateLimitKey::User(id)).is_ok());
        assert!(
            limiter.check(&RateLimitKey::ApiKey(id)).is_ok(),
            "same UUID under a different principal variant is a distinct bucket"
        );
    }
}
