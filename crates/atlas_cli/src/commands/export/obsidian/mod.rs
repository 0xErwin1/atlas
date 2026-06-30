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
use std::path::PathBuf;

use clap::Parser;
use uuid::Uuid;

use atlas_client::ClientError;

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

    for (id, path) in &folder_paths {
        let _ = id;
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

    if !boards.is_empty() {
        plan.dirs.push(DirOp {
            rel_path: PathBuf::from("boards"),
        });
    }

    for board in &boards {
        let board_dir = PathBuf::from("boards").join(sanitize_name(&board.name));
        plan.dirs.push(DirOp {
            rel_path: board_dir.clone(),
        });

        let epic_fm = serde_json::json!({ "type": "epic" });
        let epic_filename = safe_filename(&board.name, &sanitize_name(&board.name));
        plan.files.push(FileOp {
            rel_path: board_dir.join(&epic_filename),
            content: frontmatter_to_yaml(&epic_fm),
        });

        let columns = client.list_columns(ws, board.id).await?;
        let col_by_id: HashMap<Uuid, String> =
            columns.iter().map(|c| (c.id, c.name.clone())).collect();

        let tasks = paginate_tasks(client, ws, board.id).await?;
        let mut task_names: HashSet<String> = HashSet::new();

        for task_summary in &tasks {
            let readable_id = &task_summary.readable_id;
            let refs = client.list_references(ws, readable_id).await?;

            let depends: Vec<String> = refs
                .iter()
                .filter(|r| r.kind == "parent")
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

            let content = format!(
                "{}{}",
                frontmatter_to_yaml(&task_fm),
                deuid_links(&task_summary.board_name),
            );

            plan.files.push(FileOp {
                rel_path: board_dir.join(&task_filename),
                content,
            });
        }
    }

    Ok(plan)
}

/// Paginates `list_folders` until exhausted, returning all folder DTOs.
async fn paginate_folders(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::folders::FolderDto>, CliError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_folders(ws, project, cursor.as_deref(), Some(100))
            .await?;
        all.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(all)
}

/// Paginates `list_documents` until exhausted.
async fn paginate_documents(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::documents::DocumentSummaryDto>, CliError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_documents(ws, project, cursor.as_deref(), Some(100))
            .await?;
        all.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(all)
}

/// Paginates `list_boards` until exhausted.
async fn paginate_boards(
    client: &atlas_client::AtlasClient,
    ws: &str,
    project: &str,
) -> Result<Vec<atlas_api::dtos::boards_tasks::BoardSummaryDto>, CliError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_boards(ws, project, cursor.as_deref(), Some(100))
            .await?;
        all.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(all)
}

/// Paginates `list_tasks` for a given board until exhausted.
async fn paginate_tasks(
    client: &atlas_client::AtlasClient,
    ws: &str,
    board_id: Uuid,
) -> Result<Vec<atlas_api::dtos::boards_tasks::TaskSummaryDto>, CliError> {
    let mut all = Vec::new();
    let mut cursor: Option<String> = None;

    loop {
        let page = client
            .list_tasks(ws, board_id, cursor.as_deref(), Some(100))
            .await?;
        all.extend(page.items);
        if !page.has_more {
            break;
        }
        cursor = page.next_cursor;
    }

    Ok(all)
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

    let path = match folder.parent_folder_id {
        None => PathBuf::from(&folder.name),
        Some(parent_id) => {
            resolve_folder_path(parent_id, by_id, paths, depth + 1);
            match paths.get(&parent_id) {
                Some(parent_path) => parent_path.join(&folder.name),
                None => PathBuf::from(&folder.name),
            }
        }
    };

    paths.insert(id, path);
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
}
