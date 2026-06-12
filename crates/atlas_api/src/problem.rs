use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// RFC 9457 problem+json representation, extended with `request_id` and `hint`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct ProblemDetails {
    /// Stable error type URI in the form `urn:atlas:error:{slug}`.
    pub r#type: String,

    /// Human-readable summary of the error class.
    pub title: String,

    /// HTTP status code.
    pub status: u16,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,

    /// Request path — filled by the problem-stamp middleware.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance: Option<String>,

    /// Filled by the problem-stamp middleware from the `x-request-id` header.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,

    /// Actionable next-step hint for the caller.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

impl ProblemDetails {
    pub fn new(r#type: impl Into<String>, title: impl Into<String>, status: u16) -> Self {
        Self {
            r#type: r#type.into(),
            title: title.into(),
            status,
            detail: None,
            instance: None,
            request_id: None,
            hint: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }
}
