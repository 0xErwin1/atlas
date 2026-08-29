const MAX_SLUG_LEN: usize = 80;

/// Converts a title into a URL-safe slug.
///
/// Rules: lowercase, non-alphanumeric runs replaced with `-`, leading/trailing
/// hyphens trimmed, truncated to 80 characters. Returns `"untitled"` when the
/// input is empty or produces only hyphens after conversion.
pub fn slugify(title: &str) -> String {
    let lowered = title.to_lowercase();

    let mut slug = String::with_capacity(lowered.len());
    let mut last_was_hyphen = true;

    for ch in lowered.chars() {
        if ch.is_alphanumeric() {
            slug.push(ch);
            last_was_hyphen = false;
        } else if !last_was_hyphen {
            slug.push('-');
            last_was_hyphen = true;
        }
    }

    let slug = slug.trim_end_matches('-');

    if slug.is_empty() {
        return "untitled".to_string();
    }

    let truncated: String = slug.chars().take(MAX_SLUG_LEN).collect();

    truncated.trim_end_matches('-').to_string()
}

/// Appends a numeric suffix (`-2`, `-3`, …) to `base` until a slug not in
/// `taken` is found, then returns that collision-free slug.
///
/// If `base` itself is not in `taken`, returns `base` unchanged.
pub fn resolve_collision(base: &str, taken: &[&str]) -> String {
    if !taken.contains(&base) {
        return base.to_string();
    }

    let mut n: u64 = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !taken.contains(&candidate.as_str()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_conversion() {
        assert_eq!(slugify("Hello World"), "hello-world");
    }

    #[test]
    fn non_alphanumeric_chars_become_single_hyphen() {
        assert_eq!(slugify("foo  bar--baz"), "foo-bar-baz");
    }

    #[test]
    fn already_clean_slug_passes_through() {
        assert_eq!(slugify("my-doc"), "my-doc");
    }

    #[test]
    fn uppercase_lowercased() {
        assert_eq!(slugify("My Great Document"), "my-great-document");
    }

    #[test]
    fn empty_title_returns_untitled() {
        assert_eq!(slugify(""), "untitled");
    }

    #[test]
    fn all_special_chars_returns_untitled() {
        assert_eq!(slugify("---!!!"), "untitled");
    }

    #[test]
    fn max_length_truncation_at_80() {
        let long = "a".repeat(100);
        let result = slugify(&long);
        assert_eq!(result.len(), 80);
    }

    #[test]
    fn truncation_does_not_leave_trailing_hyphen() {
        let mut title = "a".repeat(79);
        title.push(' ');
        title.push_str(&"b".repeat(10));
        let result = slugify(&title);
        assert!(
            !result.ends_with('-'),
            "slug must not end with hyphen after truncation"
        );
        assert!(result.len() <= 80);
    }

    #[test]
    fn collision_suffix_2_when_base_taken() {
        let taken = ["my-doc"];
        assert_eq!(resolve_collision("my-doc", &taken), "my-doc-2");
    }

    #[test]
    fn collision_suffix_3_when_2_also_taken() {
        let taken = ["my-doc", "my-doc-2"];
        assert_eq!(resolve_collision("my-doc", &taken), "my-doc-3");
    }

    #[test]
    fn no_collision_returns_base_unchanged() {
        let taken: &[&str] = &[];
        assert_eq!(resolve_collision("my-doc", taken), "my-doc");
    }
}
