use chrono::NaiveDate;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Enums / sum types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum TypeFilter {
    #[default]
    All,
    Documents,
    Tasks,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SearchSort {
    #[default]
    Relevance,
    UpdatedDesc,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKind {
    Document,
    Task,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SearchFilter {
    Project(String),
    Tag(String),
    Status(String),
    Priority(String),
    Assignee(String),
    UpdatedAfter(NaiveDate),
    UpdatedBefore(NaiveDate),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchWarning {
    /// Task-only filter present with Documents-only type_filter: results will be empty.
    TaskFilterOnNotes,
    /// A key:value token had an unrecognised key; the whole segment is kept as free text.
    UnknownKey(String),
    /// An `updated:` token had an unparseable date; the segment is dropped.
    BadDate(String),
}

// ---------------------------------------------------------------------------
// Aggregate query
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchQuery {
    /// Free-text portion (everything that is not a recognised filter token).
    pub text: String,
    pub filters: Vec<SearchFilter>,
    pub sort: SearchSort,
    pub type_filter: TypeFilter,
    pub warnings: Vec<SearchWarning>,
}

// ---------------------------------------------------------------------------
// Domain hit (returned by SearchRepo)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SearchHit {
    pub kind: SearchKind,
    pub id: Uuid,
    /// Present only for tasks.
    pub readable_id: Option<String>,
    pub title: String,
    /// Highlighted snippet; `None` when the match is title-only or on filter-only queries.
    pub snippet: Option<String>,
    pub score: f32,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// Owning project slug; `None` for workspace-root documents with no project.
    pub project_slug: Option<String>,
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

const KNOWN_FILTER_KEYS: &[&str] = &[
    "type", "sort", "project", "tag", "status", "priority", "assignee", "updated",
];

/// Parses a raw search query string into a structured `SearchQuery`.
///
/// Token grammar:
/// - Segments are separated by ASCII whitespace.
/// - A quoted run (`"..."`) is a single segment (the quotes are preserved in
///   the text portion).
/// - A segment is a filter token iff it matches `key:value` with a known key
///   (split on the FIRST `:` only; key is ASCII-lowercase).
/// - Unknown `key:value` tokens produce `SearchWarning::UnknownKey` and the
///   whole segment is appended to free text.
/// - `updated:>YYYY-MM-DD` / `updated:<YYYY-MM-DD` / `updated:YYYY-MM-DD`
///   produce `UpdatedAfter` / `UpdatedBefore` filters; unparseable dates
///   produce `SearchWarning::BadDate` and the segment is dropped.
/// - `type:` and `sort:` tokens update `type_filter` / `sort` directly; they
///   are NOT appended to free text.
/// - When a task-only filter (`status:`, `priority:`, `assignee:`) is present
///   AND the effective `type_filter` is `Documents`, a `TaskFilterOnNotes`
///   warning is appended (caller may return an empty page).
pub fn parse_query(raw: &str) -> SearchQuery {
    let mut text_parts: Vec<String> = Vec::new();
    let mut filters: Vec<SearchFilter> = Vec::new();
    let mut sort = SearchSort::Relevance;
    let mut type_filter = TypeFilter::All;
    let mut warnings: Vec<SearchWarning> = Vec::new();

    let segments = tokenise(raw);

    for segment in segments {
        if let Some(colon) = segment.find(':') {
            let key = segment[..colon].to_ascii_lowercase();
            let value = &segment[colon + 1..];

            if KNOWN_FILTER_KEYS.contains(&key.as_str()) {
                match key.as_str() {
                    "type" => {
                        type_filter = match value.to_ascii_lowercase().as_str() {
                            "note" | "document" | "notes" | "documents" => TypeFilter::Documents,
                            "task" | "tasks" => TypeFilter::Tasks,
                            _ => TypeFilter::All,
                        };
                    }
                    "sort" => {
                        sort = match value.to_ascii_lowercase().as_str() {
                            "updated" => SearchSort::UpdatedDesc,
                            _ => SearchSort::Relevance,
                        };
                    }
                    "project" => {
                        filters.push(SearchFilter::Project(value.to_string()));
                    }
                    "tag" => {
                        filters.push(SearchFilter::Tag(value.to_string()));
                    }
                    "status" => {
                        filters.push(SearchFilter::Status(value.to_string()));
                    }
                    "priority" => {
                        filters.push(SearchFilter::Priority(value.to_string()));
                    }
                    "assignee" => {
                        filters.push(SearchFilter::Assignee(value.to_string()));
                    }
                    "updated" => {
                        parse_updated_filter(value, &mut filters, &mut warnings);
                    }
                    _ => {}
                }
            } else {
                warnings.push(SearchWarning::UnknownKey(key));
                text_parts.push(segment);
            }
        } else {
            text_parts.push(segment);
        }
    }

    let task_only_present = filters.iter().any(|f| {
        matches!(
            f,
            SearchFilter::Status(_) | SearchFilter::Priority(_) | SearchFilter::Assignee(_)
        )
    });
    if task_only_present && type_filter == TypeFilter::Documents {
        warnings.push(SearchWarning::TaskFilterOnNotes);
    }

    let text = text_parts.join(" ");

    SearchQuery {
        text,
        filters,
        sort,
        type_filter,
        warnings,
    }
}

fn parse_updated_filter(
    value: &str,
    filters: &mut Vec<SearchFilter>,
    warnings: &mut Vec<SearchWarning>,
) {
    if let Some(date_str) = value.strip_prefix('>') {
        match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(d) => filters.push(SearchFilter::UpdatedAfter(d)),
            Err(_) => warnings.push(SearchWarning::BadDate(value.to_string())),
        }
    } else if let Some(date_str) = value.strip_prefix('<') {
        match NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(d) => filters.push(SearchFilter::UpdatedBefore(d)),
            Err(_) => warnings.push(SearchWarning::BadDate(value.to_string())),
        }
    } else {
        match NaiveDate::parse_from_str(value, "%Y-%m-%d") {
            Ok(d) => filters.push(SearchFilter::UpdatedAfter(d)),
            Err(_) => warnings.push(SearchWarning::BadDate(value.to_string())),
        }
    }
}

/// Splits `raw` into segments, respecting double-quoted spans as single segments.
///
/// Quoted segments include the surrounding quotes so that the caller can
/// preserve the quoted phrase in the free-text output.
fn tokenise(raw: &str) -> Vec<String> {
    let mut segments = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in raw.chars() {
        match ch {
            '"' => {
                in_quotes = !in_quotes;
                current.push(ch);
            }
            c if c.is_ascii_whitespace() && !in_quotes => {
                let trimmed = current.trim().to_string();
                if !trimmed.is_empty() {
                    segments.push(trimmed);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        segments.push(trimmed);
    }

    segments
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_free_text_only() {
        let q = parse_query("hello world");
        assert_eq!(q.text, "hello world");
        assert!(q.filters.is_empty());
        assert!(q.warnings.is_empty());
        assert_eq!(q.type_filter, TypeFilter::All);
        assert_eq!(q.sort, SearchSort::Relevance);
    }

    #[test]
    fn parse_filter_tokens_mixed_with_free_text() {
        let q = parse_query("project:atlas tag:shell permisos");
        assert!(q.filters.contains(&SearchFilter::Project("atlas".to_string())));
        assert!(q.filters.contains(&SearchFilter::Tag("shell".to_string())));
        assert_eq!(q.text, "permisos");
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn parse_quoted_phrase_kept_in_text() {
        let q = parse_query("\"quoted phrase\"");
        assert_eq!(q.text, "\"quoted phrase\"");
        assert!(q.filters.is_empty());
    }

    #[test]
    fn parse_status_token_with_note_type_produces_task_filter_on_notes_warning() {
        let q = parse_query("status:open type:note");
        assert!(q.filters.contains(&SearchFilter::Status("open".to_string())));
        assert_eq!(q.type_filter, TypeFilter::Documents);
        assert!(q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn parse_unknown_key_produces_warning_and_keeps_text() {
        let q = parse_query("unknown:foo bar");
        assert!(q.warnings.contains(&SearchWarning::UnknownKey("unknown".to_string())));
        assert!(q.text.contains("unknown:foo"));
        assert!(q.text.contains("bar"));
    }

    #[test]
    fn parse_updated_after_date() {
        let q = parse_query("updated:>2025-01-01");
        let expected = NaiveDate::from_ymd_opt(2025, 1, 1).unwrap();
        assert!(q.filters.contains(&SearchFilter::UpdatedAfter(expected)));
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn parse_updated_before_date() {
        let q = parse_query("updated:<2025-12-31");
        let expected = NaiveDate::from_ymd_opt(2025, 12, 31).unwrap();
        assert!(q.filters.contains(&SearchFilter::UpdatedBefore(expected)));
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn parse_updated_bare_date_is_after() {
        let q = parse_query("updated:2025-06-15");
        let expected = NaiveDate::from_ymd_opt(2025, 6, 15).unwrap();
        assert!(q.filters.contains(&SearchFilter::UpdatedAfter(expected)));
    }

    #[test]
    fn parse_updated_bad_date_produces_warning() {
        let q = parse_query("updated:<bad");
        assert!(q.filters.is_empty());
        assert!(q.warnings.contains(&SearchWarning::BadDate("<bad".to_string())));
    }

    #[test]
    fn parse_whitespace_only_returns_empty() {
        let q = parse_query("   ");
        assert_eq!(q.text, "");
        assert!(q.filters.is_empty());
        assert!(q.warnings.is_empty());
    }

    #[test]
    fn parse_empty_string_returns_empty() {
        let q = parse_query("");
        assert_eq!(q.text, "");
        assert!(q.filters.is_empty());
    }

    #[test]
    fn parse_type_token_updates_type_filter() {
        let q = parse_query("type:task hello");
        assert_eq!(q.type_filter, TypeFilter::Tasks);
        assert_eq!(q.text, "hello");
    }

    #[test]
    fn parse_sort_token_updates_sort() {
        let q = parse_query("sort:updated hello");
        assert_eq!(q.sort, SearchSort::UpdatedDesc);
        assert_eq!(q.text, "hello");
    }

    #[test]
    fn parse_split_on_first_colon_only() {
        // "project:foo:bar" — only "project" is the key, "foo:bar" is the value
        let q = parse_query("project:foo:bar");
        assert!(q.filters.contains(&SearchFilter::Project("foo:bar".to_string())));
    }

    #[test]
    fn parse_task_only_filter_with_type_all_no_warning() {
        let q = parse_query("status:open type:task");
        assert!(q.filters.contains(&SearchFilter::Status("open".to_string())));
        assert_eq!(q.type_filter, TypeFilter::Tasks);
        assert!(!q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn parse_priority_filter() {
        let q = parse_query("priority:high");
        assert!(q.filters.contains(&SearchFilter::Priority("high".to_string())));
    }

    #[test]
    fn parse_assignee_filter() {
        let q = parse_query("assignee:alice");
        assert!(q.filters.contains(&SearchFilter::Assignee("alice".to_string())));
    }

    #[test]
    fn parse_multiple_filters_of_same_key() {
        let q = parse_query("tag:rust tag:async");
        assert!(q.filters.contains(&SearchFilter::Tag("rust".to_string())));
        assert!(q.filters.contains(&SearchFilter::Tag("async".to_string())));
    }

    #[test]
    fn parse_no_colon_segment_is_free_text() {
        let q = parse_query("hello world foo");
        assert_eq!(q.text, "hello world foo");
        assert!(q.filters.is_empty());
    }
}
