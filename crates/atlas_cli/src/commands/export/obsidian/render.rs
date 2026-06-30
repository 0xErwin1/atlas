#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use atlas_domain::{parse_wikilink_target, strip_frontmatter};

/// Upper bound on a wikilink inner span considered for de-UUID processing.
const MAX_LINK_LEN: usize = 512;

/// Converts a JSON frontmatter object into a YAML frontmatter block.
///
/// JSON arrays become block sequences (`key:\n  - item`). Scalars render inline.
/// Returns an empty string when `fm` is not an object or is empty, so the
/// caller can concatenate without a leading `---` on content-only documents.
pub(crate) fn frontmatter_to_yaml(fm: &serde_json::Value) -> String {
    let obj = match fm.as_object() {
        Some(m) if !m.is_empty() => m,
        _ => return String::new(),
    };

    let mut out = String::from("---\n");

    for (key, val) in obj {
        match val {
            serde_json::Value::Array(arr) => {
                out.push_str(key);
                out.push_str(":\n");
                for item in arr {
                    out.push_str("  - ");
                    out.push_str(&scalar_to_yaml_str(item));
                    out.push('\n');
                }
            }
            other => {
                out.push_str(key);
                out.push_str(": ");
                out.push_str(&scalar_to_yaml_str(other));
                out.push('\n');
            }
        }
    }

    out.push_str("---\n");
    out
}

/// Converts a single JSON value to a YAML scalar string.
///
/// Strings that would be misinterpreted by a YAML parser are double-quoted.
fn scalar_to_yaml_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => yaml_quote(s),
        serde_json::Value::Array(_) | serde_json::Value::Object(_) => yaml_quote(&v.to_string()),
    }
}

/// Returns the string quoted with `"` when it contains characters that would
/// trip a YAML parser; returns the string as-is otherwise.
fn yaml_quote(s: &str) -> String {
    if needs_yaml_quoting(s) {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

fn needs_yaml_quoting(s: &str) -> bool {
    if s.is_empty() {
        return true;
    }

    let lower = s.to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "true" | "false" | "yes" | "no" | "on" | "off" | "null" | "~"
    ) {
        return true;
    }

    if s.parse::<f64>().is_ok() {
        return true;
    }

    let first = s.chars().next().unwrap_or(' ');
    if matches!(
        first,
        '[' | '{' | '!' | '&' | '*' | '|' | '>' | '\'' | '"' | '%' | '@' | '`'
    ) {
        return true;
    }

    if s.contains(": ") || s.starts_with(": ") || s.starts_with(":") && s.len() == 1 {
        return true;
    }

    if s.contains(" #") {
        return true;
    }

    false
}

/// Rewrites every `[[inner]]` span by stripping any UUID binding, leaving only
/// the display title — producing portable Obsidian wikilinks with no server IDs.
///
/// Spans that span multiple lines or exceed `MAX_LINK_LEN` are left unchanged
/// because they are not valid wikilinks.
pub(crate) fn deuid_links(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let mut remaining = content;

    while let Some(open_pos) = remaining.find("[[") {
        result.push_str(&remaining[..open_pos]);
        remaining = &remaining[open_pos + 2..];

        let Some(close_pos) = remaining.find("]]") else {
            result.push_str("[[");
            result.push_str(remaining);
            remaining = "";
            break;
        };

        let inner = &remaining[..close_pos];
        remaining = &remaining[close_pos + 2..];

        if inner.is_empty() || inner.contains('\n') || inner.len() > MAX_LINK_LEN {
            result.push_str("[[");
            result.push_str(inner);
            result.push_str("]]");
            continue;
        }

        let (_id, title) = parse_wikilink_target(inner);
        result.push_str("[[");
        result.push_str(&title);
        result.push_str("]]");
    }

    result.push_str(remaining);
    result
}

/// Renders a document as Obsidian markdown by rebuilding its frontmatter from
/// the Atlas JSONB representation and stripping any UUID bindings from body links.
///
/// `content` is the raw stored content (may include an existing frontmatter
/// block which is stripped and replaced by the rebuilt YAML). `_title` is
/// available for callers that need to name the file but is not emitted into
/// the body.
pub(crate) fn render_doc(_title: &str, frontmatter: &serde_json::Value, content: &str) -> String {
    let (_, body) = strip_frontmatter(content);
    format!("{}{}", frontmatter_to_yaml(frontmatter), deuid_links(body))
}

/// Returns a safe `.md` filename derived from `title`.
///
/// Sanitization: replaces `/` with `-`, removes ASCII control characters and
/// any leading `.` characters. If the sanitized title is empty, `slug` is used
/// instead. The `.md` extension is always appended.
pub(crate) fn safe_filename(title: &str, slug: &str) -> String {
    let sanitized: String = title
        .chars()
        .map(|c| if c == '/' { '-' } else { c })
        .filter(|c| !c.is_ascii_control())
        .collect::<String>()
        .trim_start_matches('.')
        .to_string();

    let base = if sanitized.is_empty() {
        slug.to_string()
    } else {
        sanitized
    };

    if base.ends_with(".md") {
        base
    } else {
        format!("{base}.md")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // -- frontmatter_to_yaml --------------------------------------------------

    #[test]
    fn frontmatter_empty_object_returns_empty_string() {
        assert_eq!(frontmatter_to_yaml(&json!({})), "");
    }

    #[test]
    fn frontmatter_null_returns_empty_string() {
        assert_eq!(frontmatter_to_yaml(&json!(null)), "");
    }

    #[test]
    fn frontmatter_non_object_string_returns_empty() {
        assert_eq!(frontmatter_to_yaml(&json!("just a string")), "");
    }

    #[test]
    fn frontmatter_scalar_string_value() {
        let fm = json!({ "type": "task" });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\ntype: task\n---\n");
    }

    #[test]
    fn frontmatter_boolean_values() {
        let fm = json!({ "done": true });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\ndone: true\n---\n");
    }

    #[test]
    fn frontmatter_null_value_emits_null() {
        let fm = json!({ "foo": null });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\nfoo: null\n---\n");
    }

    #[test]
    fn frontmatter_array_becomes_block_sequence() {
        let fm = json!({ "tags": ["rust", "cli"] });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\ntags:\n  - rust\n  - cli\n---\n");
    }

    #[test]
    fn frontmatter_empty_array_emits_block_sequence_header_only() {
        let fm = json!({ "tags": [] });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\ntags:\n---\n");
    }

    #[test]
    fn frontmatter_number_value() {
        let fm = json!({ "estimate": 3 });
        let out = frontmatter_to_yaml(&fm);
        assert_eq!(out, "---\nestimate: 3\n---\n");
    }

    // -- deuid_links ----------------------------------------------------------

    #[test]
    fn deuid_plain_link_unchanged() {
        assert_eq!(
            deuid_links("See [[Architecture]]."),
            "See [[Architecture]]."
        );
    }

    #[test]
    fn deuid_uuid_bound_link_strips_uuid() {
        let uuid = "019ed5fa-0000-7000-8000-000000000001";
        let input = format!("[[{uuid}|Design Doc]]");
        assert_eq!(deuid_links(&input), "[[Design Doc]]");
    }

    #[test]
    fn deuid_mixed_links_in_same_paragraph() {
        let uuid = "019ed5fa-0000-7000-8000-000000000002";
        let input = format!("See [[{uuid}|Doc A]] and [[Plain B]].");
        assert_eq!(deuid_links(&input), "See [[Doc A]] and [[Plain B]].");
    }

    #[test]
    fn deuid_no_links_returns_content_unchanged() {
        let s = "Just some prose without wikilinks.";
        assert_eq!(deuid_links(s), s);
    }

    #[test]
    fn deuid_unclosed_bracket_preserved_verbatim() {
        let s = "A [[Unclosed link";
        assert_eq!(deuid_links(s), s);
    }

    #[test]
    fn deuid_empty_content_returns_empty() {
        assert_eq!(deuid_links(""), "");
    }

    #[test]
    fn deuid_link_with_no_pipe_has_no_change() {
        assert_eq!(deuid_links("[[Foo Bar]]"), "[[Foo Bar]]");
    }

    // -- render_doc -----------------------------------------------------------

    #[test]
    fn render_doc_rebuilds_frontmatter_and_deduids_body() {
        let uuid = "019ed5fa-0000-7000-8000-000000000003";
        let fm = json!({ "type": "task", "status": "done" });
        let raw_content = format!("---\ntype: task\nstatus: old\n---\n\nSee [[{uuid}|Spec]].\n");

        let result = render_doc("My Task", &fm, &raw_content);

        assert!(result.starts_with("---\n"));
        assert!(result.contains("type: task"));
        assert!(result.contains("status: done"));
        assert!(result.contains("[[Spec]]"));
        assert!(!result.contains(uuid));
    }

    #[test]
    fn render_doc_no_frontmatter_just_returns_body() {
        let fm = json!({});
        let content = "Plain body with [[Link]].";
        let result = render_doc("Title", &fm, content);
        assert_eq!(result, "Plain body with [[Link]].");
    }

    #[test]
    fn render_doc_strips_existing_frontmatter_and_rebuilds() {
        let fm = json!({ "type": "epic" });
        let content = "---\ntype: old-type\n---\nBody text.";
        let result = render_doc("Doc", &fm, content);
        assert_eq!(result, "---\ntype: epic\n---\nBody text.");
    }

    // -- safe_filename --------------------------------------------------------

    #[test]
    fn safe_filename_simple_title_gets_md_extension() {
        assert_eq!(
            safe_filename("My Document", "my-document"),
            "My Document.md"
        );
    }

    #[test]
    fn safe_filename_slash_replaced_with_dash() {
        assert_eq!(safe_filename("a/b/c", "a-b-c"), "a-b-c.md");
    }

    #[test]
    fn safe_filename_leading_dot_stripped() {
        assert_eq!(safe_filename(".hidden", "hidden"), "hidden.md");
    }

    #[test]
    fn safe_filename_already_has_md_extension() {
        assert_eq!(safe_filename("note.md", "note"), "note.md");
    }

    #[test]
    fn safe_filename_empty_title_falls_back_to_slug() {
        assert_eq!(safe_filename("", "my-slug"), "my-slug.md");
    }

    #[test]
    fn safe_filename_all_dots_stripped_falls_back_to_slug() {
        assert_eq!(safe_filename("...", "slug"), "slug.md");
    }

    #[test]
    fn safe_filename_control_chars_removed() {
        assert_eq!(safe_filename("note\x00file", "slug"), "notefile.md");
    }
}
