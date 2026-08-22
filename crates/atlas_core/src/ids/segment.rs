const RESERVED: [char; 3] = [':', '/', '*'];

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SegmentError {
    #[error("segment is empty")]
    Empty,
    #[error("segment contains reserved character `{ch}`")]
    Reserved { ch: char },
}

/// Rejects an empty segment or one containing a reserved character
/// (`:`, `/`, `*`) anywhere in it. Reserved characters are rejected
/// outright; no escaping mechanism exists.
pub(crate) fn validate_segment(value: &str) -> Result<(), SegmentError> {
    if value.is_empty() {
        return Err(SegmentError::Empty);
    }

    if let Some(ch) = value.chars().find(|ch| RESERVED.contains(ch)) {
        return Err(SegmentError::Reserved { ch });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_segment_is_rejected() {
        assert_eq!(validate_segment(""), Err(SegmentError::Empty));
    }

    #[test]
    fn reserved_characters_are_rejected_wherever_they_appear() {
        let cases = [("abc:def", ':'), ("a/b", '/'), ("do*c", '*'), ("abc:", ':')];

        for (value, ch) in cases {
            assert_eq!(validate_segment(value), Err(SegmentError::Reserved { ch }));
        }
    }

    #[test]
    fn segments_without_reserved_characters_are_accepted() {
        for value in ["document", "ABC", "abc", "a b", "a-b"] {
            assert_eq!(validate_segment(value), Ok(()));
        }
    }
}
