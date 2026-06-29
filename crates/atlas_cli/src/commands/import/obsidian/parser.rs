#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// B3 fields: VaultDoc.frontmatter (doc_type, status, depends, tags used for
// epic/task mapping) and VaultDoc.raw_content (kept for export round-trip)
// are populated but not read until Batch B3.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use atlas_domain::{parse_wikilinks, strip_frontmatter};

use crate::commands::import::obsidian::frontmatter::{ImportFrontmatter, parse_import_frontmatter};

/// A single markdown file discovered during a vault walk.
pub(crate) struct VaultDoc {
    /// Path relative to the vault root (e.g. `sdd/atlas/e12/tasks.md`).
    pub rel_path: PathBuf,
    /// Resolved document title: frontmatter `title` or filename stem.
    pub title: String,
    /// `slugify(title)` — used to predict the Atlas slug at plan time.
    pub predicted_slug: String,
    /// Full original file content.
    pub raw_content: String,
    /// Raw YAML block extracted from the frontmatter (without `---` delimiters).
    pub yaml_block: Option<String>,
    /// Document body after embed pre-detection and wikilink rewrite.
    pub body: String,
    /// Structured frontmatter values used for mapping decisions.
    pub frontmatter: ImportFrontmatter,
    /// Distinct wikilink targets collected from the rewritten body.
    pub wikilink_targets: Vec<String>,
    /// Binary `![[file.ext]]` embeds — each entry is `(vault-relative path, MIME type)`.
    pub attachment_candidates: Vec<(PathBuf, &'static str)>,
    /// `![[note.md]]` transclusions — unsupported; reported, never inlined.
    pub unsupported_embeds: Vec<PathBuf>,
}

/// Recursively walks `root`, returning one `VaultDoc` per `.md` file found.
///
/// Skip rules applied to directories: `.obsidian`, `.git`, `.trash`, and any
/// name starting with `.`. The manifest file `.atlas-import.json` is also
/// skipped. Results are sorted by `rel_path` so output is deterministic across
/// platforms and filesystems.
pub(crate) fn scan_vault(root: &Path) -> Result<Vec<VaultDoc>, std::io::Error> {
    let mut docs = Vec::new();
    collect_docs(root, root, &mut docs)?;
    docs.sort_by(|a, b| a.rel_path.cmp(&b.rel_path));
    Ok(docs)
}

fn collect_docs(root: &Path, dir: &Path, docs: &mut Vec<VaultDoc>) -> Result<(), std::io::Error> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if path.is_dir() {
            if should_skip_dir(&name_str) {
                continue;
            }
            collect_docs(root, &path, docs)?;
        } else if path.is_file() {
            if name_str == ".atlas-import.json" {
                continue;
            }
            if path.extension().and_then(OsStr::to_str) == Some("md") {
                let rel = path
                    .strip_prefix(root)
                    .map_err(|e| std::io::Error::other(e.to_string()))?
                    .to_path_buf();
                let doc = build_doc(root, rel, &path)?;
                docs.push(doc);
            }
        }
    }
    Ok(())
}

fn should_skip_dir(name: &str) -> bool {
    matches!(name, ".obsidian" | ".git" | ".trash") || name.starts_with('.')
}

/// Builds one `VaultDoc` from a single file path.
///
/// Reads the file, splits frontmatter from body, parses mapping fields, then
/// pre-detects embeds and rewrites wikilinks on the body.
fn build_doc(
    _root: &Path,
    rel_path: PathBuf,
    file_path: &Path,
) -> Result<VaultDoc, std::io::Error> {
    let raw_content = std::fs::read_to_string(file_path)?;

    let (yaml_opt, raw_body) = strip_frontmatter(&raw_content);
    let yaml_block = yaml_opt.map(str::to_string);
    let frontmatter = parse_import_frontmatter(yaml_opt.unwrap_or(""));

    let title = resolve_title(&frontmatter, &rel_path);
    let predicted_slug = atlas_domain::slugify(&title);

    let (cleaned_body, attachment_candidates, unsupported_embeds) = pre_detect_embeds(raw_body);

    let rewritten_body = rewrite_wikilinks(&cleaned_body);
    let wikilink_targets = collect_wikilink_targets(&rewritten_body);

    Ok(VaultDoc {
        rel_path,
        title,
        predicted_slug,
        raw_content,
        yaml_block,
        body: rewritten_body,
        frontmatter,
        wikilink_targets,
        attachment_candidates,
        unsupported_embeds,
    })
}

fn resolve_title(fm: &ImportFrontmatter, rel_path: &Path) -> String {
    if let Some(t) = &fm.title
        && !t.is_empty()
    {
        return t.clone();
    }
    rel_path
        .file_stem()
        .and_then(OsStr::to_str)
        .unwrap_or("untitled")
        .to_string()
}

/// Pre-detects `![[...]]` embed spans in `body`, classifying each as either a
/// binary attachment candidate or an unsupported markdown transclusion.
///
/// The `![[...]]` spans are consumed and removed from the returned body so that
/// the subsequent `rewrite_wikilinks` pass does not re-capture them as `[[...]]`
/// wikilinks. This ordering invariant is load-bearing: `parse_wikilinks` in
/// `atlas_domain` would otherwise swallow embed spans.
pub(crate) fn pre_detect_embeds(
    body: &str,
) -> (String, Vec<(PathBuf, &'static str)>, Vec<PathBuf>) {
    let mut attachments: Vec<(PathBuf, &'static str)> = Vec::new();
    let mut unsupported: Vec<PathBuf> = Vec::new();
    let mut out = String::with_capacity(body.len());
    let mut remaining = body;

    while let Some(bang_pos) = remaining.find("![[") {
        out.push_str(&remaining[..bang_pos]);
        remaining = &remaining[bang_pos + 3..];

        let Some(close) = remaining.find("]]") else {
            out.push_str("![[");
            continue;
        };

        let inner = remaining[..close].trim();
        remaining = &remaining[close + 2..];

        let target_path = PathBuf::from(inner);
        let ext = target_path
            .extension()
            .and_then(OsStr::to_str)
            .unwrap_or("");

        if ext.eq_ignore_ascii_case("md") {
            unsupported.push(target_path);
        } else {
            let mime = mime_for_ext(ext);
            attachments.push((target_path, mime));
        }
    }

    out.push_str(remaining);
    (out, attachments, unsupported)
}

/// Maps a file extension to a MIME type string.
///
/// Covers the attachment types that appear in the real `~/Atlas` vault.
/// Unknown extensions fall back to `application/octet-stream`.
pub(crate) fn mime_for_ext(ext: &str) -> &'static str {
    match ext.to_lowercase().as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "pdf" => "application/pdf",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        "zip" => "application/zip",
        _ => "application/octet-stream",
    }
}

/// Rewrites every `[[inner]]` wikilink in `body` to a bare `[[Title]]` form.
///
/// Obsidian syntax: `[[Title#heading|display alias]]`. We strip the alias
/// (everything after the first `|`) and the heading (everything after the first
/// `#`) so the server can resolve the link by `slugify(Title)`.
///
/// We do NOT reuse `atlas_domain::parse_wikilink_target` here: that function
/// treats `|` as a UUID-binding separator (server-side semantics), which is the
/// opposite of Obsidian's alias `|`. A dedicated rewrite avoids that collision.
pub(crate) fn rewrite_wikilinks(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut remaining = body;

    while let Some(open) = remaining.find("[[") {
        out.push_str(&remaining[..open]);
        remaining = &remaining[open + 2..];

        let Some(close) = remaining.find("]]") else {
            out.push_str("[[");
            continue;
        };

        let inner = &remaining[..close];
        remaining = &remaining[close + 2..];

        if inner.is_empty() || inner.contains('\n') {
            out.push_str("[[");
            out.push_str(inner);
            out.push_str("]]");
            continue;
        }

        let title = link_target_title(inner);
        out.push_str("[[");
        out.push_str(&title);
        out.push_str("]]");
    }

    out.push_str(remaining);
    out
}

/// Reduces a raw Obsidian wikilink inner string to the bare note title Atlas
/// can resolve.
///
/// Steps, in order: strip the alias (part before the first `|`), strip the
/// heading anchor (part before the first `#`), then reduce a path-style target
/// to its last path component. Obsidian resolves `[[folder/note]]` by basename,
/// but Atlas resolves `[[Title]]` only by `slugify(Title)`; since a note title
/// can never contain `/`, a `/` always denotes a vault path, so taking the last
/// component lets the link resolve instead of silently breaking.
fn link_target_title(inner: &str) -> String {
    let without_alias = match inner.split_once('|') {
        Some((title_part, _)) => title_part,
        None => inner,
    };
    let without_heading = match without_alias.split_once('#') {
        Some((title_part, _)) => title_part,
        None => without_alias,
    };
    let trimmed = without_heading.trim();

    let basename = match trimmed.rsplit_once('/') {
        Some((_, last)) if !last.trim().is_empty() => last,
        _ => trimmed,
    };

    basename.trim().to_string()
}

/// Collects distinct wikilink targets from the rewritten body.
///
/// After `rewrite_wikilinks` runs, every `[[...]]` in the body is a plain
/// `[[Title]]`, so `parse_wikilinks` returns clean title strings.
pub(crate) fn collect_wikilink_targets(rewritten_body: &str) -> Vec<String> {
    parse_wikilinks(rewritten_body)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -- mime_for_ext ---------------------------------------------------------

    #[test]
    fn mime_png() {
        assert_eq!(mime_for_ext("png"), "image/png");
    }

    #[test]
    fn mime_jpg() {
        assert_eq!(mime_for_ext("jpg"), "image/jpeg");
    }

    #[test]
    fn mime_jpeg() {
        assert_eq!(mime_for_ext("jpeg"), "image/jpeg");
    }

    #[test]
    fn mime_pdf() {
        assert_eq!(mime_for_ext("pdf"), "application/pdf");
    }

    #[test]
    fn mime_gif() {
        assert_eq!(mime_for_ext("gif"), "image/gif");
    }

    #[test]
    fn mime_webp() {
        assert_eq!(mime_for_ext("webp"), "image/webp");
    }

    #[test]
    fn mime_mp4() {
        assert_eq!(mime_for_ext("mp4"), "video/mp4");
    }

    #[test]
    fn mime_mov() {
        assert_eq!(mime_for_ext("mov"), "video/quicktime");
    }

    #[test]
    fn mime_zip() {
        assert_eq!(mime_for_ext("zip"), "application/zip");
    }

    #[test]
    fn mime_unknown_falls_back() {
        assert_eq!(mime_for_ext("xyz"), "application/octet-stream");
        assert_eq!(mime_for_ext(""), "application/octet-stream");
    }

    #[test]
    fn mime_case_insensitive() {
        assert_eq!(mime_for_ext("PNG"), "image/png");
        assert_eq!(mime_for_ext("JPG"), "image/jpeg");
    }

    // -- rewrite_wikilinks ----------------------------------------------------

    #[test]
    fn wikilink_plain_unchanged() {
        assert_eq!(
            rewrite_wikilinks("See [[Title]] here."),
            "See [[Title]] here."
        );
    }

    #[test]
    fn wikilink_strip_alias() {
        assert_eq!(rewrite_wikilinks("[[Title|alias text]]"), "[[Title]]");
    }

    #[test]
    fn wikilink_strip_heading() {
        assert_eq!(rewrite_wikilinks("[[Title#Section]]"), "[[Title]]");
    }

    #[test]
    fn wikilink_strip_heading_and_alias() {
        assert_eq!(rewrite_wikilinks("[[Title#heading|alias]]"), "[[Title]]");
    }

    #[test]
    fn wikilink_path_reduced_to_basename() {
        assert_eq!(rewrite_wikilinks("[[concepts/parsing]]"), "[[parsing]]");
    }

    #[test]
    fn wikilink_nested_path_reduced_to_basename() {
        assert_eq!(rewrite_wikilinks("[[a/b/c/note]]"), "[[note]]");
    }

    #[test]
    fn wikilink_path_with_alias_and_heading_reduced() {
        assert_eq!(
            rewrite_wikilinks("[[entities/vulkan-api#Usage|Vulkan]]"),
            "[[vulkan-api]]"
        );
    }

    #[test]
    fn wikilink_trailing_slash_falls_back_to_full() {
        assert_eq!(rewrite_wikilinks("[[folder/]]"), "[[folder/]]");
    }

    #[test]
    fn wikilink_multiple_in_body() {
        let body = "See [[A#h|alias]] and [[B]] and [[C|d]].";
        assert_eq!(rewrite_wikilinks(body), "See [[A]] and [[B]] and [[C]].");
    }

    #[test]
    fn wikilink_empty_inner_preserved_verbatim() {
        assert_eq!(rewrite_wikilinks("[[]]"), "[[]]");
    }

    // -- pre_detect_embeds ----------------------------------------------------

    #[test]
    fn binary_embed_becomes_attachment_not_wikilink() {
        let body = "Here is ![[diagram.png]] embedded.";
        let (cleaned, attachments, unsupported) = pre_detect_embeds(body);

        assert!(!cleaned.contains("![["), "embed span must be consumed");
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].0, PathBuf::from("diagram.png"));
        assert_eq!(attachments[0].1, "image/png");
        assert!(unsupported.is_empty());

        // The cleaned body must not produce a [[diagram.png]] wikilink target.
        let targets = collect_wikilink_targets(&rewrite_wikilinks(&cleaned));
        assert!(
            targets.iter().all(|t| t != "diagram.png"),
            "binary embed must not appear as a wikilink target"
        );
    }

    #[test]
    fn md_embed_is_unsupported_not_wikilink() {
        let body = "Embedded: ![[note.md]]";
        let (cleaned, attachments, unsupported) = pre_detect_embeds(body);

        assert!(attachments.is_empty());
        assert_eq!(unsupported, vec![PathBuf::from("note.md")]);

        let targets = collect_wikilink_targets(&rewrite_wikilinks(&cleaned));
        assert!(
            targets.iter().all(|t| t != "note.md"),
            "md embed must not appear as a wikilink target"
        );
    }

    #[test]
    fn mixed_embeds_and_wikilinks() {
        let body = "![[img.png]] text [[Link]] more ![[doc.md]]";
        let (cleaned, attachments, unsupported) = pre_detect_embeds(body);

        assert_eq!(attachments.len(), 1);
        assert_eq!(unsupported.len(), 1);

        let targets = collect_wikilink_targets(&rewrite_wikilinks(&cleaned));
        assert!(targets.contains(&"Link".to_string()));
        assert!(targets.iter().all(|t| t != "img.png"));
        assert!(targets.iter().all(|t| t != "doc.md"));
    }

    // -- collect_wikilink_targets --------------------------------------------

    #[test]
    fn collect_targets_from_rewritten_body() {
        let body = "See [[Alpha]] and [[Beta]] and [[Alpha]] again.";
        let targets = collect_wikilink_targets(body);
        assert_eq!(targets, vec!["Alpha", "Beta"]);
    }

    // -- scan_vault (tempfile-based) -----------------------------------------

    fn make_file(dir: &Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    #[test]
    fn scan_vault_recursive_discovery() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "a.md", "# A");
        make_file(root, "sub/b.md", "# B");
        make_file(root, "sub/deep/c.md", "# C");

        let docs = scan_vault(root).unwrap();
        let rels: Vec<_> = docs.iter().map(|d| d.rel_path.clone()).collect();

        assert!(rels.contains(&PathBuf::from("a.md")));
        assert!(rels.contains(&PathBuf::from("sub/b.md")));
        assert!(rels.contains(&PathBuf::from("sub/deep/c.md")));
        assert_eq!(docs.len(), 3);
    }

    #[test]
    fn scan_vault_non_md_ignored() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "a.md", "# A");
        make_file(root, "notes.txt", "plain text");
        make_file(root, "data.csv", "a,b,c");

        let docs = scan_vault(root).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].rel_path, PathBuf::from("a.md"));
    }

    #[test]
    fn scan_vault_skips_hidden_dirs() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "keep.md", "# Keep");
        make_file(root, ".obsidian/config.md", "hidden");
        make_file(root, ".git/x.md", "hidden");
        make_file(root, ".trash/old.md", "hidden");
        make_file(root, ".hidden/note.md", "hidden");

        let docs = scan_vault(root).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].rel_path, PathBuf::from("keep.md"));
    }

    #[test]
    fn scan_vault_skips_manifest_file() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "doc.md", "# Doc");
        make_file(root, ".atlas-import.json", "{}");

        let docs = scan_vault(root).unwrap();
        assert_eq!(docs.len(), 1);
    }

    #[test]
    fn scan_vault_sorted_deterministically() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "z.md", "# Z");
        make_file(root, "a.md", "# A");
        make_file(root, "m.md", "# M");

        let docs = scan_vault(root).unwrap();
        let rels: Vec<_> = docs.iter().map(|d| d.rel_path.to_str().unwrap()).collect();
        assert_eq!(rels, vec!["a.md", "m.md", "z.md"]);
    }

    #[test]
    fn build_doc_title_from_frontmatter() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(
            root,
            "weird-slug.md",
            "---\ntitle: My Real Title\n---\n# Body",
        );

        let docs = scan_vault(root).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "My Real Title");
    }

    #[test]
    fn build_doc_title_from_filename_stem() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "Meeting Notes.md", "No frontmatter here.");

        let docs = scan_vault(root).unwrap();
        assert_eq!(docs.len(), 1);
        assert_eq!(docs[0].title, "Meeting Notes");
    }

    #[test]
    fn build_doc_binary_embed_pre_detected() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "doc.md", "See ![[diagram.png]] for details.");

        let docs = scan_vault(root).unwrap();
        let doc = &docs[0];

        assert_eq!(doc.attachment_candidates.len(), 1);
        assert_eq!(doc.attachment_candidates[0].0, PathBuf::from("diagram.png"));
        assert!(
            doc.wikilink_targets.iter().all(|t| t != "diagram.png"),
            "binary embed must not appear as wikilink target"
        );
    }

    #[test]
    fn build_doc_md_embed_is_unsupported() {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path();

        make_file(root, "doc.md", "Transclude: ![[other.md]]");

        let docs = scan_vault(root).unwrap();
        let doc = &docs[0];

        assert_eq!(doc.unsupported_embeds.len(), 1);
        assert_eq!(doc.unsupported_embeds[0], PathBuf::from("other.md"));
        assert!(doc.attachment_candidates.is_empty());
        assert!(
            doc.wikilink_targets.iter().all(|t| t != "other.md"),
            "md embed must not appear as wikilink target"
        );
    }
}
