use crate::ids::{DocumentId, RevisionId};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct RevisionConflict {
    pub document_id: DocumentId,
    pub current_revision_id: RevisionId,
    pub current_seq: i64,
    pub base_to_current_patch: String,
}

#[derive(Debug, Error)]
pub enum DomainError {
    #[error("entity not found: {entity} {id}")]
    NotFound { entity: &'static str, id: Uuid },

    #[error("conflict: stale revision")]
    Conflict(RevisionConflict),

    #[error("invalid input: {message}")]
    InvalidInput { message: String },

    #[error("internal error: {message}")]
    Internal { message: String },

    #[error("forbidden: {message}")]
    Forbidden { message: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_error_display_not_empty() {
        let err = DomainError::NotFound {
            entity: "document",
            id: Uuid::now_v7(),
        };
        assert!(!err.to_string().is_empty());
    }
}
