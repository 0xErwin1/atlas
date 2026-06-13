/// Parses `[[Target]]` wikilinks from markdown content.
///
/// Returns de-duplicated target strings in order of first appearance.
pub fn parse_wikilinks(content: &str) -> Vec<String> {
    let mut targets: Vec<String> = Vec::new();
    let mut remaining = content;

    while let Some(open) = remaining.find("[[") {
        remaining = &remaining[open + 2..];

        if let Some(close) = remaining.find("]]") {
            let target = &remaining[..close];
            remaining = &remaining[close + 2..];

            if !target.is_empty() && !targets.iter().any(|t| t == target) {
                targets.push(target.to_string());
            }
        } else {
            break;
        }
    }

    targets
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
}
