#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::path::PathBuf;

use super::parser::VaultDoc;
use super::plan::{BoardOp, LinkOp, TaskOp};

const ROADMAP_BOARD: &str = "Roadmap";
const COLUMN_TODO: &str = "To Do";
const COLUMN_IN_PROGRESS: &str = "In Progress";
const COLUMN_DONE: &str = "Done";

/// Maps a frontmatter `status` value to the corresponding column name on the
/// Roadmap board.
///
/// Matching is case-insensitive and covers both the English and the Spanish
/// vocabularies the real vault uses (`status: hecho` for done, `status: todo`
/// for pending). Anything unrecognized — or absent — falls back to "To Do".
pub(crate) fn map_status(status: Option<&str>) -> &'static str {
    let Some(raw) = status else {
        return COLUMN_TODO;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "done" | "hecho" | "completado" | "terminado" | "listo" => COLUMN_DONE,
        "in-progress" | "in_progress" | "doing" | "en-progreso" | "en_progreso" | "en progreso"
        | "en curso" | "haciendo" => COLUMN_IN_PROGRESS,
        _ => COLUMN_TODO,
    }
}

/// Returns the title to use for an epic's task: the document's first H1 heading
/// (e.g. `# E09 — CLI`) when present, otherwise the document title (filename
/// stem). The H1 matches how the existing Atlas roadmap names its epic tasks.
fn epic_task_title(doc: &VaultDoc) -> String {
    extract_h1(&doc.body).unwrap_or_else(|| doc.title.clone())
}

/// Extracts the text of the first level-1 ATX heading (`# ...`) in `body`.
fn extract_h1(body: &str) -> Option<String> {
    body.lines().find_map(|line| {
        let rest = line.strip_prefix("# ")?;
        let title = rest.trim();
        if title.is_empty() {
            None
        } else {
            Some(title.to_string())
        }
    })
}

/// Inspects vault documents for the `type: epic` convention and returns
/// board, task, and link operations derived from that convention.
///
/// Docs with `type: epic` become tasks on a single "Roadmap" board (column
/// from status). Each epic also gets a `docs` reference back to its source
/// document so the task and document stay linked. All other doc types are
/// ignored by this function.
pub(crate) fn build_ops(docs: &[VaultDoc]) -> (Vec<BoardOp>, Vec<TaskOp>, Vec<LinkOp>) {
    let epics: Vec<&VaultDoc> = docs
        .iter()
        .filter(|d| d.frontmatter.doc_type.as_deref() == Some("epic"))
        .collect();

    if epics.is_empty() {
        return (vec![], vec![], vec![]);
    }

    let board_op = BoardOp {
        epic_rel: PathBuf::from(ROADMAP_BOARD),
        name: ROADMAP_BOARD.to_string(),
        columns: vec![
            COLUMN_TODO.to_string(),
            COLUMN_IN_PROGRESS.to_string(),
            COLUMN_DONE.to_string(),
        ],
    };

    let mut task_ops: Vec<TaskOp> = Vec::with_capacity(epics.len());
    let mut link_ops: Vec<LinkOp> = Vec::with_capacity(epics.len());

    for doc in epics {
        task_ops.push(TaskOp {
            rel_path: doc.rel_path.clone(),
            board_epic_rel: PathBuf::from(ROADMAP_BOARD),
            column: map_status(doc.frontmatter.status.as_deref()).to_string(),
            title: epic_task_title(doc),
            description: String::new(),
            depends: doc.frontmatter.depends.clone(),
        });

        link_ops.push(LinkOp::Docs {
            task_rel: doc.rel_path.clone(),
            source_doc_rel: doc.rel_path.clone(),
        });
    }

    (vec![board_op], task_ops, link_ops)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use atlas_domain::slugify;

    use crate::commands::import::obsidian::frontmatter::parse_import_frontmatter;
    use crate::commands::import::obsidian::parser::VaultDoc;
    use crate::commands::import::obsidian::plan::LinkOp;

    use super::*;

    fn make_epic(rel_path: &str, title: &str, status: Option<&str>) -> VaultDoc {
        let yaml = match status {
            Some(s) => format!("type: epic\nstatus: {s}\n"),
            None => "type: epic\n".to_string(),
        };
        VaultDoc {
            rel_path: PathBuf::from(rel_path),
            title: title.to_string(),
            predicted_slug: slugify(title),
            raw_content: String::new(),
            yaml_block: None,
            body: String::new(),
            frontmatter: parse_import_frontmatter(&yaml),
            wikilink_targets: vec![],
            attachment_candidates: vec![],
            unsupported_embeds: vec![],
        }
    }

    fn make_doc_with_type(rel_path: &str, title: &str, doc_type: &str) -> VaultDoc {
        let yaml = format!("type: {doc_type}\n");
        VaultDoc {
            rel_path: PathBuf::from(rel_path),
            title: title.to_string(),
            predicted_slug: slugify(title),
            raw_content: String::new(),
            yaml_block: None,
            body: String::new(),
            frontmatter: parse_import_frontmatter(&yaml),
            wikilink_targets: vec![],
            attachment_candidates: vec![],
            unsupported_embeds: vec![],
        }
    }

    fn make_plain_doc(rel_path: &str, title: &str) -> VaultDoc {
        VaultDoc {
            rel_path: PathBuf::from(rel_path),
            title: title.to_string(),
            predicted_slug: slugify(title),
            raw_content: String::new(),
            yaml_block: None,
            body: String::new(),
            frontmatter: parse_import_frontmatter(""),
            wikilink_targets: vec![],
            attachment_candidates: vec![],
            unsupported_embeds: vec![],
        }
    }

    // -- map_status -----------------------------------------------------------

    #[test]
    fn map_status_todo_returns_to_do() {
        assert_eq!(map_status(Some("todo")), "To Do");
    }

    #[test]
    fn map_status_in_progress_hyphen_returns_in_progress() {
        assert_eq!(map_status(Some("in-progress")), "In Progress");
    }

    #[test]
    fn map_status_in_progress_underscore_returns_in_progress() {
        assert_eq!(map_status(Some("in_progress")), "In Progress");
    }

    #[test]
    fn map_status_doing_returns_in_progress() {
        assert_eq!(map_status(Some("doing")), "In Progress");
    }

    #[test]
    fn map_status_done_returns_done() {
        assert_eq!(map_status(Some("done")), "Done");
    }

    #[test]
    fn map_status_none_returns_to_do() {
        assert_eq!(map_status(None), "To Do");
    }

    #[test]
    fn map_status_unknown_value_returns_to_do() {
        assert_eq!(map_status(Some("blocked")), "To Do");
    }

    #[test]
    fn map_status_spanish_hecho_returns_done() {
        assert_eq!(map_status(Some("hecho")), "Done");
    }

    #[test]
    fn map_status_is_case_insensitive() {
        assert_eq!(map_status(Some("HECHO")), "Done");
        assert_eq!(map_status(Some("Todo")), "To Do");
    }

    #[test]
    fn map_status_spanish_en_progreso_returns_in_progress() {
        assert_eq!(map_status(Some("en progreso")), "In Progress");
    }

    // -- epic task title (H1 heading) ----------------------------------------

    #[test]
    fn extract_h1_reads_first_heading() {
        assert_eq!(
            extract_h1("# E09 — CLI\n\nbody"),
            Some("E09 — CLI".to_string())
        );
    }

    #[test]
    fn extract_h1_absent_returns_none() {
        assert_eq!(extract_h1("no heading here"), None);
    }

    #[test]
    fn build_ops_epic_task_title_from_h1_heading() {
        let mut doc = make_epic("Atlas/E09-cli/E09-cli.md", "E09-cli", Some("todo"));
        doc.body = "# E09 — CLI\n\nthe epic body".to_string();
        let (_, tasks, _) = build_ops(&[doc]);
        assert_eq!(
            tasks[0].title, "E09 — CLI",
            "epic task title comes from the H1, not the filename"
        );
    }

    #[test]
    fn build_ops_epic_task_title_falls_back_to_filename_when_no_h1() {
        let docs = vec![make_epic("e1.md", "E1 Filename", Some("todo"))];
        let (_, tasks, _) = build_ops(&docs);
        assert_eq!(tasks[0].title, "E1 Filename");
    }

    // -- build_ops: no epics --------------------------------------------------

    #[test]
    fn build_ops_empty_docs_returns_empty() {
        let (boards, tasks, links) = build_ops(&[]);
        assert!(boards.is_empty());
        assert!(tasks.is_empty());
        assert!(links.is_empty());
    }

    #[test]
    fn build_ops_no_epics_returns_empty() {
        let docs = vec![
            make_plain_doc("a.md", "Alpha"),
            make_plain_doc("b.md", "Beta"),
        ];
        let (boards, tasks, links) = build_ops(&docs);
        assert!(boards.is_empty());
        assert!(tasks.is_empty());
        assert!(links.is_empty());
    }

    // -- build_ops: one epic --------------------------------------------------

    #[test]
    fn build_ops_one_epic_emits_one_board_one_task_one_link() {
        let docs = vec![make_epic("epics/e1.md", "Epic One", Some("todo"))];
        let (boards, tasks, links) = build_ops(&docs);

        assert_eq!(boards.len(), 1, "exactly one Roadmap board");
        assert_eq!(boards[0].name, "Roadmap");

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Epic One");
        assert_eq!(tasks[0].column, "To Do");
        assert_eq!(tasks[0].rel_path, PathBuf::from("epics/e1.md"));

        assert_eq!(links.len(), 1);
        match &links[0] {
            LinkOp::Docs {
                task_rel,
                source_doc_rel,
            } => {
                assert_eq!(task_rel, &PathBuf::from("epics/e1.md"));
                assert_eq!(source_doc_rel, &PathBuf::from("epics/e1.md"));
            }
            other => panic!("expected LinkOp::Docs, got {other:?}"),
        }
    }

    #[test]
    fn build_ops_one_epic_board_has_three_standard_columns() {
        let docs = vec![make_epic("e1.md", "E1", None)];
        let (boards, _, _) = build_ops(&docs);
        assert_eq!(
            boards[0].columns,
            vec!["To Do", "In Progress", "Done"],
            "Roadmap board must have the three standard columns"
        );
    }

    #[test]
    fn build_ops_one_epic_no_status_defaults_to_to_do() {
        let docs = vec![make_epic("e1.md", "E1", None)];
        let (_, tasks, _) = build_ops(&docs);
        assert_eq!(tasks[0].column, "To Do");
    }

    // -- build_ops: multiple epics with mixed statuses ------------------------

    #[test]
    fn build_ops_multiple_epics_emits_one_board_only() {
        let docs = vec![
            make_epic("e1.md", "E1", Some("todo")),
            make_epic("e2.md", "E2", Some("in-progress")),
            make_epic("e3.md", "E3", Some("done")),
        ];
        let (boards, tasks, links) = build_ops(&docs);

        assert_eq!(
            boards.len(),
            1,
            "still only one Roadmap board regardless of epic count"
        );
        assert_eq!(tasks.len(), 3);
        assert_eq!(links.len(), 3);
    }

    #[test]
    fn build_ops_multiple_epics_correct_columns() {
        let docs = vec![
            make_epic("e1.md", "E1", Some("todo")),
            make_epic("e2.md", "E2", Some("in-progress")),
            make_epic("e3.md", "E3", Some("done")),
            make_epic("e4.md", "E4", Some("doing")),
            make_epic("e5.md", "E5", Some("in_progress")),
        ];
        let (_, tasks, _) = build_ops(&docs);

        assert_eq!(tasks[0].column, "To Do");
        assert_eq!(tasks[1].column, "In Progress");
        assert_eq!(tasks[2].column, "Done");
        assert_eq!(tasks[3].column, "In Progress");
        assert_eq!(tasks[4].column, "In Progress");
    }

    // -- build_ops: non-epic docs ignored ------------------------------------

    #[test]
    fn build_ops_non_epic_docs_do_not_produce_tasks() {
        let docs = vec![
            make_epic("e1.md", "Real Epic", Some("todo")),
            make_plain_doc("note.md", "Plain Note"),
        ];
        let (_, tasks, _) = build_ops(&docs);

        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].title, "Real Epic");
    }

    #[test]
    fn build_ops_type_tasks_does_not_produce_task() {
        let docs = vec![make_doc_with_type("tasks.md", "Tasks Doc", "tasks")];
        let (boards, tasks, links) = build_ops(&docs);
        assert!(boards.is_empty());
        assert!(tasks.is_empty());
        assert!(links.is_empty());
    }

    #[test]
    fn build_ops_type_proposal_does_not_produce_task() {
        let docs = vec![make_doc_with_type(
            "proposal.md",
            "Proposal Doc",
            "proposal",
        )];
        let (boards, tasks, links) = build_ops(&docs);
        assert!(boards.is_empty());
        assert!(tasks.is_empty());
        assert!(links.is_empty());
    }
}
