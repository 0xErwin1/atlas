/// Upper bound on a wikilink target's byte length. A real target is a short
/// title (optionally id-bound), never a paragraph; anything longer is a `[[`
/// opened in prose or an inline-code span whose `]]` only appears far away.
/// Bounding it also keeps the value below Postgres' btree index row limit, so a
/// false positive cannot overflow the unique index on `document_links`.
const MAX_WIKILINK_TARGET_LEN: usize = 512;

/// Parses `[[Target]]` wikilinks from markdown content.
///
/// Returns de-duplicated target strings in order of first appearance.
/// Multi-line or oversized candidates are skipped: a genuine wikilink is a
/// short, single-line title, so such spans are false positives.
pub fn parse_wikilinks(content: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    let mut remaining = content;

    while let Some(open) = remaining.find("[[") {
        remaining = &remaining[open + 2..];

        if let Some(close) = remaining.find("]]") {
            let target = &remaining[..close];
            remaining = &remaining[close + 2..];

            if !target.is_empty()
                && !target.contains('\n')
                && target.len() <= MAX_WIKILINK_TARGET_LEN
                && !targets.iter().any(|t| t == target)
            {
                targets.push(target.to_string());
            }
        } else {
            break;
        }
    }

    targets
}

/// Resolves the inner content of a `[[...]]` wikilink into an optional target
/// document id and a display title.
///
/// The syntax is split on the FIRST `|`. When a `|` is present and the left part
/// (trimmed) parses as a UUID, the link is id-bound: the UUID is the stable
/// target and the right part (trimmed) is the display title. Otherwise the link
/// is treated as legacy / hand-typed: no id, and the whole trimmed `raw` is the
/// title — this deliberately covers a `|` whose left part is not a UUID (e.g.
/// `Foo|Bar`), which stays a legacy title rather than an id-bound link.
pub fn parse_wikilink_target(raw: &str) -> (Option<uuid::Uuid>, String) {
    if let Some((left, right)) = raw.split_once('|')
        && let Ok(id) = uuid::Uuid::parse_str(left.trim())
    {
        return (Some(id), right.trim().to_string());
    }

    (None, raw.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_returns_empty() {
        assert_eq!(parse_wikilinks(""), Vec::<String>::new());
    }

    #[test]
    fn no_wikilinks_returns_empty() {
        assert_eq!(
            parse_wikilinks("Just some plain text with [single] brackets."),
            Vec::<String>::new()
        );
    }

    #[test]
    fn single_wikilink_extracted() {
        assert_eq!(
            parse_wikilinks("See [[Architecture]] for details."),
            vec!["Architecture".to_string()]
        );
    }

    #[test]
    fn duplicate_wikilinks_deduplicated() {
        let result = parse_wikilinks("[[Foo]] and then [[Foo]] again");
        assert_eq!(result, vec!["Foo".to_string()]);
    }

    #[test]
    fn multiple_distinct_wikilinks_in_order_of_first_appearance() {
        let result = parse_wikilinks("[[Alpha]] then [[Beta]] then [[Alpha]] then [[Gamma]]");
        assert_eq!(
            result,
            vec!["Alpha".to_string(), "Beta".to_string(), "Gamma".to_string()]
        );
    }

    #[test]
    fn malformed_unclosed_bracket_not_matched() {
        assert_eq!(parse_wikilinks("[[Unclosed"), Vec::<String>::new());
    }

    #[test]
    fn multiline_content_parsed_correctly() {
        let content = "# Title\n\nSee [[Design Doc]] for details.\n\nAlso check [[Architecture]].";
        let result = parse_wikilinks(content);
        assert_eq!(
            result,
            vec!["Design Doc".to_string(), "Architecture".to_string()]
        );
    }

    #[test]
    fn empty_wikilink_not_included() {
        assert_eq!(parse_wikilinks("[[]]"), Vec::<String>::new());
    }

    #[test]
    fn multiline_target_is_ignored() {
        // A real wikilink is single-line; a `[[` opened in prose/code that only
        // closes lines later must not be captured as a target.
        assert_eq!(parse_wikilinks("[[line one\nline two]]"), Vec::<String>::new());
    }

    #[test]
    fn oversized_target_is_ignored() {
        let big = "x".repeat(600);
        assert_eq!(parse_wikilinks(&format!("[[{big}]]")), Vec::<String>::new());
    }

    #[test]
    fn oversized_target_does_not_block_later_links() {
        let big = "x".repeat(600);
        assert_eq!(
            parse_wikilinks(&format!("[[{big}]] and [[Real]]")),
            vec!["Real".to_string()]
        );
    }

    #[test]
    fn target_id_bound_link_returns_uuid_and_display_title() {
        let uuid =
            uuid::Uuid::parse_str("019ed5fa-0000-7000-8000-000000000000").expect("valid test uuid");
        let raw = format!("{uuid}|Editor test");

        assert_eq!(
            parse_wikilink_target(&raw),
            (Some(uuid), "Editor test".to_string())
        );
    }

    #[test]
    fn target_id_bound_link_trims_surrounding_whitespace() {
        let uuid =
            uuid::Uuid::parse_str("019ed5fa-0000-7000-8000-000000000000").expect("valid test uuid");
        let raw = format!("  {uuid}  |  Editor test  ");

        assert_eq!(
            parse_wikilink_target(&raw),
            (Some(uuid), "Editor test".to_string())
        );
    }

    #[test]
    fn target_plain_title_returns_none_and_title() {
        assert_eq!(
            parse_wikilink_target("Plain Title"),
            (None, "Plain Title".to_string())
        );
    }

    #[test]
    fn target_non_uuid_before_pipe_is_legacy_title() {
        assert_eq!(
            parse_wikilink_target("Foo|Bar"),
            (None, "Foo|Bar".to_string())
        );
    }

    #[test]
    fn target_empty_returns_none_and_empty_title() {
        assert_eq!(parse_wikilink_target(""), (None, String::new()));
    }
}
