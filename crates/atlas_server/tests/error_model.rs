#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use atlas_core::error::DomainError;
use atlas_core::error::RevisionConflict;
use atlas_server::error::ApiError;
use atlas_server::error::acta_conflict;
use axum::{
    Router,
    body::Body,
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use tower::ServiceExt;
use uuid::Uuid;

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
async fn last_owner_error_produces_409_with_own_urn() {
    let app = router_with_handler(get(|| async {
        Err::<(), ApiError>(ApiError::LastOwner {
            message: "A workspace must keep at least one owner".into(),
        })
    }));

    let response = app
        .oneshot(Request::builder().uri("/test").body(Body::empty()).unwrap())
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CONFLICT);

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

    assert_eq!(
        body["type"], "urn:atlas:error:last-owner",
        "type must be the last-owner urn, not the generic revision-conflict urn"
    );
    assert_eq!(body["status"], 409);
    assert!(
        body["hint"].is_string(),
        "hint must be present to guide the caller"
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

/// Drives a `Response` to completion and returns its status, parsed JSON
/// body, and `content-type` header value.
async fn response_parts(response: Response) -> (StatusCode, serde_json::Value, String) {
    let status = response.status();
    let content_type = response
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let body_bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    (status, body, content_type)
}

/// Characterizes the full RFC 9457 problem body produced for every
/// `DomainError` variant that maps through a plain `ProblemDetails`. This is
/// the safety net for the S2b `atlas_core` split: it must stay green,
/// byte-for-byte, across the move.
#[tokio::test]
async fn every_plain_domain_error_variant_produces_its_documented_problem_body() {
    let entity_id = Uuid::now_v7();
    let cases: Vec<(&str, DomainError, StatusCode, serde_json::Value)> = vec![
        (
            "not_found",
            DomainError::NotFound {
                entity: "document",
                id: entity_id,
            },
            StatusCode::NOT_FOUND,
            serde_json::json!({
                "type": "urn:atlas:error:not-found",
                "title": "Not Found",
                "status": 404,
                "detail": format!("document {entity_id} not found"),
                "hint": "Check the identifier — it may not exist or you may not have access.",
            }),
        ),
        (
            "invalid_input",
            DomainError::InvalidInput {
                message: "bad field value".into(),
            },
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!({
                "type": "urn:atlas:error:invalid-input",
                "title": "Invalid Input",
                "status": 422,
                "detail": "bad field value",
            }),
        ),
        (
            "already_exists",
            DomainError::AlreadyExists {
                message: "a document with this slug already exists".into(),
            },
            StatusCode::CONFLICT,
            serde_json::json!({
                "type": "urn:atlas:error:already-exists",
                "title": "Already Exists",
                "status": 409,
                "detail": "a document with this slug already exists",
                "hint": "An item with this name already exists here — choose a different name.",
            }),
        ),
        (
            "restore_parent_deleted",
            DomainError::RestoreParentDeleted { kind: "folder" },
            StatusCode::CONFLICT,
            serde_json::json!({
                "type": "urn:atlas:error:restore-parent-deleted",
                "title": "Restore Blocked",
                "status": 409,
                "detail": "restore is blocked because the folder's parent is deleted",
                "hint": "Restore the deleted parent before restoring this item.",
            }),
        ),
        (
            "restore_identity_conflict",
            DomainError::RestoreIdentityConflict { kind: "document" },
            StatusCode::CONFLICT,
            serde_json::json!({
                "type": "urn:atlas:error:restore-identity-conflict",
                "title": "Restore Blocked",
                "status": 409,
                "detail": "restore is blocked because a live document has the same identity",
                "hint": "Resolve the live conflicting identity before restoring this item.",
            }),
        ),
        (
            "internal",
            DomainError::Internal {
                message: "db exploded".into(),
            },
            StatusCode::INTERNAL_SERVER_ERROR,
            serde_json::json!({
                "type": "urn:atlas:error:internal",
                "title": "Internal Server Error",
                "status": 500,
                "detail": "An internal error occurred.",
            }),
        ),
        (
            "forbidden",
            DomainError::Forbidden {
                message: "caller lacks permission".into(),
            },
            StatusCode::FORBIDDEN,
            serde_json::json!({
                "type": "urn:atlas:error:forbidden",
                "title": "Forbidden",
                "status": 403,
                "detail": "caller lacks permission",
            }),
        ),
        (
            "comment_draft_conflict",
            DomainError::CommentDraftConflict {
                reason: "another draft is already open".into(),
            },
            StatusCode::CONFLICT,
            serde_json::json!({
                "type": "urn:atlas:error:comment-draft-conflict",
                "title": "Comment Draft Conflict",
                "status": 409,
                "detail": "another draft is already open",
            }),
        ),
        (
            "comment_draft_gone",
            DomainError::CommentDraftGone {
                reason: "the draft was discarded".into(),
            },
            StatusCode::GONE,
            serde_json::json!({
                "type": "urn:atlas:error:comment-draft-gone",
                "title": "Comment Draft Gone",
                "status": 410,
                "detail": "the draft was discarded",
            }),
        ),
    ];

    for (name, err, expected_status, expected_body) in cases {
        let response = ApiError::Domain(err).into_response();
        let (status, body, content_type) = response_parts(response).await;

        assert_eq!(status, expected_status, "case {name}: status mismatch");
        assert_eq!(body, expected_body, "case {name}: full body mismatch");
        assert!(
            content_type.contains("application/problem+json"),
            "case {name}: content-type must be application/problem+json, got: {content_type}"
        );
    }
}

/// Revision conflict still reaches the wire unchanged: full
/// `ConflictProblemDto` body, all four caller-visible fields, byte-identical
/// across the S2b split.
#[tokio::test]
async fn revision_conflict_produces_full_conflict_problem_dto_body() {
    let current_revision_id = Uuid::now_v7();
    let err = DomainError::Conflict(RevisionConflict {
        resource_id: Uuid::now_v7(),
        current_revision_id,
        current_seq: 7,
        base_to_current_patch: "@@ -1 +1 @@\n-old\n+new\n".into(),
    });

    let response = ApiError::Domain(err).into_response();
    let (status, body, content_type) = response_parts(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        content_type.contains("application/problem+json"),
        "content-type must be application/problem+json, got: {content_type}"
    );
    assert_eq!(
        body,
        serde_json::json!({
            "type": "urn:atlas:error:revision-conflict",
            "title": "Revision Conflict",
            "status": 409,
            "detail": "The base_revision_id does not match the current revision. \
                 Apply base_to_current_patch and retry.",
            "hint": "Apply the provided patch to your local content, then retry with the new revision id.",
            "current_revision_id": current_revision_id,
            "current_seq": 7,
            "base_to_current_patch": "@@ -1 +1 @@\n-old\n+new\n",
        }),
        "ConflictProblemDto body must be byte-identical across the split"
    );
}

/// Position-exhausted still reaches the wire unchanged, and critically has no
/// `detail` key: `ComponentConflict { message: None }` must not introduce
/// one (design R-S2b-2).
#[tokio::test]
async fn position_exhausted_produces_full_body_with_no_detail_key() {
    let err = DomainError::ComponentConflict {
        code: acta_conflict::POSITION_EXHAUSTED,
        message: None,
    };

    let response = ApiError::Domain(err).into_response();
    let (status, body, content_type) = response_parts(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        content_type.contains("application/problem+json"),
        "content-type must be application/problem+json, got: {content_type}"
    );
    assert_eq!(
        body,
        serde_json::json!({
            "type": "urn:atlas:error:position-exhausted",
            "title": "Position Exhausted",
            "status": 409,
            "hint": "Retry the move; the server attempted to rebalance column positions.",
        }),
        "position-exhausted body must be byte-identical across the split"
    );
    assert!(
        body.get("detail").is_none(),
        "position-exhausted must never gain a detail key: got {body:?}"
    );
}

/// Exhaustiveness gate for the `ComponentConflict` code-mapping (design
/// R-S2b-1): every declared component-conflict code must map to its own
/// response, not fall through to the unmapped-code default arm. Adding a new
/// code const without a matching mapping arm must fail this test.
#[tokio::test]
async fn every_declared_component_conflict_code_is_mapped() {
    const ALL: &[&str] = &[acta_conflict::POSITION_EXHAUSTED];

    for code in ALL {
        let err = DomainError::ComponentConflict {
            code,
            message: None,
        };

        let response = ApiError::Domain(err).into_response();
        let (status, body, _content_type) = response_parts(response).await;

        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "code {code}: expected 409, the default fallback also returns 409 \
             but with a different body — checked below"
        );
        assert_ne!(
            body["type"], "urn:atlas:error:conflict",
            "code {code} fell through to the unmapped-code default body"
        );
    }
}

/// An unrecognized component-conflict code must fall back to a generic
/// conflict body, never silently reuse a mapped one.
#[tokio::test]
async fn unmapped_component_conflict_code_falls_back_to_generic_conflict() {
    let err = DomainError::ComponentConflict {
        code: "not-a-real-code",
        message: Some("boom".into()),
    };

    let response = ApiError::Domain(err).into_response();
    let (status, body, _content_type) = response_parts(response).await;

    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["type"], "urn:atlas:error:conflict");
    assert_eq!(body["detail"], "boom");
}
