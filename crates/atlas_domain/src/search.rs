use chrono::NaiveDate;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Type filter — set of content kinds
// ---------------------------------------------------------------------------

/// Selected content kinds for a search. A set, not a single value.
///
/// `notes` selects documents; `tasks` selects board tasks. The default and the
/// canonical "no restriction" value is `all()` (every field `true`); an empty
/// set is never produced by `parse` (it collapses to `all()`), so the SQL gate
/// only sees a non-empty set on the request path.
///
/// Forward-compat: a future third kind (e.g. `comments: bool`) is an additive
/// field. After that, `{notes:true, tasks:true}` will NOT equal `all()` — it
/// will exclude comments — which is the correct future behavior. The two paths
/// (an explicit set vs the `all` sentinel) are deliberately distinguishable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TypeSet {
    pub notes: bool,
    pub tasks: bool,
}

impl Default for TypeSet {
    fn default() -> Self {
        Self::all()
    }
}

impl TypeSet {
    /// Every kind selected — the canonical "no restriction" value.
    pub const fn all() -> Self {
        Self {
            notes: true,
            tasks: true,
        }
    }

    /// No kind selected. Internal use only (e.g. as a fold accumulator);
    /// `parse` never returns this — an empty parse result collapses to `all()`.
    pub const fn none() -> Self {
        Self {
            notes: false,
            tasks: false,
        }
    }

    /// True iff no kind is selected.
    pub const fn is_empty(self) -> bool {
        !self.notes && !self.tasks
    }

    /// True iff every kind is selected (today: both notes and tasks).
    pub const fn is_all(self) -> bool {
        self.notes && self.tasks
    }

    /// Parses a comma-separated list of type tokens into a `TypeSet`.
    ///
    /// Grammar (shared by the `type=` query param and the `type:` q-token):
    /// - split on `,`; trim and ascii-lowercase each token;
    /// - `all` anywhere in the list short-circuits to `all()` (D2);
    /// - `note`/`notes`/`document`/`documents` -> notes; `task`/`tasks` -> tasks;
    /// - unknown / unsupported tokens are ignored (D1, lenient);
    /// - if the resulting set is empty (only unknowns, or an empty/`type=` input),
    ///   it collapses to `all()` — "empty == all", matching absent-param semantics.
    ///   So `parse` NEVER returns an empty set.
    pub fn parse(raw: &str) -> Self {
        let mut acc = Self::none();
        for token in raw.split(',') {
            let t = token.trim().to_ascii_lowercase();
            match t.as_str() {
                "all" => return Self::all(),
                "note" | "notes" | "document" | "documents" => acc.notes = true,
                "task" | "tasks" => acc.tasks = true,
                _ => {}
            }
        }
        // Empty result means only unknowns (or empty input): treat as all.
        if acc.is_empty() { Self::all() } else { acc }
    }
}

/// Single source of truth for the `TaskFilterOnNotes` condition.
///
/// A task-only filter (`status:`/`priority:`/`assignee:`) is futile when the
/// selection is notes-only — documents have no status/priority/assignee, so the
/// result is guaranteed empty. Fires iff a task-only filter is present AND the
/// selection includes notes but NOT tasks. With tasks also selected the filter
/// is meaningful on the task arm, so no warning.
pub fn task_filter_on_notes(type_set: TypeSet, has_task_only_filter: bool) -> bool {
    has_task_only_filter && type_set.notes && !type_set.tasks
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
    pub type_filter: TypeSet,
    pub warnings: Vec<SearchWarning>,
    /// When true, each free-text word is matched as a prefix (typeahead) instead
    /// of a whole word. Opt-in: defaults to `false` so the default whole-word
    /// behaviour is preserved.
    pub prefix: bool,
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
    /// Current column (status) name; `Some` only for task hits.
    pub column_name: Option<String>,
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
    let mut type_filter = TypeSet::all();
    let mut warnings: Vec<SearchWarning> = Vec::new();

    let segments = tokenise(raw);

    for segment in segments {
        if let Some(colon) = segment.find(':') {
            let key = segment[..colon].to_ascii_lowercase();
            let value = &segment[colon + 1..];

            if KNOWN_FILTER_KEYS.contains(&key.as_str()) {
                match key.as_str() {
                    "type" => {
                        type_filter = TypeSet::parse(value);
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
    if task_filter_on_notes(type_filter, task_only_present) {
        warnings.push(SearchWarning::TaskFilterOnNotes);
    }

    let text = text_parts.join(" ");

    SearchQuery {
        text,
        filters,
        sort,
        type_filter,
        warnings,
        prefix: false,
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

    // -----------------------------------------------------------------------
    // T01 — TypeSet::parse (RED → GREEN after T04)
    // -----------------------------------------------------------------------

    #[test]
    fn typeset_parse_note_single_value() {
        assert_eq!(
            TypeSet::parse("note"),
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn typeset_parse_task_single_value() {
        assert_eq!(
            TypeSet::parse("task"),
            TypeSet {
                notes: false,
                tasks: true
            }
        );
    }

    #[test]
    fn typeset_parse_all_single_value() {
        assert_eq!(TypeSet::parse("all"), TypeSet::all());
    }

    #[test]
    fn typeset_parse_note_task_multi_value() {
        assert_eq!(
            TypeSet::parse("note,task"),
            TypeSet {
                notes: true,
                tasks: true
            }
        );
    }

    #[test]
    fn typeset_parse_task_note_order_independent() {
        assert_eq!(
            TypeSet::parse("task,note"),
            TypeSet {
                notes: true,
                tasks: true
            }
        );
    }

    #[test]
    fn typeset_parse_dedup_note_note() {
        assert_eq!(
            TypeSet::parse("note,note"),
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn typeset_parse_trim_and_lowercase() {
        assert_eq!(
            TypeSet::parse("Note, Task "),
            TypeSet {
                notes: true,
                tasks: true
            }
        );
    }

    #[test]
    fn typeset_parse_note_all_short_circuits_to_all() {
        assert_eq!(TypeSet::parse("note,all"), TypeSet::all());
    }

    #[test]
    fn typeset_parse_unknown_only_collapses_to_all() {
        assert_eq!(TypeSet::parse("doc"), TypeSet::all());
    }

    #[test]
    fn typeset_parse_empty_collapses_to_all() {
        assert_eq!(TypeSet::parse(""), TypeSet::all());
    }

    #[test]
    fn typeset_parse_all_unknown_collapses_to_all() {
        assert_eq!(TypeSet::parse("comment,xyz"), TypeSet::all());
    }

    #[test]
    fn typeset_parse_document_alias() {
        assert_eq!(
            TypeSet::parse("document"),
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn typeset_parse_notes_alias() {
        assert_eq!(
            TypeSet::parse("notes"),
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn typeset_parse_documents_alias() {
        assert_eq!(
            TypeSet::parse("documents"),
            TypeSet {
                notes: true,
                tasks: false
            }
        );
    }

    #[test]
    fn typeset_parse_tasks_alias() {
        assert_eq!(
            TypeSet::parse("tasks"),
            TypeSet {
                notes: false,
                tasks: true
            }
        );
    }

    // -----------------------------------------------------------------------
    // T02 — TypeSet API + task_filter_on_notes truth table (RED → GREEN after T04)
    // -----------------------------------------------------------------------

    #[test]
    fn typeset_all_is_all() {
        let s = TypeSet::all();
        assert!(s.is_all());
        assert!(!s.is_empty());
    }

    #[test]
    fn typeset_none_is_empty() {
        let s = TypeSet::none();
        assert!(s.is_empty());
        assert!(!s.is_all());
    }

    #[test]
    fn typeset_default_is_all() {
        assert_eq!(TypeSet::default(), TypeSet::all());
    }

    #[test]
    fn task_filter_on_notes_all_with_task_only_filter_false() {
        assert!(!task_filter_on_notes(TypeSet::all(), true));
    }

    #[test]
    fn task_filter_on_notes_all_without_task_only_filter_false() {
        assert!(!task_filter_on_notes(TypeSet::all(), false));
    }

    #[test]
    fn task_filter_on_notes_notes_only_with_task_only_filter_true() {
        assert!(task_filter_on_notes(
            TypeSet {
                notes: true,
                tasks: false
            },
            true
        ));
    }

    #[test]
    fn task_filter_on_notes_notes_only_without_task_only_filter_false() {
        assert!(!task_filter_on_notes(
            TypeSet {
                notes: true,
                tasks: false
            },
            false
        ));
    }

    #[test]
    fn task_filter_on_notes_tasks_only_with_task_only_filter_false() {
        assert!(!task_filter_on_notes(
            TypeSet {
                notes: false,
                tasks: true
            },
            true
        ));
    }

    #[test]
    fn task_filter_on_notes_none_with_task_only_filter_false() {
        assert!(!task_filter_on_notes(TypeSet::none(), true));
    }

    // -----------------------------------------------------------------------
    // T03 — parse_query tests rewritten/added for TypeSet (RED → GREEN after T04)
    // -----------------------------------------------------------------------

    #[test]
    fn parse_free_text_only() {
        let q = parse_query("hello world");
        assert_eq!(q.text, "hello world");
        assert!(q.filters.is_empty());
        assert!(q.warnings.is_empty());
        assert_eq!(q.type_filter, TypeSet::all());
        assert_eq!(q.sort, SearchSort::Relevance);
    }

    #[test]
    fn parse_filter_tokens_mixed_with_free_text() {
        let q = parse_query("project:atlas tag:shell permisos");
        assert!(
            q.filters
                .contains(&SearchFilter::Project("atlas".to_string()))
        );
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
        assert!(
            q.filters
                .contains(&SearchFilter::Status("open".to_string()))
        );
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: false
            }
        );
        assert!(q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn parse_unknown_key_produces_warning_and_keeps_text() {
        let q = parse_query("unknown:foo bar");
        assert!(
            q.warnings
                .contains(&SearchWarning::UnknownKey("unknown".to_string()))
        );
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
        assert!(
            q.warnings
                .contains(&SearchWarning::BadDate("<bad".to_string()))
        );
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
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: false,
                tasks: true
            }
        );
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
        assert!(
            q.filters
                .contains(&SearchFilter::Project("foo:bar".to_string()))
        );
    }

    #[test]
    fn parse_task_only_filter_with_type_all_no_warning() {
        let q = parse_query("status:open type:task");
        assert!(
            q.filters
                .contains(&SearchFilter::Status("open".to_string()))
        );
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: false,
                tasks: true
            }
        );
        assert!(!q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn parse_priority_filter() {
        let q = parse_query("priority:high");
        assert!(
            q.filters
                .contains(&SearchFilter::Priority("high".to_string()))
        );
    }

    #[test]
    fn parse_assignee_filter() {
        let q = parse_query("assignee:alice");
        assert!(
            q.filters
                .contains(&SearchFilter::Assignee("alice".to_string()))
        );
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

    #[test]
    fn parse_type_note_task_multi_no_warning() {
        let q = parse_query("type:note,task status:open");
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: true,
                tasks: true
            }
        );
        assert!(!q.warnings.contains(&SearchWarning::TaskFilterOnNotes));
    }

    #[test]
    fn parse_type_repeated_tokens_last_wins() {
        let q = parse_query("type:note type:task");
        assert_eq!(
            q.type_filter,
            TypeSet {
                notes: false,
                tasks: true
            }
        );
    }
}
