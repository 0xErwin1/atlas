//! Shared `sea_orm::DbErr` to `atlas_core::error::DomainError` mapping.
//!
//! This is the uniform fallback: any `DbErr` that a component's repository
//! does not classify more specifically (for example a unique-constraint
//! violation mapped to a 409) should be mapped through [`db_err`] instead of
//! duplicating the `Internal { message: e.to_string() }` line locally.
//! Component-specific classifications (status-code branching on `DbErr::sql_err`,
//! and similar) stay component-side and call this helper only for their
//! fallback arm.

use atlas_core::error::DomainError;

/// Maps a `sea_orm::DbErr` to `DomainError::Internal`, carrying the error's
/// `Display` output as the message.
pub fn db_err(e: sea_orm::DbErr) -> DomainError {
    DomainError::Internal {
        message: e.to_string(),
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::db_err;
    use atlas_core::error::DomainError;
    use sea_orm::DbErr;

    #[test]
    fn maps_custom_db_err_to_internal_with_display_message() {
        let e = DbErr::Custom("boom".to_string());
        let expected_message = e.to_string();

        let DomainError::Internal { message } = db_err(e) else {
            panic!("expected DomainError::Internal");
        };
        assert_eq!(message, expected_message);
    }

    #[test]
    fn maps_record_not_found_db_err_to_internal_with_display_message() {
        let e = DbErr::RecordNotFound("row missing".to_string());
        let expected_message = e.to_string();

        let DomainError::Internal { message } = db_err(e) else {
            panic!("expected DomainError::Internal");
        };
        assert_eq!(message, expected_message);
    }
}
