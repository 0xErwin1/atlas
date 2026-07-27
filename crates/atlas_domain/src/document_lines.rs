/// Counts the LF-delimited lines in document content.
pub fn document_line_count(content: &str) -> usize {
    document_lines(content).count()
}

/// Iterates 1-based document lines while preserving every non-delimiter byte.
pub fn document_lines(content: &str) -> impl Iterator<Item = (usize, &str)> {
    content
        .split_inclusive('\n')
        .map(|segment| match segment.strip_suffix('\n') {
            Some(line) => line.strip_suffix('\r').unwrap_or(line),
            None => segment,
        })
        .enumerate()
        .map(|(index, line)| (index + 1, line))
}

#[cfg(test)]
mod tests {
    use super::{document_line_count, document_lines};

    #[test]
    fn lines_are_one_based_lf_segments_without_a_synthetic_trailing_line() {
        let content = "first\nsecond\nthird\n";

        assert_eq!(document_line_count(content), 3);
        assert_eq!(
            document_lines(content).collect::<Vec<_>>(),
            vec![(1, "first"), (2, "second"), (3, "third")]
        );
    }

    #[test]
    fn crlf_hides_only_the_delimiter_carriage_return_and_lone_cr_is_data() {
        let content = "first\r\nsecond\rlone\r\nthird";

        assert_eq!(document_line_count(content), 3);
        assert_eq!(
            document_lines(content).collect::<Vec<_>>(),
            vec![(1, "first"), (2, "second\rlone"), (3, "third")]
        );
    }

    #[test]
    fn terminal_lone_cr_is_preserved_as_data() {
        let content = "final\r";

        assert_eq!(document_line_count(content), 1);
        assert_eq!(
            document_lines(content).collect::<Vec<_>>(),
            vec![(1, "final\r")]
        );
    }

    #[test]
    fn empty_content_has_no_lines() {
        assert_eq!(document_line_count(""), 0);
        assert!(document_lines("").next().is_none());
    }
}
