#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub(crate) mod plan;
pub(crate) mod render;
pub(crate) mod write;

use std::collections::{HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};

use clap::Parser;
use uuid::Uuid;

use atlas_api::pagination::Page;
use atlas_client::ClientError;

use crate::commands::import::obsidian::read_yes;
use crate::ctx::Ctx;
use crate::error::CliError;

use plan::{DirOp, ExportPlan, FileOp};
use render::{deuid_links, frontmatter_to_yaml, render_doc, safe_filename};

/// Arguments for `atlas export obsidian`.
#[derive(Parser)]
pub(crate) struct ObsidianExportArgs {
    /// Workspace slug (uses the configured default when omitted).
    #[arg(long)]
    pub workspace: Option<String>,

    /// Source project slug.
    #[arg(long)]
    pub project: String,

    /// Path to write the Obsidian vault to.
    #[arg(index = 1)]
    pub path: PathBuf,

    /// Preview what would be exported without writing to disk.
    #[arg(long)]
    pub dry_run: bool,

    /// Write into an existing non-empty target directory without asking.
    #[arg(long)]
    pub force: bool,
}

/// Entry point for `atlas export obsidian`.
///
/// Flow: (1) resolve workspace; (2) verify the project exists; (3) read the
/// full project structure from Atlas (folders, documents, boards, tasks);
/// (4) build the `ExportPlan`; (5) if `--dry-run`, print the plan and return;
/// (6) otherwise materialize the plan on disk.
pub(crate) async fn run_obsidian(ctx: &Ctx, args: ObsidianExportArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    match ctx.client.get_project(ws, &args.project).await {
        Ok(_) => {}
        Err(ClientError::Api(p)) if p.status == 404 => {
            return Err(CliError::Validation(format!(
                "project '{}' does not exist",
                args.project
            )));
        }
        Err(e) => return Err(CliError::from(e)),
    }

    let plan = build_export_plan(&ctx.client, ws, &args.project).await?;

    if args.dry_run {
        println!(
            "[DRY-RUN] Would write {} director{} and {} file{}.",
            plan.dirs.len(),
            if plan.dirs.len() == 1 { "y" } else { "ies" },
            plan.files.len(),
            if plan.files.len() == 1 { "" } else { "s" },
        );
        for dir in &plan.dirs {
            println!("[DIR]  {}", dir.rel_path.display());
        }
        for file in &plan.files {
            println!("[FILE] {}", file.rel_path.display());
        }
        return Ok(());
    }

    if !args.force && dir_exists_nonempty(&args.path)? {
        eprint!(
            "Target '{}' already exists and is not empty — files with matching names will be \
             overwritten. Proceed? [y/N] ",
            args.path.display()
        );
        std::io::stderr().flush().ok();
        if !read_yes(std::io::stdin().lock()) {
            eprintln!("Export aborted.");
            return Ok(());
        }
    }

    write::materialize(&plan, &args.path)?;

    println!(
        "Exported {} director{} and {} file{} to {}.",
        plan.dirs.len(),
        if plan.dirs.len() == 1 { "y" } else { "ies" },
        plan.files.len(),
        if plan.files.len() == 1 { "" } else { "s" },
        args.path.display(),
    );

    Ok(())
}

/// Builds the complete export plan by reading all project content from Atlas.
async fn build_export_plan(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<ExportPlan, CliError> {
    let mut plan = ExportPlan::new();

    let folders = paginate_folders(client, ws, project).await?;
    let folder_paths = compute_folder_paths(&folders);

    for path in folder_paths.values() {
        plan.dirs.push(DirOp {
            rel_path: path.clone(),
        });
    }

    let doc_summaries = paginate_documents(client, ws, project).await?;
    let mut used_filenames: HashMap<PathBuf, HashSet<String>> = HashMap::new();

    for summary in &doc_summaries {
        let slug = summary.slug.as_deref().unwrap_or("");
        let doc = client.get_document(ws, slug).await?;

        let dir = match doc.folder_id.and_then(|id| folder_paths.get(&id)) {
            Some(p) => p.clone(),
            None => PathBuf::new(),
        };

        let names = used_filenames.entry(dir.clone()).or_default();
        let filename = unique_filename(&doc.title, slug, names);
        names.insert(filename.clone());

        let rel_path = if dir.as_os_str().is_empty() {
            PathBuf::from(&filename)
        } else {
            dir.join(&filename)
        };

        let content = render_doc(&doc.title, &doc.frontmatter, &doc.content);

        plan.files.push(FileOp { rel_path, content });
    }

    let boards = paginate_boards(client, ws, project).await?;
    let mut boards_dir_added = false;

    for board in &boards {
        let board_dir = PathBuf::from("boards").join(sanitize_name(&board.name));
        let task_files = collect_board_task_files(client, ws, board, &board_dir).await?;

        if !task_files.is_empty() {
            if !boards_dir_added {
                plan.dirs.push(DirOp {
                    rel_path: PathBuf::from("boards"),
                });
                boards_dir_added = true;
            }

            plan.dirs.push(DirOp {
                rel_path: board_dir,
            });

            plan.files.extend(task_files);
        }
    }

    Ok(plan)
}

/// Returns `true` when a task should be written as a standalone file.
///
/// Tasks backed by a document (identified by a "docs" outbound reference) are
/// already exported via their source document's file; writing them again would
/// produce duplicate content that re-import cannot de-duplicate.
pub(crate) fn task_needs_file(ref_kinds: &[&str]) -> bool {
    !ref_kinds.contains(&"docs")
}

/// Fetches tasks for one board and builds the `FileOp` list for tasks that are
/// not backed by a source document.
///
/// Each task's outbound references are inspected: if any reference has
/// `kind == "docs"` the task is skipped (its source document file already
/// carries the content). Only Atlas-native tasks without a document backing
/// are materialised as standalone files under `board_dir`.
async fn collect_board_task_files(
    client: &atlas_client::AtlasClient,
    ws: &str,
    board: &atlas_api::dtos::boards_tasks::BoardSummaryDto,
    board_dir: &Path,
) -> Result<Vec<FileOp>, CliError> {
    let columns = client.list_columns(ws, board.id).await?;
    let col_by_id: HashMap<Uuid, String> = columns.iter().map(|c| (c.id, c.name.clone())).collect();

    let tasks = paginate_tasks(client, ws, board.id).await?;
    let mut task_names: HashSet<String> = HashSet::new();
    let mut files: Vec<FileOp> = Vec::new();

    for task_summary in &tasks {
        let refs = client
            .list_references(ws, &task_summary.readable_id)
            .await?;
        let ref_kinds: Vec<&str> = refs
            .iter()
            .filter_map(|reference| reference.manual_kind.as_deref())
            .collect();

        if !task_needs_file(&ref_kinds) {
            continue;
        }

        let depends: Vec<String> = refs
            .iter()
            .filter(|r| r.manual_kind.as_deref() == Some("parent"))
            .filter_map(|r| r.target_readable_id.clone())
            .collect();

        let status = col_by_id
            .get(&task_summary.column_id)
            .cloned()
            .unwrap_or_else(|| task_summary.column_name.clone());

        let mut fm_map = serde_json::Map::new();
        fm_map.insert(
            "type".to_string(),
            serde_json::Value::String("task".to_string()),
        );
        fm_map.insert("status".to_string(), serde_json::Value::String(status));

        if !depends.is_empty() {
            fm_map.insert(
                "depends".to_string(),
                serde_json::Value::Array(
                    depends.into_iter().map(serde_json::Value::String).collect(),
                ),
            );
        }

        let task_fm = serde_json::Value::Object(fm_map);
        let task_slug = sanitize_name(&task_summary.readable_id);
        let task_filename = unique_filename(&task_summary.title, &task_slug, &task_names);
        task_names.insert(task_filename.clone());

        let task = client.get_task(ws, &task_summary.readable_id).await?;
        let content = render_task_content(&task_fm, &task.description);

        files.push(FileOp {
            rel_path: board_dir.join(&task_filename),
            content,
        });
    }

    Ok(files)
}

/// Renders a standalone task file: YAML frontmatter followed by the task's
/// description (empty body when the task has no description).
fn render_task_content(task_fm: &serde_json::Value, description: &str) -> String {
    format!(
        "{}{}",
        frontmatter_to_yaml(task_fm),
        deuid_links(description)
    )
}

/// Returns `true` when `path` is an existing directory containing at least one
/// entry, i.e. an export would write into pre-existing content.
fn dir_exists_nonempty(path: &Path) -> Result<bool, CliError> {
    if !path.is_dir() {
        return Ok(false);
    }

    Ok(std::fs::read_dir(path)?.next().is_some())
}

/// Page size used when draining paginated list endpoints during export.
const EXPORT_PAGE_SIZE: u32 = 100;

/// Drains a paginated list endpoint by repeatedly calling `fetch` with the
/// previous page's cursor until `has_more` is false, collecting all items.
async fn paginate_all<T, F, Fut>(mut fetch: F) -> Result<Vec<T>, CliError>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<Page<T>, ClientError>>,
{
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = fetch(cursor).await?;
        all.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(all)
}

/// Paginates `list_folders` until exhausted, returning all folder DTOs.
async fn paginate_folders(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::folders::FolderDto>, CliError> {
    paginate_all(|cursor| async move {
        client
            .list_folders(ws, project, cursor.as_deref(), Some(EXPORT_PAGE_SIZE))
            .await
    })
    .await
}

/// Paginates `list_documents` until exhausted.
async fn paginate_documents(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::documents::DocumentSummaryDto>, CliError> {
    paginate_all(|cursor| async move {
        client
            .list_documents(ws, project, cursor.as_deref(), Some(EXPORT_PAGE_SIZE))
            .await
    })
    .await
}

/// Paginates `list_boards` until exhausted.
async fn paginate_boards(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::boards_tasks::BoardSummaryDto>, CliError> {
    paginate_all(|cursor| async move {
        client
            .list_boards(ws, project, cursor.as_deref(), Some(EXPORT_PAGE_SIZE))
            .await
    })
    .await
}

/// Paginates `list_tasks` for a given board until exhausted.
async fn paginate_tasks(
    client: &atlas_client::AtlasClient,
    ws: &str,
    board_id: Uuid,
) -> Result<Vec<atlas_api::dtos::boards_tasks::TaskSummaryDto>, CliError> {
    paginate_all(|cursor| async move {
        client
            .list_tasks(ws, board_id, cursor.as_deref(), Some(EXPORT_PAGE_SIZE))
            .await
    })
    .await
}

/// Computes relative filesystem paths for all folders by walking parent chains.
fn compute_folder_paths(folders: &[atlas_api::dtos::folders::FolderDto]) -> HashMap<Uuid, PathBuf> {
    let by_id: HashMap<Uuid, &atlas_api::dtos::folders::FolderDto> =
        folders.iter().map(|f| (f.id, f)).collect();
    let mut paths: HashMap<Uuid, PathBuf> = HashMap::new();

    for folder in folders {
        resolve_folder_path(folder.id, &by_id, &mut paths, 0);
    }

    paths
}

fn resolve_folder_path(
    id: Uuid,
    by_id: &HashMap<Uuid, &atlas_api::dtos::folders::FolderDto>,
    paths: &mut HashMap<Uuid, PathBuf>,
    depth: usize,
) {
    if paths.contains_key(&id) || depth > 64 {
        return;
    }

    let folder = match by_id.get(&id) {
        Some(f) => f,
        None => return,
    };

    let component = safe_dir_component(&folder.name);

    let path = match folder.parent_folder_id {
        None => PathBuf::from(&component),
        Some(parent_id) => {
            resolve_folder_path(parent_id, by_id, paths, depth + 1);
            match paths.get(&parent_id) {
                Some(parent_path) => parent_path.join(&component),
                None => PathBuf::from(&component),
            }
        }
    };

    paths.insert(id, path);
}

/// Sanitizes a server-supplied folder name into a single safe path component.
///
/// The server allows `/` and `..` in folder names, which would otherwise
/// introduce extra path components or parent-directory traversal when joined
/// onto the export root. Names that sanitize to nothing usable (empty or
/// dots-only, e.g. `..`) fall back to a fixed placeholder.
fn safe_dir_component(name: &str) -> String {
    let sanitized = sanitize_name(name);

    if sanitized.is_empty() || sanitized.chars().all(|c| c == '.') {
        "unnamed".to_string()
    } else {
        sanitized
    }
}

/// Returns a filename that is not already in `used`, appending the slug
/// to disambiguate when the sanitized title collides.
fn unique_filename(title: &str, slug: &str, used: &HashSet<String>) -> String {
    let base = safe_filename(title, slug);

    if !used.contains(&base) {
        return base;
    }

    let stem = base.strip_suffix(".md").unwrap_or(&base);
    let candidate = format!("{stem}-{slug}.md");
    if !used.contains(&candidate) {
        return candidate;
    }

    let mut n: u32 = 2;
    loop {
        let candidate = format!("{stem}-{n}.md");
        if !used.contains(&candidate) {
            return candidate;
        }
        n = n.saturating_add(1);
    }
}

/// Produces a filesystem-safe name by replacing characters invalid in most
/// operating systems with a dash and collapsing consecutive dashes.
fn sanitize_name(s: &str) -> String {
    let raw: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();

    let mut out = String::with_capacity(raw.len());
    let mut prev_dash = false;

    for c in raw.chars() {
        if c == '-' {
            if !prev_dash {
                out.push(c);
            }
            prev_dash = true;
        } else {
            out.push(c);
            prev_dash = false;
        }
    }

    out.trim_matches('-').to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Commands;
    use clap::Parser as ClapParser;

    #[derive(ClapParser)]
    struct Cli {
        #[command(subcommand)]
        command: Commands,
    }

    #[test]
    fn obsidian_export_parses_required_args() {
        let cli = Cli::try_parse_from([
            "atlas",
            "export",
            "obsidian",
            "--project",
            "my-project",
            "/tmp/vault",
        ])
        .unwrap();
        let Commands::Export(args) = cli.command else {
            panic!("expected Export command");
        };
        let super::super::ExportCmd::Obsidian(obs) = args.command;
        assert_eq!(obs.project, "my-project");
        assert_eq!(obs.path, PathBuf::from("/tmp/vault"));
        assert!(!obs.dry_run);
    }

    #[test]
    fn obsidian_export_parses_optional_flags() {
        let cli = Cli::try_parse_from([
            "atlas",
            "export",
            "obsidian",
            "--workspace",
            "ws",
            "--project",
            "p",
            "--dry-run",
            "/vault",
        ])
        .unwrap();
        let Commands::Export(args) = cli.command else {
            panic!("expected Export command");
        };
        let super::super::ExportCmd::Obsidian(obs) = args.command;
        assert_eq!(obs.workspace.as_deref(), Some("ws"));
        assert!(obs.dry_run);
    }

    #[test]
    fn obsidian_export_requires_project() {
        let result = Cli::try_parse_from(["atlas", "export", "obsidian", "/tmp/vault"]);
        assert!(
            result.is_err(),
            "export obsidian without --project must fail"
        );
    }

    // -- task_needs_file -------------------------------------------------------

    #[test]
    fn task_needs_file_empty_refs_returns_true() {
        assert!(task_needs_file(&[]));
    }

    #[test]
    fn task_needs_file_without_docs_ref_returns_true() {
        assert!(task_needs_file(&["parent", "relates"]));
    }

    #[test]
    fn task_needs_file_with_docs_ref_returns_false() {
        assert!(!task_needs_file(&["docs"]));
    }

    #[test]
    fn task_needs_file_docs_among_others_returns_false() {
        assert!(!task_needs_file(&["parent", "docs", "relates"]));
    }

    // -- sanitize_name ---------------------------------------------------------

    #[test]
    fn sanitize_name_replaces_spaces_with_dashes() {
        assert_eq!(sanitize_name("My Board"), "My-Board");
    }

    #[test]
    fn sanitize_name_collapses_consecutive_dashes() {
        assert_eq!(sanitize_name("A  B"), "A-B");
    }

    #[test]
    fn sanitize_name_trims_leading_trailing_dashes() {
        assert_eq!(sanitize_name(" Board "), "Board");
    }

    #[test]
    fn unique_filename_no_collision_returns_base() {
        let used = HashSet::new();
        assert_eq!(unique_filename("Note", "note", &used), "Note.md");
    }

    #[test]
    fn unique_filename_collision_appends_slug() {
        let mut used = HashSet::new();
        used.insert("Note.md".to_string());
        assert_eq!(unique_filename("Note", "atl-1", &used), "Note-atl-1.md");
    }

    #[test]
    fn unique_filename_double_collision_appends_number() {
        let mut used = HashSet::new();
        used.insert("Note.md".to_string());
        used.insert("Note-atl-1.md".to_string());
        assert_eq!(unique_filename("Note", "atl-1", &used), "Note-2.md");
    }

    // -- safe_dir_component / folder path traversal ----------------------------

    fn folder_dto(
        id: u128,
        name: &str,
        parent: Option<u128>,
    ) -> atlas_api::dtos::folders::FolderDto {
        atlas_api::dtos::folders::FolderDto {
            id: Uuid::from_u128(id),
            workspace_id: Uuid::from_u128(999),
            project_id: None,
            parent_folder_id: parent.map(Uuid::from_u128),
            name: name.to_string(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn safe_dir_component_neutralizes_parent_dir_name() {
        assert_eq!(safe_dir_component(".."), "unnamed");
    }

    #[test]
    fn safe_dir_component_neutralizes_empty_and_dot_names() {
        assert_eq!(safe_dir_component(""), "unnamed");
        assert_eq!(safe_dir_component("."), "unnamed");
        assert_eq!(safe_dir_component("/"), "unnamed");
    }

    #[test]
    fn safe_dir_component_keeps_regular_names() {
        assert_eq!(safe_dir_component("My Folder"), "My-Folder");
    }

    #[test]
    fn compute_folder_paths_sanitizes_traversal_folder_name() {
        let folders = vec![folder_dto(1, "../x", None)];
        let paths = compute_folder_paths(&folders);

        let path = paths.get(&Uuid::from_u128(1)).unwrap();
        assert_eq!(path, &PathBuf::from("..-x"));
    }

    #[test]
    fn compute_folder_paths_sanitizes_absolute_folder_name() {
        let folders = vec![folder_dto(1, "/etc/x", None)];
        let paths = compute_folder_paths(&folders);

        let path = paths.get(&Uuid::from_u128(1)).unwrap();
        assert_eq!(path, &PathBuf::from("etc-x"));
    }

    #[test]
    fn compute_folder_paths_sanitizes_nested_traversal_names() {
        let folders = vec![
            folder_dto(1, "..", None),
            folder_dto(2, "../../etc", Some(1)),
        ];
        let paths = compute_folder_paths(&folders);

        for path in paths.values() {
            for component in path.components() {
                assert!(
                    matches!(component, std::path::Component::Normal(_)),
                    "folder path '{}' must contain only normal components",
                    path.display()
                );
            }
        }
    }

    // -- render_task_content ---------------------------------------------------

    #[test]
    fn render_task_content_uses_description_as_body() {
        let fm = serde_json::json!({"type": "task", "status": "Doing"});
        let content = render_task_content(&fm, "The actual task description.");

        assert!(content.ends_with("The actual task description."));
        assert!(content.contains("status: Doing"));
    }

    #[test]
    fn render_task_content_empty_description_yields_frontmatter_only() {
        let fm = serde_json::json!({"type": "task", "status": "Doing"});
        let content = render_task_content(&fm, "");

        assert_eq!(content, frontmatter_to_yaml(&fm));
    }

    // -- dir_exists_nonempty / --force -----------------------------------------

    #[test]
    fn dir_exists_nonempty_false_for_missing_path() {
        assert!(!dir_exists_nonempty(Path::new("/nonexistent/missing/dir")).unwrap());
    }

    #[test]
    fn dir_exists_nonempty_false_for_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!dir_exists_nonempty(dir.path()).unwrap());
    }

    #[test]
    fn dir_exists_nonempty_true_for_dir_with_entries() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("existing.md"), "x").unwrap();
        assert!(dir_exists_nonempty(dir.path()).unwrap());
    }

    #[test]
    fn obsidian_export_parses_force_flag() {
        let cli = Cli::try_parse_from([
            "atlas",
            "export",
            "obsidian",
            "--project",
            "p",
            "--force",
            "/vault",
        ])
        .unwrap();
        let Commands::Export(args) = cli.command else {
            panic!("expected Export command");
        };
        let super::super::ExportCmd::Obsidian(obs) = args.command;
        assert!(obs.force);
    }
}
