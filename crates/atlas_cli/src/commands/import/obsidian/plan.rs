#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// B1/B3/B4 fields: FolderOp.parent_rel, DocumentOp.predicted_slug / folder_rel
// / content, DocAction::Update.slug, BoardOp.columns, TaskOp.board_epic_rel /
// column / description / depends, LinkOp variants, AttachmentOp.owner_rel /
// file_name, SkipReason::Canvas / ComplexYaml / Unchanged — all unused until
// their respective batches.
#![allow(dead_code)]

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Serialize;

use crate::error::CliError;
use crate::output::{self, OutputFormat};

use super::manifest::Manifest;
use super::parser::VaultDoc;

// ---------------------------------------------------------------------------
// Plan op types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub(crate) struct FolderOp {
    pub rel_path: PathBuf,
    pub name: String,
    pub parent_rel: Option<PathBuf>,
}

#[derive(Debug, Serialize)]
pub(crate) enum DocAction {
    Create,
    Update { slug: String },
}

#[derive(Debug, Serialize)]
pub(crate) struct DocumentOp {
    pub rel_path: PathBuf,
    pub title: String,
    pub predicted_slug: String,
    pub folder_rel: Option<PathBuf>,
    /// Verbatim frontmatter block + rewritten body; the content sent to Atlas.
    pub content: String,
    pub action: DocAction,
    pub broken_links: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BoardOp {
    pub epic_rel: PathBuf,
    pub name: String,
    pub columns: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct TaskOp {
    pub rel_path: PathBuf,
    pub board_epic_rel: PathBuf,
    pub column: String,
    pub title: String,
    pub description: String,
    pub depends: Vec<String>,
}

#[derive(Debug, Serialize)]
pub(crate) enum LinkOp {
    Docs {
        task_rel: PathBuf,
        source_doc_rel: PathBuf,
    },
    Parent {
        child_rel: PathBuf,
        parent_rel: PathBuf,
    },
}

#[derive(Debug, Serialize)]
pub(crate) struct AttachmentOp {
    pub owner_rel: PathBuf,
    pub file_path: PathBuf,
    pub file_name: String,
    pub content_type: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct SkippedOp {
    pub rel_path: PathBuf,
    pub reason: SkipReason,
}

#[derive(Debug, Serialize)]
pub(crate) enum SkipReason {
    UnsupportedEmbedMd,
    Canvas,
    Dataview,
    InlineField,
    ComplexYaml,
    Unchanged,
}

/// The single shared model produced by the pure scan phase and consumed by
/// both `--dry-run` preview and the execution phase.
#[derive(Debug, Serialize)]
pub(crate) struct ImportPlan {
    pub folders: Vec<FolderOp>,
    pub documents: Vec<DocumentOp>,
    pub boards: Vec<BoardOp>,
    pub tasks: Vec<TaskOp>,
    pub links: Vec<LinkOp>,
    pub attachments: Vec<AttachmentOp>,
    pub skipped: Vec<SkippedOp>,
}

// ---------------------------------------------------------------------------
// build_plan
// ---------------------------------------------------------------------------

/// Builds an `ImportPlan` from a vault scan and the current manifest.
///
/// Pure — performs no API calls, no filesystem writes. Folders are sorted
/// topologically (parent before child) so the execute phase can create them
/// depth-first. Document action is predicted from the manifest: a manifest
/// entry for the rel_path predicts `Update`; otherwise `Create`.
///
/// Caveat: a doc imported outside this manifest shows `[CREATE]` in dry-run
/// but executes as an update (execution always calls `get_document` first).
/// This honours "dry-run performs zero API calls" while staying truthful.
pub(crate) fn build_plan(docs: &[VaultDoc], manifest: &Manifest) -> ImportPlan {
    let link_index = build_link_index(docs);
    let folders = collect_folders(docs);

    let mut documents: Vec<DocumentOp> = Vec::with_capacity(docs.len());
    let mut attachments: Vec<AttachmentOp> = Vec::new();
    let mut skipped: Vec<SkippedOp> = Vec::new();

    for doc in docs {
        let folder_rel = doc
            .rel_path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .map(PathBuf::from);

        let (resolved_body, broken_links) = resolve_links(&doc.body, &link_index);

        let content = match &doc.yaml_block {
            Some(yaml) => format!("---\n{yaml}\n---\n{resolved_body}"),
            None => resolved_body,
        };

        let action = {
            let key = doc.rel_path.to_string_lossy();
            match manifest.documents.get(key.as_ref()) {
                Some(entry) => DocAction::Update {
                    slug: entry.slug.clone(),
                },
                None => DocAction::Create,
            }
        };

        for (file_path, mime) in &doc.attachment_candidates {
            let file_name = file_path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            attachments.push(AttachmentOp {
                owner_rel: doc.rel_path.clone(),
                file_path: file_path.clone(),
                file_name,
                content_type: (*mime).to_string(),
            });
        }

        for embed in &doc.unsupported_embeds {
            skipped.push(SkippedOp {
                rel_path: embed.clone(),
                reason: SkipReason::UnsupportedEmbedMd,
            });
        }

        if body_has_dataview(&doc.body) {
            skipped.push(SkippedOp {
                rel_path: doc.rel_path.clone(),
                reason: SkipReason::Dataview,
            });
        }

        if body_has_inline_field(&doc.body) {
            skipped.push(SkippedOp {
                rel_path: doc.rel_path.clone(),
                reason: SkipReason::InlineField,
            });
        }

        documents.push(DocumentOp {
            rel_path: doc.rel_path.clone(),
            title: doc.title.clone(),
            predicted_slug: doc.predicted_slug.clone(),
            folder_rel,
            content,
            action,
            broken_links,
        });
    }

    let (boards, tasks, links) = super::mapping::build_ops(docs);

    ImportPlan {
        folders,
        documents,
        boards,
        tasks,
        links,
        attachments,
        skipped,
    }
}

/// Collects unique vault directory rel-paths from `docs`, sorted so parents
/// always precede their children.
///
/// The topological ordering (by component depth, then lexicographic) is
/// load-bearing for the execute phase: parent folders must exist before child
/// folders can reference their UUIDs.
fn collect_folders(docs: &[VaultDoc]) -> Vec<FolderOp> {
    use std::collections::BTreeSet;

    let mut dir_set: BTreeSet<PathBuf> = BTreeSet::new();

    for doc in docs {
        let mut cursor = doc.rel_path.parent().map(PathBuf::from);

        while let Some(dir) = cursor {
            if dir.as_os_str().is_empty() {
                break;
            }
            if !dir_set.insert(dir.clone()) {
                break;
            }
            cursor = dir.parent().map(PathBuf::from);
        }
    }

    let mut dirs: Vec<PathBuf> = dir_set.into_iter().collect();

    dirs.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });

    dirs.into_iter()
        .map(|dir| {
            let name = dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("")
                .to_string();

            let parent_rel = dir
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map(PathBuf::from);

            FolderOp {
                rel_path: dir,
                name,
                parent_rel,
            }
        })
        .collect()
}

/// Builds a vault-wide link resolution index that maps a normalized key to
/// the target doc's title.
///
/// Obsidian links by filename but Atlas slugs by title, so resolve filename→title.
/// Each doc contributes two keys: `slugify(filename_stem)` and `slugify(title)`,
/// both pointing to the doc's title. On a key collision, the first doc in the
/// slice wins (callers pass docs sorted by `rel_path`).
fn build_link_index(docs: &[VaultDoc]) -> HashMap<String, String> {
    let mut index: HashMap<String, String> = HashMap::new();

    for doc in docs {
        let stem = doc
            .rel_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("");

        let filename_key = atlas_domain::slugify(stem);
        index
            .entry(filename_key)
            .or_insert_with(|| doc.title.clone());

        let title_key = atlas_domain::slugify(&doc.title);
        index.entry(title_key).or_insert_with(|| doc.title.clone());
    }

    index
}

/// Resolves `[[target]]` wikilinks in `body` against a vault-wide index.
///
/// Obsidian links reference notes by their filename, but Atlas derives the
/// document slug from the title. This function maps each link target through
/// the index (which covers both filename-slugs and title-slugs) so that
/// `[[rendering-pipeline]]` becomes `[[Graphics Rendering Pipeline]]` when
/// that doc's title is "Graphics Rendering Pipeline".
///
/// Returns the rewritten body and the list of unresolved target strings.
pub(crate) fn resolve_links(body: &str, index: &HashMap<String, String>) -> (String, Vec<String>) {
    let mut out = String::with_capacity(body.len());
    let mut broken: Vec<String> = Vec::new();
    let mut remaining = body;

    while let Some(open) = remaining.find("[[") {
        out.push_str(&remaining[..open]);
        remaining = &remaining[open + 2..];

        let Some(close) = remaining.find("]]") else {
            out.push_str("[[");
            continue;
        };

        let target = remaining[..close].trim();
        remaining = &remaining[close + 2..];

        let key = atlas_domain::slugify(target);

        match index.get(&key) {
            Some(resolved_title) => {
                out.push_str("[[");
                out.push_str(resolved_title);
                out.push_str("]]");
            }
            None => {
                out.push_str("[[");
                out.push_str(target);
                out.push_str("]]");
                broken.push(target.to_string());
            }
        }
    }

    out.push_str(remaining);
    (out, broken)
}

fn body_has_dataview(body: &str) -> bool {
    body.contains("```dataview")
}

/// Returns `true` when `body` contains an Obsidian inline field (`key:: value`).
///
/// An inline field is a non-empty key followed by `:: ` (double colon space)
/// on the same line. Headings (lines starting with `#`) are excluded.
fn body_has_inline_field(body: &str) -> bool {
    body.lines().any(|line| {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            return false;
        }
        if let Some(pos) = trimmed.find(":: ") {
            pos > 0
        } else {
            let ends = trimmed.ends_with("::");
            ends && trimmed.len() > 2
        }
    })
}

// ---------------------------------------------------------------------------
// print_plan
// ---------------------------------------------------------------------------

/// Prints `plan` to stdout in the active output format.
///
/// Human format: a flat table grouped by op kind using the canonical line-kind
/// tags. JSON format: the full `ImportPlan` serialized as pretty JSON.
pub(crate) fn print_plan(plan: &ImportPlan, output: OutputFormat) -> Result<(), CliError> {
    if output == OutputFormat::Json {
        return output::print_json(plan);
    }

    let mut rows: Vec<Vec<String>> = Vec::new();

    for op in &plan.folders {
        rows.push(vec![
            "[FOLDER]".to_string(),
            op.rel_path.display().to_string(),
            op.name.clone(),
        ]);
    }

    for op in &plan.documents {
        let (kind, suffix) = match &op.action {
            DocAction::Create => ("[CREATE]", String::new()),
            DocAction::Update { slug } => ("[UPDATE]", format!(" → {slug}")),
        };
        rows.push(vec![
            kind.to_string(),
            op.rel_path.display().to_string(),
            format!("{}{suffix}", op.title),
        ]);
        for link in &op.broken_links {
            rows.push(vec![
                "[BROKEN_LINK]".to_string(),
                op.rel_path.display().to_string(),
                link.clone(),
            ]);
        }
    }

    for op in &plan.boards {
        rows.push(vec![
            "[BOARD_CREATE]".to_string(),
            op.epic_rel.display().to_string(),
            op.name.clone(),
        ]);
    }

    for op in &plan.tasks {
        rows.push(vec![
            "[TASK_CREATE]".to_string(),
            op.rel_path.display().to_string(),
            op.title.clone(),
        ]);
    }

    for op in &plan.skipped {
        let kind = match op.reason {
            SkipReason::Unchanged => "[SKIP]",
            _ => "[UNSUPPORTED]",
        };
        rows.push(vec![
            kind.to_string(),
            op.rel_path.display().to_string(),
            skip_reason_label(&op.reason).to_string(),
        ]);
    }

    if rows.is_empty() {
        println!("Nothing to import.");
        return Ok(());
    }

    output::print_table(&["Kind", "Path", "Detail"], rows)
}

fn skip_reason_label(reason: &SkipReason) -> &'static str {
    match reason {
        SkipReason::UnsupportedEmbedMd => "unsupported-embed-md",
        SkipReason::Canvas => "canvas",
        SkipReason::Dataview => "dataview",
        SkipReason::InlineField => "inline-field",
        SkipReason::ComplexYaml => "complex-yaml",
        SkipReason::Unchanged => "unchanged",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use tempfile::TempDir;

    use atlas_domain::slugify;

    use crate::commands::import::obsidian::frontmatter::parse_import_frontmatter;
    use crate::commands::import::obsidian::manifest::{Manifest, ManifestDocEntry};
    use crate::commands::import::obsidian::parser::{VaultDoc, scan_vault};

    use super::*;

    // -- helpers ---------------------------------------------------------------

    fn make_doc(rel_path: &str, title: &str, body: &str) -> VaultDoc {
        VaultDoc {
            rel_path: PathBuf::from(rel_path),
            title: title.to_string(),
            predicted_slug: slugify(title),
            raw_content: body.to_string(),
            yaml_block: None,
            body: body.to_string(),
            frontmatter: parse_import_frontmatter(""),
            wikilink_targets: atlas_domain::parse_wikilinks(body),
            attachment_candidates: vec![],
            unsupported_embeds: vec![],
        }
    }

    fn make_file(dir: &std::path::Path, rel: &str, content: &str) {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    // -- build_plan: basic action prediction -----------------------------------

    #[test]
    fn build_plan_create_for_new_file() {
        let doc = make_doc("notes/a.md", "Alpha", "Hello world.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert_eq!(plan.documents.len(), 1);
        assert!(
            matches!(plan.documents[0].action, DocAction::Create),
            "doc absent from manifest must be Create"
        );
    }

    #[test]
    fn build_plan_update_from_manifest() {
        let doc = make_doc("notes/a.md", "Alpha", "Hello world.");
        let mut manifest = Manifest::empty();
        manifest.documents.insert(
            "notes/a.md".to_string(),
            ManifestDocEntry {
                slug: "alpha".to_string(),
                id: "some-uuid".to_string(),
                content_hash: String::new(),
            },
        );

        let plan = build_plan(&[doc], &manifest);

        assert_eq!(plan.documents.len(), 1);
        match &plan.documents[0].action {
            DocAction::Update { slug } => assert_eq!(slug, "alpha"),
            DocAction::Create => panic!("doc present in manifest must be Update"),
        }
    }

    // -- build_plan: broken links ----------------------------------------------

    #[test]
    fn build_plan_broken_link_detected() {
        let doc = make_doc("a.md", "Alpha", "See [[Ghost]] for details.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert_eq!(plan.documents.len(), 1);
        assert!(
            plan.documents[0]
                .broken_links
                .contains(&"Ghost".to_string()),
            "wikilink target absent from vault must be a broken link"
        );
    }

    #[test]
    fn build_plan_no_broken_link_when_target_in_vault() {
        let alpha = make_doc("alpha.md", "Alpha", "See [[Beta]] for details.");
        let beta = make_doc("beta.md", "Beta", "Hello.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[alpha, beta], &manifest);

        assert!(
            plan.documents[0].broken_links.is_empty(),
            "link to a doc in the vault must not be broken"
        );
    }

    // -- build_plan: skipped ops -----------------------------------------------

    #[test]
    fn build_plan_unsupported_md_embed_in_skipped() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "doc.md", "Transclude: ![[other.md]]");

        let docs = scan_vault(tmp.path()).unwrap();
        let manifest = Manifest::empty();

        let plan = build_plan(&docs, &manifest);

        assert!(
            plan.skipped
                .iter()
                .any(|s| matches!(s.reason, SkipReason::UnsupportedEmbedMd)),
            "unsupported md embed must appear in skipped"
        );
    }

    #[test]
    fn build_plan_dataview_block_in_skipped() {
        let doc = make_doc(
            "a.md",
            "Alpha",
            "Some text.\n```dataview\nLIST\n```\nMore text.",
        );
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert!(
            plan.skipped
                .iter()
                .any(|s| matches!(s.reason, SkipReason::Dataview)),
            "doc with dataview block must be reported as unsupported"
        );
    }

    #[test]
    fn build_plan_inline_field_in_skipped() {
        let doc = make_doc("a.md", "Alpha", "Some text.\nrating:: 5\nMore text.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert!(
            plan.skipped
                .iter()
                .any(|s| matches!(s.reason, SkipReason::InlineField)),
            "doc with inline field must be reported as unsupported"
        );
    }

    // -- build_plan: folder ordering -------------------------------------------

    #[test]
    fn build_plan_folders_parent_before_child() {
        let tmp = TempDir::new().unwrap();
        make_file(tmp.path(), "sub/b.md", "# B");
        make_file(tmp.path(), "sub/deep/a.md", "# A");

        let docs = scan_vault(tmp.path()).unwrap();
        let manifest = Manifest::empty();

        let plan = build_plan(&docs, &manifest);

        let positions: Vec<usize> = ["sub", "sub/deep"]
            .iter()
            .map(|expected| {
                plan.folders
                    .iter()
                    .position(|f| f.rel_path == PathBuf::from(expected))
                    .unwrap_or_else(|| panic!("folder {expected} not found in plan"))
            })
            .collect();

        assert!(
            positions[0] < positions[1],
            "parent folder 'sub' must appear before child 'sub/deep'"
        );
    }

    #[test]
    fn build_plan_root_docs_produce_no_folders() {
        let doc = make_doc("a.md", "Alpha", "Hello.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert!(
            plan.folders.is_empty(),
            "documents at vault root must not produce folder ops"
        );
    }

    // -- body_has_dataview / body_has_inline_field unit tests ------------------

    #[test]
    fn dataview_detected_in_code_fence() {
        assert!(body_has_dataview("```dataview\nLIST\n```"));
    }

    #[test]
    fn dataview_not_detected_in_plain_text() {
        assert!(!body_has_dataview("just some text about dataview concepts"));
    }

    #[test]
    fn inline_field_detected() {
        assert!(body_has_inline_field("rating:: 5"));
    }

    #[test]
    fn inline_field_not_detected_in_heading() {
        assert!(!body_has_inline_field("# heading:: not a field"));
    }

    #[test]
    fn inline_field_not_detected_in_single_colon() {
        assert!(!body_has_inline_field("key: value"));
    }

    // -- resolve_links ---------------------------------------------------------

    #[test]
    fn resolve_links_filename_link_rewrites_to_title() {
        // A doc filed as "rendering-pipeline.md" has title "Graphics Rendering Pipeline".
        // A link [[rendering-pipeline]] should be rewritten to [[Graphics Rendering Pipeline]].
        let mut index = HashMap::new();
        index.insert(
            "rendering-pipeline".to_string(),
            "Graphics Rendering Pipeline".to_string(),
        );

        let (resolved, broken) = resolve_links("See [[rendering-pipeline]] for details.", &index);

        assert_eq!(resolved, "See [[Graphics Rendering Pipeline]] for details.");
        assert!(broken.is_empty());
    }

    #[test]
    fn resolve_links_title_link_resolves_idempotently() {
        // A link using the exact title also resolves (title key is in the index too).
        let mut index = HashMap::new();
        index.insert(
            "graphics-rendering-pipeline".to_string(),
            "Graphics Rendering Pipeline".to_string(),
        );

        let (resolved, broken) =
            resolve_links("See [[Graphics Rendering Pipeline]] for details.", &index);

        assert_eq!(resolved, "See [[Graphics Rendering Pipeline]] for details.");
        assert!(broken.is_empty());
    }

    #[test]
    fn resolve_links_missing_target_left_verbatim_and_in_broken() {
        let index = HashMap::new();

        let (resolved, broken) = resolve_links("See [[Ghost]] for details.", &index);

        assert_eq!(resolved, "See [[Ghost]] for details.");
        assert!(broken.contains(&"Ghost".to_string()));
    }

    // -- build_plan: filename≠title resolution --------------------------------

    #[test]
    fn build_plan_resolves_filename_link_to_title() {
        // B's filename is "rendering-pipeline.md" but its title is "Graphics Rendering Pipeline".
        // A links to B using the filename [[rendering-pipeline]].
        // After build_plan, A's content must contain the title form and broken_links must be empty.
        let doc_b = make_doc(
            "rendering-pipeline.md",
            "Graphics Rendering Pipeline",
            "This is the rendering pipeline.",
        );
        let doc_a = make_doc("a.md", "A Doc", "See [[rendering-pipeline]] for details.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc_a, doc_b], &manifest);

        let op_a = plan
            .documents
            .iter()
            .find(|d| d.rel_path.to_str() == Some("a.md"))
            .unwrap();

        assert!(
            op_a.broken_links.is_empty(),
            "link to a vault doc by its filename must not be broken"
        );
        assert!(
            op_a.content.contains("[[Graphics Rendering Pipeline]]"),
            "content must contain the resolved title in the link"
        );
    }

    // -- build_plan: collision determinism ------------------------------------

    #[test]
    fn build_plan_collision_first_by_rel_path_wins_no_panic() {
        // "a.md" and "sub/a.md" both yield filename key slugify("a") = "a".
        // The first in the slice wins; a third doc links to [[a]] and must
        // resolve to that first winner's title without panicking.
        let doc_first = make_doc("a.md", "Alpha First", "Hello.");
        let doc_second = make_doc("sub/a.md", "Alpha Second", "World.");
        let doc_linker = make_doc("linker.md", "Linker", "See [[a]] for details.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc_first, doc_second, doc_linker], &manifest);

        assert_eq!(plan.documents.len(), 3);

        let op = plan
            .documents
            .iter()
            .find(|d| d.rel_path.to_str() == Some("linker.md"))
            .unwrap();

        assert!(
            op.broken_links.is_empty(),
            "link must resolve to first winner"
        );
        assert!(
            op.content.contains("[[Alpha First]]"),
            "first doc in rel_path order must win on filename key collision"
        );
    }

    // -- build_plan: genuinely missing target remains in broken_links ---------

    #[test]
    fn build_plan_genuinely_missing_target_in_broken_links() {
        let doc = make_doc("a.md", "A Doc", "See [[NonExistent]] here.");
        let manifest = Manifest::empty();

        let plan = build_plan(&[doc], &manifest);

        assert!(
            plan.documents[0]
                .broken_links
                .contains(&"NonExistent".to_string()),
            "link to a doc not in the vault must appear in broken_links"
        );
    }
}
