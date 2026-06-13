use atlas_api::problem::ProblemDetails;
use axum::{
    body::Body,
    extract::Request,
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use tower_http::request_id::RequestId;

/// Middleware that fills `request_id` and `instance` on problem+json error responses.
///
/// On responses with status >= 400 and `Content-Type: application/problem+json`,
/// this middleware deserializes the (small) body, sets `request_id` from the
/// `x-request-id` header extension and `instance` from the request path, then
/// re-serializes. Cost is bounded to error paths only.
pub async fn problem_stamp(request: Request, next: Next) -> Response {
    let path = request.uri().path().to_string();
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .and_then(|id| id.header_value().to_str().ok().map(str::to_owned));

    let response = next.run(request).await;

    let status = response.status();
    let is_problem_json = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("application/problem+json"));

    if status.as_u16() < 400 || !is_problem_json {
        return response;
    }

    let (parts, body) = response.into_parts();
    let bytes = match axum::body::to_bytes(body, 64 * 1024).await {
        Ok(b) => b,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };

    // Stamp using a generic JSON object so extended problem bodies (e.g. ConflictProblemDto)
    // keep their extra fields while still receiving request_id and instance.
    let mut map: serde_json::Map<String, serde_json::Value> = match serde_json::from_slice(&bytes) {
        Ok(serde_json::Value::Object(m)) => m,
        _ => return Response::from_parts(parts, Body::from(bytes)),
    };

    if !map.contains_key("request_id")
        && let Some(rid) = request_id
    {
        map.insert("request_id".into(), serde_json::Value::String(rid));
    }
    if !map.contains_key("instance") {
        map.insert("instance".into(), serde_json::Value::String(path));
    }

    let stamped = serde_json::to_vec(&map).unwrap_or_else(|_| bytes.to_vec());
    let mut response = Response::from_parts(parts, Body::from(stamped));
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// Fallback handler for 404 routes (no route matched).
pub async fn not_found_handler() -> Response {
    let problem = ProblemDetails::new("urn:atlas:error:not-found", "Not Found", 404)
        .with_hint("Check the identifier — it may not exist or you may not have access.");
    let body = serde_json::to_vec(&problem).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = Response::new(Body::from(body));
    *response.status_mut() = StatusCode::NOT_FOUND;
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}
