use thiserror::Error;
use uuid::Uuid;

/// Compare-and-swap conflict payload for a versioned resource.
///
/// Neutral by construction: the HTTP adapter's 409 body consumes only the
/// four fields below, none of which names a component type.
#[derive(Debug, Clone)]
pub struct RevisionConflict {
    pub resource_id: Uuid,
    pub current_revision_id: Uuid,
    pub current_seq: i64,
    pub base_to_current_patch: String,
}

/// The outcome vocabulary of domain operations across the platform.
///
/// Every variant maps to a specific RFC 9457 problem type in
/// `atlas_server::error::domain_error_response`. The two variants that once
/// carried component-specific payloads (`Conflict`, formerly
/// `PositionExhausted`) have been neutralized so this enum names no
/// component type; see `ComponentConflict` for the escape hatch that keeps
/// component-specific conflicts off this enum.
#[derive(Debug, Error)]
pub enum DomainError {
    #[error("entity not found: {entity} {id}")]
    NotFound { entity: &'static str, id: Uuid },

    #[error("conflict: stale revision")]
    Conflict(RevisionConflict),

    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    #[error("already exists: {message}")]
    AlreadyExists { message: String },

    #[error("restore is blocked by a deleted parent")]
    RestoreParentDeleted { kind: &'static str },

    #[error("restore is blocked by a live conflicting identity")]
    RestoreIdentityConflict { kind: &'static str },

    #[error("internal error: {message}")]
    Internal { message: String },

    #[error("forbidden: {message}")]
    Forbidden { message: String },

    #[error("comment draft conflict: {reason}")]
    CommentDraftConflict { reason: String },

    #[error("comment draft is gone: {reason}")]
    CommentDraftGone { reason: String },

    /// A component-scoped conflict identified by a stable machine code.
    ///
    /// Core neither defines nor interprets the codes; the owning component
    /// declares them as consts and the HTTP adapter maps them. This is the
    /// escape hatch that keeps component-specific 409s off the core enum
    /// without forcing a component error type through every signature.
    #[error("conflict [{code}]")]
    ComponentConflict {
        code: &'static str,
        message: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the exact `Display` string for every variant. These strings are
    /// never wire-visible (the HTTP adapter builds problem bodies with its
    /// own `format!`/literals), but they do flow into `tracing::error!` log
    /// lines, so they must stay stable.
    #[test]
    fn every_variant_display_string_is_byte_identical() {
        let id = Uuid::now_v7();
        let cases: Vec<(DomainError, String)> = vec![
            (
                DomainError::NotFound {
                    entity: "document",
                    id,
                },
                format!("entity not found: document {id}"),
            ),
            (
                DomainError::Conflict(RevisionConflict {
                    resource_id: Uuid::now_v7(),
                    current_revision_id: Uuid::now_v7(),
                    current_seq: 1,
                    base_to_current_patch: String::new(),
                }),
                "conflict: stale revision".into(),
            ),
            (
                DomainError::InvalidInput {
                    message: "bad field".into(),
                },
                "invalid input: bad field".into(),
            ),
            (
                DomainError::AlreadyExists {
                    message: "duplicate slug".into(),
                },
                "already exists: duplicate slug".into(),
            ),
            (
                DomainError::RestoreParentDeleted { kind: "folder" },
                "restore is blocked by a deleted parent".into(),
            ),
            (
                DomainError::RestoreIdentityConflict { kind: "document" },
                "restore is blocked by a live conflicting identity".into(),
            ),
            (
                DomainError::Internal {
                    message: "db exploded".into(),
                },
                "internal error: db exploded".into(),
            ),
            (
                DomainError::Forbidden {
                    message: "no permission".into(),
                },
                "forbidden: no permission".into(),
            ),
            (
                DomainError::CommentDraftConflict {
                    reason: "draft open".into(),
                },
                "comment draft conflict: draft open".into(),
            ),
            (
                DomainError::CommentDraftGone {
                    reason: "discarded".into(),
                },
                "comment draft is gone: discarded".into(),
            ),
            (
                DomainError::ComponentConflict {
                    code: "position-exhausted",
                    message: None,
                },
                "conflict [position-exhausted]".into(),
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected, "display mismatch for {err:?}");
        }
    }
}
