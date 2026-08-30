/// Counts the LF-delimited lines in document content.
pub fn document_line_count(content: &str) -> usize {
    document_lines(content).count()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLineEdit {
    Insert {
        position: i64,
        content: String,
    },
    Replace {
        start: i64,
        end: i64,
        content: String,
    },
    Delete {
        start: i64,
        end: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DocumentLineEditError {
    InvalidInsertPosition(i64),
    InvalidRangeStart(i64),
    TerminalLoneCarriageReturnAppend,
}

/// Applies a line edit while retaining bytes outside the edited range.
pub fn apply_document_line_edit(
    document_content: &str,
    edit: DocumentLineEdit,
) -> Result<String, DocumentLineEditError> {
    match edit {
        DocumentLineEdit::Insert { position, content } => {
            let line_count = document_line_count(document_content);
            let max_position = i64::try_from(line_count)
                .unwrap_or(i64::MAX)
                .saturating_add(1);

            if position < 1 || position > max_position {
                return Err(DocumentLineEditError::InvalidInsertPosition(position));
            }

            if content.is_empty() {
                return Ok(document_content.to_string());
            }

            if position == max_position && document_content.ends_with('\r') {
                return Err(DocumentLineEditError::TerminalLoneCarriageReturnAppend);
            }

            let insertion_index = usize::try_from(position - 1).unwrap_or(usize::MAX);
            let offset = line_starts(document_content)
                .get(insertion_index)
                .copied()
                .unwrap_or(document_content.len());
            Ok(splice_document_lines(
                document_content,
                offset,
                offset,
                &content,
            ))
        }
        DocumentLineEdit::Replace {
            start,
            end,
            content,
        } => replace_document_lines(document_content, start, end, &content),
        DocumentLineEdit::Delete { start, end } => {
            replace_document_lines(document_content, start, end, "")
        }
    }
}

fn replace_document_lines(
    content: &str,
    start: i64,
    end: i64,
    replacement: &str,
) -> Result<String, DocumentLineEditError> {
    if start < 1 {
        return Err(DocumentLineEditError::InvalidRangeStart(start));
    }

    if start > end {
        return Ok(content.to_string());
    }

    let line_count = document_line_count(content);
    let line_count_i64 = i64::try_from(line_count).unwrap_or(i64::MAX);

    if start > line_count_i64 {
        return Ok(content.to_string());
    }

    let starts = line_starts(content);
    let start_index = usize::try_from(start - 1).unwrap_or(usize::MAX);
    let end_index = usize::try_from(end.min(line_count_i64) - 1).unwrap_or(usize::MAX);
    let start_offset = starts.get(start_index).copied().unwrap_or(content.len());
    let end_offset = starts.get(end_index + 1).copied().unwrap_or(content.len());

    Ok(splice_document_lines(
        content,
        start_offset,
        end_offset,
        replacement,
    ))
}

fn splice_document_lines(content: &str, start: usize, end: usize, replacement: &str) -> String {
    if replacement.is_empty() {
        let mut result = String::with_capacity(content.len() - (end - start));
        result.push_str(&content[..start]);
        result.push_str(&content[end..]);
        return result;
    }

    let payload_lines = edit_payload_lines(replacement);
    let payload_ends_empty = payload_lines.last().is_some_and(String::is_empty);
    let mut result = String::with_capacity(content.len() + replacement.len() + 2);
    result.push_str(&content[..start]);

    if start > 0 && !content[..start].ends_with('\n') {
        result.push('\n');
    }

    result.push_str(&payload_lines.join("\n"));

    if end < content.len() || payload_ends_empty {
        result.push('\n');
    }

    result.push_str(&content[end..]);
    result
}

fn line_starts(content: &str) -> Vec<usize> {
    if content.is_empty() {
        return Vec::new();
    }

    let mut starts = vec![0];

    for (offset, byte) in content.bytes().enumerate() {
        if byte == b'\n' && offset + 1 < content.len() {
            starts.push(offset + 1);
        }
    }

    starts
}

fn edit_payload_lines(content: &str) -> Vec<String> {
    content
        .split_inclusive('\n')
        .map(|segment| segment.strip_suffix('\n').unwrap_or(segment).to_string())
        .collect()
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
    use super::{
        DocumentLineEdit, DocumentLineEditError, apply_document_line_edit, document_line_count,
        document_lines,
    };

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

    #[test]
    fn insert_uses_lf_for_new_delimiters_and_preserves_untouched_bytes() {
        let result = apply_document_line_edit(
            "first\r\nsecond\rthird",
            DocumentLineEdit::Insert {
                position: 2,
                content: "inserted\nvalue\n".to_string(),
            },
        );

        assert_eq!(
            result,
            Ok("first\r\ninserted\nvalue\nsecond\rthird".to_string())
        );
    }

    #[test]
    fn insert_handles_append_empty_and_trailing_lf_payloads() {
        let appended = apply_document_line_edit(
            "last",
            DocumentLineEdit::Insert {
                position: 2,
                content: "next\n".to_string(),
            },
        );
        let empty_line = apply_document_line_edit(
            "last",
            DocumentLineEdit::Insert {
                position: 2,
                content: "\n".to_string(),
            },
        );
        let into_empty_document = apply_document_line_edit(
            "",
            DocumentLineEdit::Insert {
                position: 1,
                content: "first\n".to_string(),
            },
        );
        let empty_payload = apply_document_line_edit(
            "last",
            DocumentLineEdit::Insert {
                position: 2,
                content: String::new(),
            },
        );

        assert_eq!(appended, Ok("last\nnext".to_string()));
        assert_eq!(empty_line, Ok("last\n\n".to_string()));
        assert_eq!(into_empty_document, Ok("first".to_string()));
        assert_eq!(empty_payload, Ok("last".to_string()));
    }

    #[test]
    fn append_rejects_a_terminal_lone_carriage_return() {
        let result = apply_document_line_edit(
            "last\r",
            DocumentLineEdit::Insert {
                position: 2,
                content: "next".to_string(),
            },
        );

        assert_eq!(
            result,
            Err(DocumentLineEditError::TerminalLoneCarriageReturnAppend)
        );
    }

    #[test]
    fn empty_append_preserves_a_terminal_lone_carriage_return() {
        let result = apply_document_line_edit(
            "last\r",
            DocumentLineEdit::Insert {
                position: 2,
                content: String::new(),
            },
        );

        assert_eq!(result, Ok("last\r".to_string()));
    }

    #[test]
    fn replace_and_delete_preserve_surrounding_bytes() {
        let replaced = apply_document_line_edit(
            "first\r\nsecond\r\nthird",
            DocumentLineEdit::Replace {
                start: 2,
                end: 2,
                content: "replacement\n".to_string(),
            },
        );
        let deleted = apply_document_line_edit(
            "first\r\nsecond\r\nthird",
            DocumentLineEdit::Delete { start: 2, end: 2 },
        );

        assert_eq!(replaced, Ok("first\r\nreplacement\nthird".to_string()));
        assert_eq!(deleted, Ok("first\r\nthird".to_string()));
    }

    #[test]
    fn empty_payload_and_no_op_ranges_leave_content_unchanged() {
        let empty_replacement = apply_document_line_edit(
            "first\nsecond",
            DocumentLineEdit::Replace {
                start: 1,
                end: 1,
                content: String::new(),
            },
        );
        let reversed_range = apply_document_line_edit(
            "first\nsecond",
            DocumentLineEdit::Delete { start: 2, end: 1 },
        );
        let zero_end_range = apply_document_line_edit(
            "first\nsecond",
            DocumentLineEdit::Delete { start: 1, end: 0 },
        );
        let beyond_eof = apply_document_line_edit(
            "first\nsecond",
            DocumentLineEdit::Replace {
                start: 3,
                end: 4,
                content: "ignored".to_string(),
            },
        );
        let partially_beyond_eof = apply_document_line_edit(
            "first\nsecond",
            DocumentLineEdit::Delete { start: 2, end: 4 },
        );

        assert_eq!(empty_replacement, Ok("second".to_string()));
        assert_eq!(reversed_range, Ok("first\nsecond".to_string()));
        assert_eq!(zero_end_range, Ok("first\nsecond".to_string()));
        assert_eq!(beyond_eof, Ok("first\nsecond".to_string()));
        assert_eq!(partially_beyond_eof, Ok("first\n".to_string()));
    }

    #[test]
    fn invalid_line_bounds_are_rejected() {
        let invalid_insert = apply_document_line_edit(
            "line",
            DocumentLineEdit::Insert {
                position: 0,
                content: "new".to_string(),
            },
        );
        let invalid_range =
            apply_document_line_edit("line", DocumentLineEdit::Delete { start: -1, end: 1 });
        let zero_start = apply_document_line_edit(
            "line",
            DocumentLineEdit::Replace {
                start: 0,
                end: 1,
                content: "replacement".to_string(),
            },
        );
        let beyond_append = apply_document_line_edit(
            "line",
            DocumentLineEdit::Insert {
                position: 3,
                content: "new".to_string(),
            },
        );

        assert_eq!(
            invalid_insert,
            Err(DocumentLineEditError::InvalidInsertPosition(0))
        );
        assert_eq!(
            invalid_range,
            Err(DocumentLineEditError::InvalidRangeStart(-1))
        );
        assert_eq!(zero_start, Err(DocumentLineEditError::InvalidRangeStart(0)));
        assert_eq!(
            beyond_append,
            Err(DocumentLineEditError::InvalidInsertPosition(3))
        );
    }
}
