use crate::error::ApiError;

const MAX_NAME_LEN: usize = 200;
const MAX_DESC_LEN: usize = 20_000;
const MAX_LABEL_LEN: usize = 64;
const MAX_LABELS: usize = 50;
const MAX_CUSTOM_ENTRIES: usize = 100;

/// Validates a short name/title field.
///
/// Trims the value; rejects blank strings and strings longer than 200 characters.
pub(crate) fn validate_name(field: &str, value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must not be blank"),
        });
    }

    if trimmed.len() > MAX_NAME_LEN {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must be at most {MAX_NAME_LEN} characters"),
        });
    }

    Ok(())
}

/// Validates a long free-text field (e.g. description) against a maximum byte length.
pub(crate) fn validate_long_text(field: &str, value: &str, max: usize) -> Result<(), ApiError> {
    if value.len() > max {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must be at most {max} characters"),
        });
    }

    Ok(())
}

/// Validates a labels array.
///
/// Rejects arrays with more than 50 entries, empty/whitespace-only labels, or
/// any individual label exceeding 64 characters.
pub(crate) fn validate_labels(labels: &[String]) -> Result<(), ApiError> {
    if labels.len() > MAX_LABELS {
        return Err(ApiError::InvalidInput {
            message: format!("labels must not exceed {MAX_LABELS} entries"),
        });
    }

    for label in labels {
        let trimmed = label.trim();
        if trimmed.is_empty() {
            return Err(ApiError::InvalidInput {
                message: "each label must not be blank".into(),
            });
        }
        if trimmed.len() > MAX_LABEL_LEN {
            return Err(ApiError::InvalidInput {
                message: format!("each label must be at most {MAX_LABEL_LEN} characters"),
            });
        }
    }

    Ok(())
}

/// Validates the entry count of a JSON object used as a custom properties map.
///
/// Rejects objects with more than 100 entries. Non-object values pass through
/// without validation (type errors are handled by domain/DB layer).
pub(crate) fn validate_custom_entry_count(value: &serde_json::Value) -> Result<(), ApiError> {
    if let serde_json::Value::Object(map) = value
        && map.len() > MAX_CUSTOM_ENTRIES
    {
        return Err(ApiError::InvalidInput {
            message: format!("custom properties must not exceed {MAX_CUSTOM_ENTRIES} entries"),
        });
    }

    Ok(())
}

/// Validates the description field using the standard 20 000-character cap.
pub(crate) fn validate_description(value: &str) -> Result<(), ApiError> {
    validate_long_text("description", value, MAX_DESC_LEN)
}
