#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// Items in this module are used by parser.rs and will be called from the
// run_obsidian entrypoint once Batch B0b wires the scan→plan pipeline.
#![allow(dead_code)]

use std::collections::BTreeMap;

/// A single frontmatter value — either a bare scalar or a list of strings.
pub(crate) enum FmValue {
    Scalar(String),
    List(Vec<String>),
}

/// Client-side mapping view of Obsidian frontmatter.
///
/// Only the keys the importer maps are typed; every key (including typed ones)
/// is also preserved in `raw` for export rebuild and diagnostics. The document
/// content sent to Atlas is always verbatim — this struct is used only for the
/// importer's own decisions (type/status/depends/tags/title).
pub(crate) struct ImportFrontmatter {
    pub doc_type: Option<String>,
    pub status: Option<String>,
    pub title: Option<String>,
    pub depends: Vec<String>,
    pub tags: Vec<String>,
    pub raw: BTreeMap<String, FmValue>,
}

/// Parses an Obsidian YAML frontmatter block into an `ImportFrontmatter`.
///
/// Handles three shapes used by the real `~/Atlas` vault:
/// - Scalars: `key: value`
/// - Inline arrays: `key: [a, b, c]`
/// - Block sequences: `key:\n  - a\n  - b`
///
/// Unknown or complex values (nested maps, anchors) are stored in `raw` as a
/// `Scalar` of the raw text — a parse miss does not corrupt the document.
pub(crate) fn parse_import_frontmatter(yaml: &str) -> ImportFrontmatter {
    let raw = scan_yaml_lines(yaml);

    let doc_type = extract_scalar(&raw, "type");
    let status = extract_scalar(&raw, "status");
    let title = extract_scalar(&raw, "title");
    let depends = extract_list(&raw, "depends");
    let tags = extract_list(&raw, "tags");

    ImportFrontmatter {
        doc_type,
        status,
        title,
        depends,
        tags,
        raw,
    }
}

fn scan_yaml_lines(yaml: &str) -> BTreeMap<String, FmValue> {
    let mut raw: BTreeMap<String, FmValue> = BTreeMap::new();

    let mut seq_key: Option<String> = None;
    let mut seq_items: Vec<String> = Vec::new();

    for line in yaml.lines() {
        if let Some(ref key) = seq_key.clone() {
            if let Some(item) = parse_block_item(line) {
                seq_items.push(item);
                continue;
            }
            raw.insert(key.clone(), FmValue::List(seq_items.clone()));
            seq_key = None;
            seq_items = Vec::new();
        }

        let Some((key, rest)) = line.split_once(':') else {
            continue;
        };

        let key = key.trim();
        let rest = rest.trim();

        if key.is_empty() {
            continue;
        }

        if rest.starts_with('[') && rest.ends_with(']') {
            let inner = &rest[1..rest.len() - 1];
            raw.insert(key.to_string(), FmValue::List(parse_inline_array(inner)));
        } else if rest.is_empty() {
            seq_key = Some(key.to_string());
        } else {
            raw.insert(key.to_string(), FmValue::Scalar(unquote(rest)));
        }
    }

    if let Some(key) = seq_key {
        raw.insert(key, FmValue::List(seq_items));
    }

    raw
}

/// Returns the trimmed, unquoted item text when `line` is a YAML block
/// sequence entry (any leading whitespace followed by `- `).
fn parse_block_item(line: &str) -> Option<String> {
    let stripped = line.trim_start();
    let item = stripped.strip_prefix("- ")?;
    Some(unquote(item.trim()))
}

fn parse_inline_array(inner: &str) -> Vec<String> {
    inner
        .split(',')
        .map(|part| unquote(part.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn unquote(s: &str) -> String {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| s.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(s)
        .to_string()
}

fn extract_scalar(raw: &BTreeMap<String, FmValue>, key: &str) -> Option<String> {
    match raw.get(key) {
        Some(FmValue::Scalar(s)) => Some(s.clone()),
        _ => None,
    }
}

fn extract_list(raw: &BTreeMap<String, FmValue>, key: &str) -> Vec<String> {
    match raw.get(key) {
        Some(FmValue::List(items)) => items.clone(),
        _ => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scalar_type_and_status() {
        let yaml = "type: epic\nstatus: in-progress";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.doc_type.as_deref(), Some("epic"));
        assert_eq!(fm.status.as_deref(), Some("in-progress"));
    }

    #[test]
    fn inline_array_depends() {
        let yaml = "depends: [E01, E02]";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.depends, vec!["E01", "E02"]);
    }

    #[test]
    fn block_sequence_tags() {
        let yaml = "tags:\n  - atlas\n  - backend\n  - sdd-tasks";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["atlas", "backend", "sdd-tasks"]);
    }

    #[test]
    fn absent_key_is_empty() {
        let yaml = "type: epic";
        let fm = parse_import_frontmatter(yaml);
        assert!(fm.depends.is_empty());
    }

    #[test]
    fn real_vault_doc_shape() {
        let yaml = "type: tasks\nstatus: done\ntags:\n  - atlas\n  - backend\n  - sdd-tasks";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.doc_type.as_deref(), Some("tasks"));
        assert_eq!(fm.status.as_deref(), Some("done"));
        assert_eq!(fm.tags, vec!["atlas", "backend", "sdd-tasks"]);
    }

    #[test]
    fn quoted_scalar_unquoted() {
        let yaml = "title: \"My Title\"";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.title.as_deref(), Some("My Title"));
    }

    #[test]
    fn inline_array_with_spaces() {
        let yaml = "depends: [ E01 , E02 ]";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.depends, vec!["E01", "E02"]);
    }

    #[test]
    fn block_sequence_with_quoted_items() {
        let yaml = "tags:\n  - \"my tag\"\n  - plain";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["my tag", "plain"]);
    }

    #[test]
    fn empty_yaml_returns_empty_frontmatter() {
        let fm = parse_import_frontmatter("");
        assert!(fm.doc_type.is_none());
        assert!(fm.status.is_none());
        assert!(fm.title.is_none());
        assert!(fm.depends.is_empty());
        assert!(fm.tags.is_empty());
        assert!(fm.raw.is_empty());
    }

    #[test]
    fn all_raw_keys_present() {
        let yaml = "type: epic\nstatus: todo\ndepends: [E01]\ntags:\n  - x";
        let fm = parse_import_frontmatter(yaml);
        assert!(fm.raw.contains_key("type"));
        assert!(fm.raw.contains_key("status"));
        assert!(fm.raw.contains_key("depends"));
        assert!(fm.raw.contains_key("tags"));
    }

    #[test]
    fn block_sequence_immediately_followed_by_another_key() {
        let yaml = "tags:\n  - a\n  - b\nstatus: done";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.tags, vec!["a", "b"]);
        assert_eq!(fm.status.as_deref(), Some("done"));
    }

    #[test]
    fn inline_array_single_item() {
        let yaml = "depends: [E01]";
        let fm = parse_import_frontmatter(yaml);
        assert_eq!(fm.depends, vec!["E01"]);
    }

    #[test]
    fn empty_inline_array_produces_empty_list() {
        let yaml = "depends: []";
        let fm = parse_import_frontmatter(yaml);
        assert!(fm.depends.is_empty());
    }
}
