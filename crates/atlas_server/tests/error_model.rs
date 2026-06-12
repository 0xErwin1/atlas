#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_server::error::ApiError;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    routing::get,
};
use tower::ServiceExt;

fn router_with_handler(handler: axum::routing::MethodRouter) -> Router {
    atlas_server::test_app_with_route("/test", handler)
}

#[tokio::test]
async fn unauthorized_error_produces_problem_json_with_401() {
    let app = router_with_handler(get(|| async {
        Err::<(), ApiError>(ApiError::Unauthorized)
    }));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap();
    assert!(
        content_type.contains("application/problem+json"),
        "content-type must be application/problem+json, got: {content_type}"
    );

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body["type"], "urn:atlas:error:unauthorized");
    assert_eq!(body["status"], 401);
    assert!(body["title"].is_string(), "title must be present");
}

#[tokio::test]
async fn invalid_input_error_produces_422_problem_json() {
    let app = router_with_handler(get(|| async {
        Err::<(), ApiError>(ApiError::InvalidInput {
            message: "bad field value".into(),
        })
    }));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(body["type"], "urn:atlas:error:invalid-input");
    assert_eq!(body["status"], 422);
    assert_eq!(body["detail"], "bad field value");
}

#[tokio::test]
async fn problem_stamp_fills_request_id_from_header() {
    let app = router_with_handler(get(|| async {
        Err::<(), ApiError>(ApiError::Unauthorized)
    }));

    let response = app
        .oneshot(
            Request::builder()
                .uri("/test")
                .header("x-request-id", "test-123")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body["request_id"], "test-123",
        "request_id must equal the supplied x-request-id header"
    );
}

#[tokio::test]
async fn problem_stamp_fills_instance_with_request_path() {
    let app = router_with_handler(get(|| async {
        Err::<(), ApiError>(ApiError::Unauthorized)
    }));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    let body_bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    assert_eq!(
        body["instance"], "/test",
        "instance must equal the request path"
    );
}
