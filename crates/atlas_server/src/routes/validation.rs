use crate::error::ApiError;
use atlas_api::dtos::task_views::TaskViewFiltersDto;
use atlas_domain::entities::workspace_core::{PropertyDefinition, PropertyKind};
use serde_json::Value;

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

/// Validates a task's custom-property map against the workspace's property
/// definitions.
///
/// `custom` is the per-task map of definition key to value. Each entry is checked
/// against the matching definition: an unknown key, a value whose JSON type does
/// not match the field kind, or a select value outside the allowed options is
/// rejected with a 422. A JSON `null` value is always allowed — it clears the
/// field. A `null` or non-object `custom` is a no-op (the entry-count check and
/// the DB own those cases).
pub(crate) fn validate_custom_properties(
    custom: &Value,
    definitions: &[PropertyDefinition],
) -> Result<(), ApiError> {
    let map = match custom {
        Value::Object(map) => map,
        _ => return Ok(()),
    };

    for (key, value) in map {
        if value.is_null() {
            continue;
        }

        let definition =
            definitions
                .iter()
                .find(|d| &d.key == key)
                .ok_or_else(|| ApiError::InvalidInput {
                    message: format!("unknown custom field '{key}'"),
                })?;

        validate_custom_value(key, value, definition)?;
    }

    Ok(())
}

fn validate_custom_value(
    key: &str,
    value: &Value,
    definition: &PropertyDefinition,
) -> Result<(), ApiError> {
    let type_error = |expected: &str| ApiError::InvalidInput {
        message: format!("custom field '{key}' must be {expected}"),
    };

    match definition.kind {
        PropertyKind::Text => {
            if !value.is_string() {
                return Err(type_error("a string"));
            }
        }
        PropertyKind::Number => {
            if !value.is_number() {
                return Err(type_error("a number"));
            }
        }
        PropertyKind::Boolean => {
            if !value.is_boolean() {
                return Err(type_error("a boolean"));
            }
        }
        PropertyKind::Date => {
            let s = value
                .as_str()
                .ok_or_else(|| type_error("an RFC 3339 date string"))?;
            if chrono::DateTime::parse_from_rfc3339(s).is_err() {
                return Err(type_error("an RFC 3339 date string"));
            }
        }
        PropertyKind::Select => {
            let s = value.as_str().ok_or_else(|| type_error("a string"))?;
            let allowed = definition_options(definition);
            if !allowed.iter().any(|o| o == s) {
                return Err(ApiError::InvalidInput {
                    message: format!("custom field '{key}' value '{s}' is not an allowed option"),
                });
            }
        }
        PropertyKind::MultiSelect => {
            let array = value
                .as_array()
                .ok_or_else(|| type_error("an array of strings"))?;
            let allowed = definition_options(definition);

            let mut seen = std::collections::HashSet::new();
            for entry in array {
                let s = entry
                    .as_str()
                    .ok_or_else(|| type_error("an array of strings"))?;

                if !allowed.iter().any(|o| o == s) {
                    return Err(ApiError::InvalidInput {
                        message: format!(
                            "custom field '{key}' value '{s}' is not an allowed option"
                        ),
                    });
                }

                if !seen.insert(s) {
                    return Err(ApiError::InvalidInput {
                        message: format!("custom field '{key}' contains duplicate value '{s}'"),
                    });
                }
            }
        }
    }

    Ok(())
}

fn definition_options(definition: &PropertyDefinition) -> Vec<String> {
    definition
        .options
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|o| o.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
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

/// Validates a `task_prefix` value against the DB CHECK constraint.
///
/// Accepted format: 2–10 characters, first character A–Z, remaining characters A–Z or 0–9.
/// This mirrors the `projects_task_prefix_check` DB constraint exactly.
pub(crate) fn validate_task_prefix(field: &str, value: &str) -> Result<(), ApiError> {
    let bytes = value.as_bytes();
    let len = bytes.len();

    let first_ok = bytes
        .first()
        .map(|b| b.is_ascii_uppercase())
        .unwrap_or(false);

    let rest_ok = bytes
        .get(1..)
        .map(|rest| {
            rest.iter()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        })
        .unwrap_or(false);

    if first_ok && rest_ok && (2..=10).contains(&len) {
        return Ok(());
    }

    Err(ApiError::InvalidInput {
        message: format!(
            "{field} must be 2–10 characters, start with A–Z, and contain only A–Z or 0–9"
        ),
    })
}

/// Valid swatch IDs, mirroring `apps/web/src/lib/swatches.ts`.
const VALID_SWATCH_IDS: &[&str] = &[
    "neutral", "blue", "green", "amber", "red", "magenta", "cyan",
];

fn is_hex_color(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].bytes().all(|b| b.is_ascii_hexdigit())
}

/// Validates a color value against the known swatch-id set or a `#RRGGBB` hex color.
///
/// Accepts the 7 swatch ids and any 6-digit hex string in the form `#RRGGBB`
/// (case-insensitive). Rejects everything else so arbitrary strings cannot be stored.
/// `None` is accepted (no color = use the default).
pub(crate) fn validate_swatch(field: &str, id: &str) -> Result<(), ApiError> {
    if VALID_SWATCH_IDS.contains(&id) || is_hex_color(id) {
        return Ok(());
    }

    Err(ApiError::InvalidInput {
        message: format!(
            "{field} must be one of: {} or a #RRGGBB hex color",
            VALID_SWATCH_IDS.join(", ")
        ),
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
