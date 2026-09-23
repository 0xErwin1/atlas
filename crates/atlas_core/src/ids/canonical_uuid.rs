use uuid::Uuid;

/// The UUID `text` spells, only when it is spelled canonically: lowercase,
/// hyphenated. Grant and deny targets are stored on that spelling, so any
/// other spelling `Uuid::parse_str` accepts (uppercase, simple, braced)
/// would name the same row under a path no deny covers. Providers treat
/// such aliases as missing.
pub fn canonical_uuid(text: &str) -> Option<Uuid> {
    Uuid::parse_str(text)
        .ok()
        .filter(|parsed| parsed.hyphenated().to_string() == text)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;

    const CANONICAL: &str = "0190c3a4-5b6e-7f80-9a1b-2c3d4e5f6a7b";

    #[test]
    fn the_lowercase_hyphenated_spelling_is_canonical() {
        assert_eq!(
            canonical_uuid(CANONICAL),
            Some(Uuid::parse_str(CANONICAL).unwrap())
        );
    }

    #[test]
    fn an_uppercase_spelling_is_not_canonical() {
        assert_eq!(canonical_uuid(&CANONICAL.to_uppercase()), None);
    }

    #[test]
    fn a_simple_spelling_is_not_canonical() {
        assert_eq!(canonical_uuid(&CANONICAL.replace('-', "")), None);
    }

    #[test]
    fn a_braced_spelling_is_not_canonical() {
        assert_eq!(canonical_uuid(&format!("{{{CANONICAL}}}")), None);
    }

    #[test]
    fn text_that_is_not_a_uuid_is_not_canonical() {
        assert_eq!(canonical_uuid("not-a-uuid"), None);
    }
}
