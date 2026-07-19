use crate::error::ApiError;
use atlas_api::dtos::task_views::TaskViewFiltersDto;
use atlas_domain::entities::workspace_core::{PropertyDefinition, PropertyKind};
use serde_json::Value;

const MAX_NAME_LEN: usize = 200;
const MAX_DESC_LEN: usize = 20_000;
const MAX_COMMENT_LEN: usize = 10_000;
const MAX_LABEL_LEN: usize = 64;
const MAX_LABELS: usize = 50;
const MAX_CUSTOM_ENTRIES: usize = 100;
const MAX_EMAIL_LEN: usize = 254;
pub(crate) const MAX_QUERY_LEN: usize = 2_000;

/// Validates the password meets minimum length before any DB work is done.
///
/// A min-length of 8 is the only rule applied here. It is checked before
/// argon2 hashing so no hashing cost is paid for a password that would be
/// rejected anyway.
pub(crate) fn validate_password_strength(pw: &str) -> Result<(), ApiError> {
    if pw.chars().count() < 8 {
        return Err(ApiError::InvalidInput {
            message: "Password must be at least 8 characters long.".into(),
        });
    }

    Ok(())
}

/// Validates an email address with a minimal, bounded format check.
///
/// Deliberately not full RFC 5322 validation: the only guarantees enforced are
/// a non-blank value, a bounded length, and a single `@` separating non-empty
/// local and domain parts.
pub(crate) fn validate_email(field: &str, value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must not be blank"),
        });
    }

    if value.len() > MAX_EMAIL_LEN {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must be at most {MAX_EMAIL_LEN} bytes"),
        });
    }

    let format_ok = match value.split_once('@') {
        Some((local, domain)) => !local.is_empty() && !domain.is_empty() && !domain.contains('@'),
        None => false,
    };

    if !format_ok {
        return Err(ApiError::InvalidInput {
            message: format!(
                "{field} must contain a single '@' with non-empty local and domain parts"
            ),
        });
    }

    Ok(())
}

/// Validates a short name/title field.
///
/// Trims the value; rejects blank strings and strings longer than 200 UTF-8 bytes.
pub(crate) fn validate_name(field: &str, value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();

    if trimmed.is_empty() {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must not be blank"),
        });
    }

    if trimmed.len() > MAX_NAME_LEN {
        return Err(ApiError::InvalidInput {
            message: format!("{field} must be at most {MAX_NAME_LEN} bytes"),
        });
    }

    Ok(())
}

/// Extensions that always denote executable or script content.
///
/// Uploads with any of these extensions are rejected regardless of their bytes:
/// an executable payload disguised under a benign name is still an executable.
/// The magic-byte layer below is an independent check — per ATL-75 we reject an
/// upload if either the extension or the sniffed content signals a disallowed
/// type.
const BLOCKED_EXTENSIONS: &[&str] = &[
    "exe", "com", "scr", "msi", "msix", "appx", "bat", "cmd", "ps1", "psm1", "vbs", "vbe", "sh",
    "bash", "zsh", "ksh", "run", "bin", "elf", "so", "dll", "dylib", "o", "out", "jar", "class",
    "app", "apk", "deb", "rpm", "wasm", "dmg", "pkg",
];

/// Enforces the extension portion of the attachment upload policy.
///
/// This is kept separate from byte/content validation so metadata-only operations,
/// such as rename, cannot bypass the built-in blocklist or configured allow-list.
pub(crate) fn validate_upload_extension(
    file_name: &str,
    allowed_extensions: Option<&std::collections::HashSet<String>>,
) -> Result<(), ApiError> {
    let extension = file_name
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase());

    if let Some(ext) = &extension
        && BLOCKED_EXTENSIONS.contains(&ext.as_str())
    {
        return Err(ApiError::InvalidInput {
            message: format!("File extension '{ext}' is not allowed"),
        });
    }

    if let Some(allowed) = allowed_extensions {
        match &extension {
            Some(ext) if allowed.contains(ext) => {}
            Some(ext) => {
                return Err(ApiError::InvalidInput {
                    message: format!("File extension '{ext}' is not allowed"),
                });
            }
            None => {
                return Err(ApiError::InvalidInput {
                    message: "files without an extension are not allowed".to_string(),
                });
            }
        }
    }

    Ok(())
}

/// Enforces the upload content policy for attachment bytes.
///
/// Rejects executables and scripts by two independent signals: a hard-floor
/// blocklist of dangerous file extensions and magic-byte inspection of the actual
/// content. Only images, documents/e-books, and plain text (including files with
/// no known binary signature that decode as valid UTF-8) are accepted.
///
/// When `allowed_extensions` is `Some`, an additional positive gate is ANDed with
/// the content check: the upload's declared extension must appear in that
/// (non-empty) allow-list, otherwise the upload is rejected regardless of its
/// bytes. When `None`, no positive gate is applied. The size limit is enforced
/// elsewhere and is intentionally untouched here.
pub(crate) fn validate_upload(
    file_name: &str,
    data: &[u8],
    allowed_extensions: Option<&std::collections::HashSet<String>>,
) -> Result<(), ApiError> {
    validate_upload_extension(file_name, allowed_extensions)?;

    match infer::get(data) {
        Some(kind) => {
            use infer::MatcherType::*;

            // `infer` classifies PDF under `Archive`, not `Doc`, so PDFs are
            // allowed explicitly by MIME type rather than by matcher category.
            let allowed = matches!(kind.matcher_type(), Image | Doc | Book)
                || kind.mime_type() == "application/pdf"
                || (kind.matcher_type() == Text && kind.extension() != "sh");

            if !allowed {
                return Err(ApiError::InvalidInput {
                    message: format!("File content type '{}' is not allowed", kind.mime_type()),
                });
            }
        }
        None => {
            if data.contains(&0) || std::str::from_utf8(data).is_err() {
                return Err(ApiError::InvalidInput {
                    message: "File type could not be identified or is not allowed".to_string(),
                });
            }
        }
    }

    Ok(())
}

const MAX_SLUG_LEN: usize = 80;

/// Validates a user-supplied workspace slug.
///
/// Accepted format mirrors the output of `slugify`: lowercase ASCII alphanumeric
/// segments joined by single hyphens, with no leading, trailing, or doubled
/// hyphens, and a length of 1–80 characters. Rejecting anything else keeps slugs
/// URL-safe and stable, so a hand-edited slug behaves identically to a derived one.
pub(crate) fn validate_slug(field: &str, value: &str) -> Result<(), ApiError> {
    let invalid = || ApiError::InvalidInput {
        message: format!(
            "{field} must be 1–{MAX_SLUG_LEN} lowercase letters, digits, or single hyphens \
             (no leading, trailing, or repeated hyphens)"
        ),
    };

    if value.is_empty() || value.len() > MAX_SLUG_LEN {
        return Err(invalid());
    }

    if value.starts_with('-') || value.ends_with('-') || value.contains("--") {
        return Err(invalid());
    }

    let charset_ok = value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');

    if !charset_ok {
        return Err(invalid());
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

/// Validates a comment body.
///
/// Rejects an empty or whitespace-only body and a body longer than 10 000
/// Unicode scalar values (counted with `chars().count()`, not byte length, so
/// multi-byte markdown content isn't penalized for its UTF-8 encoding size).
pub(crate) fn validate_comment_body(value: &str) -> Result<(), ApiError> {
    if value.trim().is_empty() {
        return Err(ApiError::InvalidInput {
            message: "body must not be blank".into(),
        });
    }

    if value.chars().count() > MAX_COMMENT_LEN {
        return Err(ApiError::InvalidInput {
            message: format!("body must be at most {MAX_COMMENT_LEN} characters"),
        });
    }

    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn accepts_png_by_magic_bytes() {
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert!(validate_upload("photo.png", data, None).is_ok());
    }

    #[test]
    fn accepts_pdf_by_magic_bytes() {
        let data = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
        assert!(validate_upload("doc.pdf", data, None).is_ok());
    }

    #[test]
    fn accepts_plain_text() {
        let data = b"hello world";
        assert!(validate_upload("note.txt", data, None).is_ok());
    }

    #[test]
    fn accepts_markdown() {
        let data = b"# Title\n\nSome **markdown** content.\n";
        assert!(validate_upload("readme.md", data, None).is_ok());
    }

    #[test]
    fn rejects_elf_with_spoofed_extension() {
        let data = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00";
        assert!(validate_upload("photo.png", data, None).is_err());
    }

    #[test]
    fn rejects_shell_script_content() {
        let data = b"#!/bin/bash\necho hi\n";
        assert!(validate_upload("script.txt", data, None).is_err());
    }

    #[test]
    fn rejects_blocked_extension_regardless_of_content() {
        let data = b"harmless text";
        assert!(validate_upload("malware.exe", data, None).is_err());
    }

    #[test]
    fn rejects_binary_without_signature() {
        let data = b"some text\x00more binary";
        assert!(validate_upload("data.txt", data, None).is_err());
    }

    #[test]
    fn allowlist_accepts_declared_extension_in_set() {
        let set = HashSet::from(["png".to_string(), "txt".to_string()]);
        let data = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        assert!(validate_upload("photo.png", data, Some(&set)).is_ok());
    }

    #[test]
    fn allowlist_rejects_declared_extension_not_in_set() {
        let set = HashSet::from(["png".to_string(), "txt".to_string()]);
        let data = b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n";
        assert!(validate_upload("doc.pdf", data, Some(&set)).is_err());
    }

    #[test]
    fn allowlist_accepts_plain_text_with_allowed_extension() {
        let set = HashSet::from(["png".to_string(), "txt".to_string()]);
        assert!(validate_upload("note.txt", b"hello", Some(&set)).is_ok());
    }

    #[test]
    fn allowlist_rejects_file_without_extension() {
        let set = HashSet::from(["png".to_string(), "txt".to_string()]);
        assert!(validate_upload("noext", b"hello", Some(&set)).is_err());
    }

    #[test]
    fn allowlist_still_rejects_executable_content_with_allowed_extension() {
        let set = HashSet::from(["png".to_string(), "txt".to_string()]);
        let data = b"\x7fELF\x02\x01\x01\x00\x00\x00\x00\x00";
        assert!(validate_upload("photo.png", data, Some(&set)).is_err());
    }

    #[test]
    fn password_strength_rejects_short_and_empty() {
        assert!(validate_password_strength("").is_err());
        assert!(validate_password_strength("short").is_err());
        assert!(validate_password_strength("1234567").is_err());
    }

    #[test]
    fn password_strength_accepts_eight_or_more_chars() {
        assert!(validate_password_strength("12345678").is_ok());
        assert!(validate_password_strength("a much longer passphrase").is_ok());
    }

    #[test]
    fn email_accepts_minimal_valid_forms() {
        assert!(validate_email("email", "a@b").is_ok());
        assert!(validate_email("email", "user.name+tag@example.com").is_ok());
    }

    #[test]
    fn email_rejects_blank_and_malformed() {
        assert!(validate_email("email", "").is_err());
        assert!(validate_email("email", "   ").is_err());
        assert!(validate_email("email", "no-at-sign").is_err());
        assert!(validate_email("email", "@no-local").is_err());
        assert!(validate_email("email", "no-domain@").is_err());
        assert!(validate_email("email", "two@@ats").is_err());
        assert!(validate_email("email", "a@b@c").is_err());
    }

    #[test]
    fn email_rejects_overlong_value() {
        let local = "a".repeat(250);
        assert!(validate_email("email", &format!("{local}@example.com")).is_err());
    }
}
