use crate::error::ApiError;
use atlas_api::dtos::task_views::TaskViewFiltersDto;

const MAX_NAME_LEN: usize = 200;
const MAX_DESC_LEN: usize = 20_000;
const MAX_LABEL_LEN: usize = 64;
const MAX_LABELS: usize = 50;
const MAX_CUSTOM_ENTRIES: usize = 100;
pub(crate) const MAX_QUERY_LEN: usize = 2_000;

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

/// Validates the `query` field for a saved search.
///
/// Empty queries are allowed (they represent "search everything"). Only the
/// 2 000-character upper bound is enforced.
pub(crate) fn validate_query(value: &str) -> Result<(), ApiError> {
    validate_long_text("query", value, MAX_QUERY_LEN)
}

/// Valid swatch IDs, mirroring `apps/web/src/lib/swatches.ts`.
const VALID_SWATCH_IDS: &[&str] = &[
    "neutral", "blue", "green", "amber", "red", "magenta", "cyan",
];

/// Validates a color value against the known swatch-id set.
///
/// Rejects unknown ids so a bad client cannot store arbitrary strings in the DB.
/// `None` is accepted (no color = use the default).
pub(crate) fn validate_swatch(field: &str, id: &str) -> Result<(), ApiError> {
    if VALID_SWATCH_IDS.contains(&id) {
        return Ok(());
    }

    Err(ApiError::InvalidInput {
        message: format!("{field} must be one of: {}", VALID_SWATCH_IDS.join(", ")),
    })
}

const VALID_SORT_KEYS: &[&str] = &[
    "updated_at_desc",
    "updated_at_asc",
    "created_at_desc",
    "created_at_asc",
    "priority_desc",
    "title_asc",
];

const VALID_PRIORITIES: &[&str] = &["low", "medium", "high", "urgent"];
const MAX_FILTER_COLLECTION_SIZE: usize = 50;

/// Validates the filters object for a task view.
///
/// Enforces: known sort key, collection sizes ≤ 50 (column_ids, labels, priorities),
/// and that all priority strings are from the known set.
pub(crate) fn validate_task_view_filters(filters: &TaskViewFiltersDto) -> Result<(), ApiError> {
    if let Some(sort) = &filters.sort
        && !VALID_SORT_KEYS.contains(&sort.as_str())
    {
        return Err(ApiError::InvalidInput {
            message: format!("sort must be one of: {}", VALID_SORT_KEYS.join(", ")),
        });
    }

    if filters.priorities.len() > MAX_FILTER_COLLECTION_SIZE {
        return Err(ApiError::InvalidInput {
            message: format!("priorities must not exceed {MAX_FILTER_COLLECTION_SIZE} entries"),
        });
    }

    for p in &filters.priorities {
        if !VALID_PRIORITIES.contains(&p.as_str()) {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "invalid priority '{p}'; must be one of: low, medium, high, urgent"
                ),
            });
        }
    }

    if filters.column_ids.len() > MAX_FILTER_COLLECTION_SIZE {
        return Err(ApiError::InvalidInput {
            message: format!("column_ids must not exceed {MAX_FILTER_COLLECTION_SIZE} entries"),
        });
    }

    if filters.labels.len() > MAX_FILTER_COLLECTION_SIZE {
        return Err(ApiError::InvalidInput {
            message: format!("labels must not exceed {MAX_FILTER_COLLECTION_SIZE} entries"),
        });
    }

    Ok(())
}
