use atlas_api::{dtos::documents::ConflictProblemDto, problem::ProblemDetails};
use atlas_domain::error::{DomainError, RevisionConflict};
use axum::{
    http::{HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};

/// Server-side error taxonomy.
///
/// Every variant maps to a specific RFC 9457 problem type and HTTP status.
/// Handlers build `ApiError` values; the `IntoResponse` impl serializes them.
/// The problem-stamp middleware fills `request_id` and `instance` after the fact.
#[derive(Debug)]
pub enum ApiError {
    Domain(DomainError),
    Unauthorized,
    CsrfRequired,
    InvalidInput {
        message: String,
    },
    NotFound,
    Forbidden {
        message: String,
    },
    /// Generic conflict (no payload). Prefer `RevisionConflict` for CAS failures.
    Conflict,
    /// CAS revision conflict with full patch payload for the 409 response body.
    RevisionConflict(RevisionConflict),
    PayloadTooLarge {
        message: String,
    },
    Internal {
        message: String,
    },
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, problem) = match self {
            ApiError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                ProblemDetails::new(
                    "urn:atlas:error:unauthorized",
                    "Unauthorized",
                    401,
                )
                .with_hint("Provide a valid Bearer token or session cookie. Login at POST /v1/auth/login."),
            ),
            ApiError::CsrfRequired => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new(
                    "urn:atlas:error:csrf-required",
                    "CSRF Protection Required",
                    403,
                )
                .with_hint("Include the 'X-Atlas-CSRF: 1' header on cookie-authenticated state-changing requests."),
            ),
            ApiError::InvalidInput { message } => (
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemDetails::new("urn:atlas:error:invalid-input", "Invalid Input", 422)
                    .with_detail(message),
            ),
            ApiError::NotFound => (
                StatusCode::NOT_FOUND,
                ProblemDetails::new("urn:atlas:error:not-found", "Not Found", 404).with_hint(
                    "Check the identifier — it may not exist or you may not have access.",
                ),
            ),
            ApiError::Forbidden { message } => (
                StatusCode::FORBIDDEN,
                ProblemDetails::new("urn:atlas:error:forbidden", "Forbidden", 403)
                    .with_detail(message),
            ),
            ApiError::Conflict => (
                StatusCode::CONFLICT,
                ProblemDetails::new(
                    "urn:atlas:error:revision-conflict",
                    "Revision Conflict",
                    409,
                ),
            ),
            ApiError::PayloadTooLarge { message } => (
                StatusCode::PAYLOAD_TOO_LARGE,
                ProblemDetails::new(
                    "urn:atlas:error:payload-too-large",
                    "Payload Too Large",
                    413,
                )
                .with_detail(message),
            ),
            ApiError::RevisionConflict(c) => {
                let body = ConflictProblemDto::new(
                    c.current_revision_id.0,
                    c.current_seq,
                    c.base_to_current_patch,
                );
                let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
                let mut response = (StatusCode::CONFLICT, bytes).into_response();
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/problem+json"),
                );
                return response;
            }
            ApiError::Internal { message } => {
                tracing::error!(error = %message, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    ProblemDetails::new("urn:atlas:error:internal", "Internal Server Error", 500)
                        .with_detail("An internal error occurred."),
                )
            }
            ApiError::Domain(domain_err) => return domain_error_response(domain_err),
        };

        build_problem_response(status, problem)
    }
}

fn domain_error_response(err: DomainError) -> Response {
    let (status, problem) = match err {
        DomainError::NotFound { entity, id } => (
            StatusCode::NOT_FOUND,
            ProblemDetails::new("urn:atlas:error:not-found", "Not Found", 404)
                .with_hint("Check the identifier — it may not exist or you may not have access.")
                .with_detail(format!("{entity} {id} not found")),
        ),
        DomainError::Conflict(c) => {
            let body = ConflictProblemDto::new(
                c.current_revision_id.0,
                c.current_seq,
                c.base_to_current_patch,
            );
            let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| b"{}".to_vec());
            let mut response = (StatusCode::CONFLICT, bytes).into_response();
            response.headers_mut().insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/problem+json"),
            );
            return response;
        }
        DomainError::InvalidInput { message } => (
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemDetails::new("urn:atlas:error:invalid-input", "Invalid Input", 422)
                .with_detail(message),
        ),
        DomainError::Forbidden { message } => (
            StatusCode::FORBIDDEN,
            ProblemDetails::new("urn:atlas:error:forbidden", "Forbidden", 403).with_detail(message),
        ),
        DomainError::Internal { message } => {
            tracing::error!(error = %message, "domain internal error");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                ProblemDetails::new("urn:atlas:error:internal", "Internal Server Error", 500)
                    .with_detail("An internal error occurred."),
            )
        }
        DomainError::PositionExhausted { .. } => (
            StatusCode::CONFLICT,
            ProblemDetails::new(
                "urn:atlas:error:position-exhausted",
                "Position Exhausted",
                409,
            )
            .with_hint("Retry the move; the server attempted to rebalance column positions."),
        ),
    };

    build_problem_response(status, problem)
}

fn build_problem_response(status: StatusCode, problem: ProblemDetails) -> Response {
    let body = serde_json::to_vec(&problem).unwrap_or_else(|_| b"{}".to_vec());
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}
