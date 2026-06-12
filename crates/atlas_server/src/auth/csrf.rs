use axum::{extract::Request, middleware::Next, response::Response};
use axum_extra::extract::CookieJar;

use crate::error::ApiError;

/// Enforces CSRF protection for cookie-authenticated state-changing requests.
///
/// Passes through when:
/// - The request carries a `Authorization: Bearer` header (API-key or bearer-token path,
///   which is inherently CSRF-safe).
/// - The method is safe (GET, HEAD, OPTIONS, TRACE).
///
/// Requires `X-Atlas-CSRF: 1` on every other request that arrives via a session cookie.
pub async fn require_csrf_for_cookie_mutations(
    jar: CookieJar,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let method = request.method().clone();

    let is_safe_method = matches!(method.as_str(), "GET" | "HEAD" | "OPTIONS" | "TRACE");

    if is_safe_method {
        return Ok(next.run(request).await);
    }

    let has_bearer = request
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.starts_with("Bearer "))
        .unwrap_or(false);

    if has_bearer {
        return Ok(next.run(request).await);
    }

    let has_session_cookie = jar.get("atlas_session").is_some();

    if has_session_cookie {
        let has_csrf_header = request
            .headers()
            .get("x-atlas-csrf")
            .and_then(|v| v.to_str().ok())
            .map(|v| v == "1")
            .unwrap_or(false);

        if !has_csrf_header {
            return Err(ApiError::CsrfRequired);
        }
    }

    Ok(next.run(request).await)
}
