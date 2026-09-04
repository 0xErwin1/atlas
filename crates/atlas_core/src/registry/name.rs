use crate::ids::SegmentError;
use crate::ids::segment::validate_segment;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryIdError {
    #[error("invalid {id_type}: {source}")]
    Segment {
        id_type: &'static str,
        #[source]
        source: SegmentError,
    },
    #[error("{id_type} contains illegal character `{ch}`")]
    Charset { id_type: &'static str, ch: char },
    #[error("{id_type} has an empty label")]
    EmptyLabel { id_type: &'static str },
    #[error("{id_type} is missing a namespace separator (`.`)")]
    MissingNamespace { id_type: &'static str },
}

/// Validates a dotted lowercase id: `validate_segment`, then `[a-z0-9_-]`
/// labels separated by `.`, every label non-empty.
pub(crate) fn validate_dotted_id(
    id_type: &'static str,
    value: &str,
) -> Result<(), RegistryIdError> {
    validate_segment(value).map_err(|source| RegistryIdError::Segment { id_type, source })?;

    for label in value.split('.') {
        if label.is_empty() {
            return Err(RegistryIdError::EmptyLabel { id_type });
        }

        if let Some(ch) = label
            .chars()
            .find(|ch| !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && *ch != '_' && *ch != '-')
        {
            return Err(RegistryIdError::Charset { id_type, ch });
        }
    }

    Ok(())
}

/// Validates a namespaced dotted id: `validate_dotted_id`, plus at least one
/// `.` separator, so `{component}.{name}` always has a non-empty component
/// half and a non-empty name half (the name half may itself contain further
/// dots, e.g. `search.pgvector_embeddings.index_worker`).
pub(crate) fn validate_namespaced_id(
    id_type: &'static str,
    value: &str,
) -> Result<(), RegistryIdError> {
    validate_dotted_id(id_type, value)?;

    if !value.contains('.') {
        return Err(RegistryIdError::MissingNamespace { id_type });
    }

    Ok(())
}

/// Validates a flat lowercase id: `validate_segment`, then `[a-z0-9_]` only
/// (a `.` is rejected as an illegal character).
pub(crate) fn validate_flat_id(id_type: &'static str, value: &str) -> Result<(), RegistryIdError> {
    validate_segment(value).map_err(|source| RegistryIdError::Segment { id_type, source })?;

    if let Some(ch) = value
        .chars()
        .find(|ch| !ch.is_ascii_lowercase() && !ch.is_ascii_digit() && *ch != '_')
    {
        return Err(RegistryIdError::Charset { id_type, ch });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dotted_id_accepts_well_formed_values() {
        for value in ["platform", "storage.filesystem"] {
            assert_eq!(validate_dotted_id("component id", value), Ok(()));
        }
    }

    #[test]
    fn dotted_id_rejects_malformed_values() {
        let cases = [
            (
                "",
                RegistryIdError::Segment {
                    id_type: "component id",
                    source: SegmentError::Empty,
                },
            ),
            (
                "a/b",
                RegistryIdError::Segment {
                    id_type: "component id",
                    source: SegmentError::Reserved { ch: '/' },
                },
            ),
            (
                "A",
                RegistryIdError::Charset {
                    id_type: "component id",
                    ch: 'A',
                },
            ),
            (
                " ",
                RegistryIdError::Charset {
                    id_type: "component id",
                    ch: ' ',
                },
            ),
            (
                ".a",
                RegistryIdError::EmptyLabel {
                    id_type: "component id",
                },
            ),
            (
                "a.",
                RegistryIdError::EmptyLabel {
                    id_type: "component id",
                },
            ),
            (
                "a..b",
                RegistryIdError::EmptyLabel {
                    id_type: "component id",
                },
            ),
        ];

        for (value, expected) in cases {
            assert_eq!(validate_dotted_id("component id", value), Err(expected));
        }
    }

    #[test]
    fn namespaced_id_accepts_a_dotted_component_and_multi_dot_name() {
        for value in ["acta.reindex", "search.pgvector_embeddings.index_worker"] {
            assert_eq!(validate_namespaced_id("worker id", value), Ok(()));
        }
    }

    #[test]
    fn namespaced_id_rejects_a_single_label_with_no_separator() {
        assert_eq!(
            validate_namespaced_id("worker id", "acta"),
            Err(RegistryIdError::MissingNamespace {
                id_type: "worker id"
            })
        );
    }

    #[test]
    fn namespaced_id_rejects_an_empty_component_half() {
        assert_eq!(
            validate_namespaced_id("worker id", ".reindex"),
            Err(RegistryIdError::EmptyLabel {
                id_type: "worker id"
            })
        );
    }

    #[test]
    fn flat_id_rejects_dots() {
        assert_eq!(
            validate_flat_id("schema id", "a.b"),
            Err(RegistryIdError::Charset {
                id_type: "schema id",
                ch: '.'
            })
        );
    }
}
