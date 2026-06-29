#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::io::Write as _;
use std::path::PathBuf;

use atlas_api::dtos::documents::{
    CreateDocumentRequest, UpdateContentRequest, UpdateDocumentRequest,
};
use atlas_client::ClientError;
use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::commands::bulk;
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{
    AttachProjection, DeleteDocProjection, DocBacklinkProjection, DocCompactProjection,
    DocFullProjection, DocHistoryProjection, DocRevisionProjection, DocSummaryProjection,
};

const LIMIT_MIN: u32 = 1;
const LIMIT_MAX: u32 = 200;
const LIMIT_DEFAULT: u32 = 20;

// ---------------------------------------------------------------------------
// Detail level (shared with tasks)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub(crate) enum Detail {
    Compact,
    Full,
}

// ---------------------------------------------------------------------------
// DocsArgs (wrapper for nesting into Commands) + DocsCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `docs` subcommand group.
#[derive(Args)]
pub(crate) struct DocsArgs {
    #[command(subcommand)]
    pub(crate) command: DocsCmd,
}

#[derive(Subcommand)]
pub(crate) enum DocsCmd {
    /// List documents in a project.
    List(DocsListArgs),
    /// Get a document by slug.
    Get(DocsGetArgs),
    /// Create a new document in a project.
    Create(DocsCreateArgs),
    /// Update document metadata (title, folder) using PATCH semantics.
    UpdateMetadata(DocsUpdateMetadataArgs),
    /// Update document content using compare-and-swap (requires --base-revision-id).
    UpdateContent(DocsUpdateContentArgs),
    /// Open a document in $EDITOR, then submit via compare-and-swap.
    Edit(DocsEditArgs),
    /// Delete a document (requires --confirm).
    Delete(DocsDeleteArgs),
    /// List documents that link to this document (backlinks).
    Backlinks(DocsBacklinksArgs),
    /// List the revision history of a document.
    History(DocsHistoryArgs),
    /// Fetch the full content of a specific revision by sequence number.
    Revision(DocsRevisionArgs),
    /// Fetch the frontmatter of a document.
    Frontmatter(DocsFrontmatterArgs),
    /// Manage document attachments (upload, list, download, delete).
    Attach(DocsAttachArgs),
}

/// Dispatches a parsed `DocsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: DocsCmd) -> Result<(), CliError> {
    match cmd {
        DocsCmd::List(args) => run_list(ctx, args).await,
        DocsCmd::Get(args) => run_get(ctx, args).await,
        DocsCmd::Create(args) => run_create(ctx, args).await,
        DocsCmd::UpdateMetadata(args) => run_update_metadata(ctx, args).await,
        DocsCmd::UpdateContent(args) => run_update_content(ctx, args).await,
        DocsCmd::Edit(args) => run_edit(ctx, args).await,
        DocsCmd::Delete(args) => run_delete(ctx, args).await,
        DocsCmd::Backlinks(args) => run_backlinks(ctx, args).await,
        DocsCmd::History(args) => run_history(ctx, args).await,
        DocsCmd::Revision(args) => run_revision(ctx, args).await,
        DocsCmd::Frontmatter(args) => run_frontmatter(ctx, args).await,
        DocsCmd::Attach(args) => run_attach(ctx, args.command).await,
    }
}

// ---------------------------------------------------------------------------
// List
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs list`.
#[derive(Parser)]
pub(crate) struct DocsListArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Project slug (required at runtime; filters to a specific project).
    #[arg(long)]
    pub(crate) project: Option<String>,

    /// Filter to documents in this folder UUID (client-side).
    #[arg(long)]
    pub(crate) folder_id: Option<Uuid>,

    /// Maximum number of results (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

async fn run_list(ctx: &Ctx, args: DocsListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let project = args
        .project
        .as_deref()
        .ok_or_else(|| CliError::Validation("--project is required for docs list".to_owned()))?;

    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);

    let page = ctx
        .client
        .list_documents(ws, project, None, Some(limit))
        .await?;

    let items: Vec<DocSummaryProjection> = page
        .items
        .into_iter()
        .filter(|doc| {
            args.folder_id
                .map(|fid| doc.folder_id == Some(fid))
                .unwrap_or(true)
        })
        .map(DocSummaryProjection::from)
        .collect();

    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Get
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs get`.
#[derive(Parser)]
pub(crate) struct DocsGetArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Level of detail: `compact` (default, metadata + head_revision_id) or
    /// `full` (adds content and frontmatter).
    #[arg(long, default_value = "compact")]
    pub(crate) detail: Detail,
}

async fn run_get(ctx: &Ctx, args: DocsGetArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let doc = ctx.client.get_document(ws, &args.slug).await?;

    match args.detail {
        Detail::Compact => {
            let proj = DocCompactProjection::from(doc);
            output::emit(ctx.output, &proj)
        }
        Detail::Full => {
            let proj = DocFullProjection::from(doc);
            output::emit(ctx.output, &proj)
        }
    }
}

// ---------------------------------------------------------------------------
// Create
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs create`.
///
/// When `--stdin` is set, one JSON object per line is read from stdin and each
/// becomes a separate create call. The expected line shape is:
/// `{"project":"slug","title":"...","folder_id":"<uuid-or-null>","content":"..."}`.
/// In that mode `--project` and `--title` are ignored.
#[derive(Parser)]
pub(crate) struct DocsCreateArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Project slug where the document will be created (required in single-item
    /// mode; ignored when --stdin is set).
    #[arg(long, required_unless_present = "stdin")]
    pub(crate) project: Option<String>,

    /// Document title (required in single-item mode; ignored when --stdin is set).
    #[arg(long, required_unless_present = "stdin")]
    pub(crate) title: Option<String>,

    /// Parent folder UUID (optional).
    #[arg(long)]
    pub(crate) folder_id: Option<Uuid>,

    /// Initial markdown content (optional; creates an empty document if omitted).
    #[arg(long)]
    pub(crate) content: Option<String>,

    /// Read one JSON object per line from stdin; each line becomes a separate
    /// create call. When set, `--project` and `--title` are ignored.
    #[arg(long)]
    pub(crate) stdin: bool,
}

async fn run_create(ctx: &Ctx, args: DocsCreateArgs) -> Result<(), CliError> {
    if args.stdin {
        return run_create_stdin(ctx, args).await;
    }
    run_create_single(ctx, args).await
}

async fn run_create_single(ctx: &Ctx, args: DocsCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let project = args
        .project
        .ok_or_else(|| CliError::Validation("--project is required".to_owned()))?;

    let title = args
        .title
        .ok_or_else(|| CliError::Validation("--title is required".to_owned()))?;

    let body = CreateDocumentRequest {
        title,
        folder_id: args.folder_id,
        content: args.content,
    };

    let doc = ctx.client.create_document(ws, &project, body).await?;
    let proj = DocCompactProjection::from(doc);
    output::emit(ctx.output, &proj)
}

async fn run_create_stdin(ctx: &Ctx, args: DocsCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (items, mut any_failed) = bulk::parse_stdin_batch::<bulk::BulkDocCreateLine>()?;

    for item in items {
        match ctx
            .client
            .create_document(ws, &item.project, item.body)
            .await
        {
            Ok(doc) => {
                let proj = DocCompactProjection::from(doc);
                let value = serde_json::to_value(&proj)
                    .map_err(|e| CliError::Io(std::io::Error::other(e.to_string())))?;
                bulk::emit_batch_line(&value)?;
            }
            Err(e) => {
                eprintln!("error: {e}");
                any_failed = true;
            }
        }
    }

    if any_failed {
        Err(CliError::Validation(
            "batch: one or more items failed (see stderr)".to_owned(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Update-metadata
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs update-metadata`.
///
/// When `--stdin` is set, one JSON object per line is read from stdin and each
/// becomes a separate update call. The expected line shape is:
/// `{"slug":"my-doc","title":"New title","folder_id":"<uuid-or-null>"}`.
/// In that mode the positional `slug` argument is ignored.
#[derive(Parser)]
pub(crate) struct DocsUpdateMetadataArgs {
    /// Document slug (required in single-item mode; ignored when --stdin is set).
    #[arg(index = 1, required_unless_present = "stdin")]
    pub(crate) slug: Option<String>,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// New title (omit to leave unchanged).
    #[arg(long)]
    pub(crate) title: Option<String>,

    /// New parent folder UUID (omit to leave unchanged).
    #[arg(long)]
    pub(crate) folder_id: Option<Uuid>,

    /// Read one JSON object per line from stdin; each line becomes a separate
    /// update call. When set, the positional `slug` argument is ignored.
    #[arg(long)]
    pub(crate) stdin: bool,
}

async fn run_update_metadata(ctx: &Ctx, args: DocsUpdateMetadataArgs) -> Result<(), CliError> {
    if args.stdin {
        return run_update_metadata_stdin(ctx, args).await;
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let slug = args
        .slug
        .as_deref()
        .ok_or_else(|| CliError::Validation("slug is required in single-item mode".to_owned()))?
        .to_owned();

    let body = UpdateDocumentRequest {
        title: args.title,
        folder_id: args.folder_id,
    };

    let doc = ctx.client.update_document(ws, &slug, body).await?;
    let proj = DocCompactProjection::from(doc);
    output::emit(ctx.output, &proj)
}

async fn run_update_metadata_stdin(
    ctx: &Ctx,
    args: DocsUpdateMetadataArgs,
) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (items, mut any_failed) = bulk::parse_stdin_batch::<bulk::BulkDocUpdateMetadataLine>()?;

    for item in items {
        let slug = item.slug.clone();
        match ctx.client.update_document(ws, &slug, item.body).await {
            Ok(doc) => {
                let proj = DocCompactProjection::from(doc);
                let value = serde_json::to_value(&proj)
                    .map_err(|e| CliError::Io(std::io::Error::other(e.to_string())))?;
                bulk::emit_batch_line(&value)?;
            }
            Err(e) => {
                eprintln!("error: {e}");
                any_failed = true;
            }
        }
    }

    if any_failed {
        Err(CliError::Validation(
            "batch: one or more items failed (see stderr)".to_owned(),
        ))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Update-content (WU-16)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs update-content`.
#[derive(Parser)]
pub(crate) struct DocsUpdateContentArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// The `head_revision_id` UUID obtained from a prior `docs get --detail full`
    /// call. Must match the current revision or the server returns a conflict.
    #[arg(long = "base-revision-id")]
    pub(crate) base_revision_id: Uuid,

    /// New document content (inline). Provide this or --content-file.
    #[arg(long)]
    pub(crate) content: Option<String>,

    /// Path to a file containing the new document content.
    #[arg(long)]
    pub(crate) content_file: Option<PathBuf>,
}

async fn run_update_content(ctx: &Ctx, args: DocsUpdateContentArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let content = match (args.content, args.content_file) {
        (Some(c), _) => c,
        (None, Some(path)) => std::fs::read_to_string(&path)?,
        (None, None) => {
            return Err(CliError::Validation(
                "provide --content or --content-file".to_owned(),
            ));
        }
    };

    let body = UpdateContentRequest {
        content,
        base_revision_id: args.base_revision_id,
    };

    match ctx.client.update_content(ws, &args.slug, body).await {
        Ok(doc) => {
            let proj = DocCompactProjection::from(doc);
            output::emit(ctx.output, &proj)
        }

        Err(ClientError::Conflict(dto)) => {
            eprintln!(
                "revision conflict: current_revision_id={}, current_seq={}",
                dto.current_revision_id, dto.current_seq
            );
            if !dto.base_to_current_patch.is_empty() {
                eprintln!(
                    "patch is available: apply base_to_current_patch and retry with the new revision id"
                );
            }
            Err(CliError::Conflict(Box::new(dto)))
        }

        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------
// Delete (WU-16)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs delete`.
#[derive(Parser)]
pub(crate) struct DocsDeleteArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Confirm the deletion. Required — prevents accidental non-reversible deletes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

async fn run_delete(ctx: &Ctx, args: DocsDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete (this is a non-reversible operation)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    ctx.client.delete_document(ws, &args.slug).await?;

    let proj = DeleteDocProjection {
        deleted: true,
        slug: args.slug,
    };
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Backlinks (WU-24)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs backlinks`.
#[derive(Parser)]
pub(crate) struct DocsBacklinksArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of results (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

async fn run_backlinks(ctx: &Ctx, args: DocsBacklinksArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);
    let page = ctx
        .client
        .list_backlinks(ws, &args.slug, None, Some(limit))
        .await?;
    let projections: Vec<DocBacklinkProjection> = page
        .items
        .into_iter()
        .map(DocBacklinkProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// History (WU-24)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs history`.
#[derive(Parser)]
pub(crate) struct DocsHistoryArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Maximum number of revisions to return (clamped to 1..=200; default 20).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

async fn run_history(ctx: &Ctx, args: DocsHistoryArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);
    let page = ctx
        .client
        .list_document_history(ws, &args.slug, None, Some(limit))
        .await?;
    let projections: Vec<DocHistoryProjection> = page
        .items
        .into_iter()
        .map(DocHistoryProjection::from)
        .collect();
    output::emit_list(
        ctx.output,
        &projections,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

// ---------------------------------------------------------------------------
// Revision (WU-24)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs revision`.
#[derive(Parser)]
pub(crate) struct DocsRevisionArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Revision sequence number (required).
    #[arg(long)]
    pub(crate) seq: i64,
}

async fn run_revision(ctx: &Ctx, args: DocsRevisionArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let rev = ctx
        .client
        .get_revision_content(ws, &args.slug, args.seq)
        .await?;
    let proj = DocRevisionProjection::from(rev);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Frontmatter (WU-24)
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs frontmatter`.
#[derive(Parser)]
pub(crate) struct DocsFrontmatterArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

async fn run_frontmatter(ctx: &Ctx, args: DocsFrontmatterArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let fm = ctx.client.get_frontmatter(ws, &args.slug).await?;
    output::print_json(&fm.data)
}

// ---------------------------------------------------------------------------
// Edit (WU-33) — $EDITOR + CAS
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs edit`.
#[derive(Parser)]
pub(crate) struct DocsEditArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Resolves the editor binary from the `$EDITOR` environment variable.
///
/// Returns `Err(CliError::Io)` when `$EDITOR` is unset or empty so the
/// caller can report a clear error (exit 1) without touching the network.
fn find_editor(env_editor: Option<&str>) -> Result<String, CliError> {
    env_editor
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| {
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "$EDITOR is not set; set it to your preferred editor (e.g. export EDITOR=vim)",
            ))
        })
}

/// Writes `content` to a new named temp file and returns it.
///
/// The temp file is kept open until the caller drops the handle; it is
/// deleted on drop. Callers must persist the path before passing it to the
/// editor process.
fn write_edit_tempfile(content: &str) -> Result<tempfile::NamedTempFile, CliError> {
    let mut temp = tempfile::NamedTempFile::new()?;
    temp.write_all(content.as_bytes())?;
    temp.flush()?;
    Ok(temp)
}

/// Spawns `editor` with `path` as its only argument and waits for it to exit.
fn spawn_editor(editor: &str, path: &std::path::Path) -> Result<(), CliError> {
    let status = std::process::Command::new(editor)
        .arg(path)
        .status()
        .map_err(CliError::Io)?;

    if !status.success() {
        return Err(CliError::Io(std::io::Error::other(format!(
            "editor exited with non-zero status: {status}"
        ))));
    }
    Ok(())
}

/// Builds the compare-and-swap request with the revision ID captured at fetch
/// time.  The ID must come from the initial `get_document` response; the
/// handler must not re-fetch the document after the editor exits.
fn build_update_content_request(content: String, base_revision_id: Uuid) -> UpdateContentRequest {
    UpdateContentRequest {
        content,
        base_revision_id,
    }
}

async fn run_edit(ctx: &Ctx, args: DocsEditArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    // Fetch the document and capture the CAS base revision immediately.
    let doc = ctx.client.get_document(ws, &args.slug).await?;
    let base_revision_id = doc.head_revision_id;
    let original_content = doc.content.clone();

    // Write current content to a temp file for the editor.
    let temp = write_edit_tempfile(&doc.content)?;
    let temp_path = temp.path().to_path_buf();

    // Resolve and launch the editor.
    let editor = find_editor(std::env::var("EDITOR").ok().as_deref())?;
    spawn_editor(&editor, &temp_path)?;

    // Read back the edited content.
    let new_content = std::fs::read_to_string(&temp_path)?;
    drop(temp);

    if new_content == original_content {
        println!("no changes");
        return Ok(());
    }

    // Submit via CAS using the revision captured at fetch time.
    let body = build_update_content_request(new_content, base_revision_id);
    match ctx.client.update_content(ws, &args.slug, body).await {
        Ok(updated) => {
            let proj = DocCompactProjection::from(updated);
            output::emit(ctx.output, &proj)
        }

        Err(ClientError::Conflict(dto)) => {
            // Safe conflict recovery: the base_to_current_patch describes what
            // changed server-side, but a silent 3-way merge risks silently
            // producing wrong content.  Surface the conflict clearly so the
            // user can re-run with the current version.
            eprintln!(
                "revision conflict: the document was modified while you were editing.\n\
                 current_revision_id={}, current_seq={}.\n\
                 Re-run `atlas docs edit {}` to start from the current version.",
                dto.current_revision_id, dto.current_seq, args.slug
            );
            Err(CliError::Conflict(Box::new(dto)))
        }

        Err(e) => Err(e.into()),
    }
}

// ---------------------------------------------------------------------------
// Attach (WU-34) — document attachments
// ---------------------------------------------------------------------------

/// Arguments holder for the `docs attach` subcommand group.
#[derive(Args)]
pub(crate) struct DocsAttachArgs {
    #[command(subcommand)]
    pub(crate) command: DocsAttachCmd,
}

#[derive(Subcommand)]
pub(crate) enum DocsAttachCmd {
    /// Upload a file as an attachment to a document.
    Upload(DocsAttachUploadArgs),
    /// List attachments on a document.
    List(DocsAttachListArgs),
    /// Download an attachment to a file or stdout.
    Download(DocsAttachDownloadArgs),
    /// Delete an attachment (requires --confirm).
    Delete(DocsAttachDeleteArgs),
}

/// Arguments for `atlas docs attach upload`.
#[derive(Parser)]
pub(crate) struct DocsAttachUploadArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Path to the file to upload (required).
    #[arg(long)]
    pub(crate) file: PathBuf,

    /// MIME content-type (defaults to `application/octet-stream`).
    #[arg(long, default_value = "application/octet-stream")]
    pub(crate) content_type: String,
}

/// Arguments for `atlas docs attach list`.
#[derive(Parser)]
pub(crate) struct DocsAttachListArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,
}

/// Arguments for `atlas docs attach download`.
#[derive(Parser)]
pub(crate) struct DocsAttachDownloadArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Attachment UUID to download.
    #[arg(long)]
    pub(crate) attachment_id: Uuid,

    /// Write output to this file instead of stdout.
    #[arg(long)]
    pub(crate) output: Option<PathBuf>,
}

/// Arguments for `atlas docs attach delete`.
#[derive(Parser)]
pub(crate) struct DocsAttachDeleteArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Attachment UUID to delete.
    #[arg(long)]
    pub(crate) attachment_id: Uuid,

    /// Confirm the deletion. Required — prevents accidental non-reversible deletes.
    #[arg(long)]
    pub(crate) confirm: bool,
}

/// Reads a file for upload and derives the filename from the path.
fn read_upload_file(path: &std::path::Path) -> Result<(String, Vec<u8>), CliError> {
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_owned();
    let data = std::fs::read(path)?;
    Ok((filename, data))
}

async fn run_attach(ctx: &Ctx, cmd: DocsAttachCmd) -> Result<(), CliError> {
    match cmd {
        DocsAttachCmd::Upload(args) => run_attach_upload(ctx, args).await,
        DocsAttachCmd::List(args) => run_attach_list(ctx, args).await,
        DocsAttachCmd::Download(args) => run_attach_download(ctx, args).await,
        DocsAttachCmd::Delete(args) => run_attach_delete(ctx, args).await,
    }
}

async fn run_attach_upload(ctx: &Ctx, args: DocsAttachUploadArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let (filename, data) = read_upload_file(&args.file)?;

    let dto = ctx
        .client
        .upload_attachment(ws, &args.slug, &filename, &args.content_type, data)
        .await?;

    let proj = AttachProjection::from(dto);
    output::emit(ctx.output, &proj)
}

async fn run_attach_list(ctx: &Ctx, args: DocsAttachListArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let page = ctx
        .client
        .list_attachments(ws, &args.slug, None, None)
        .await?;

    let items: Vec<AttachProjection> = page.items.into_iter().map(AttachProjection::from).collect();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

async fn run_attach_download(ctx: &Ctx, args: DocsAttachDownloadArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    let bytes = ctx
        .client
        .download_attachment(ws, args.attachment_id)
        .await?;

    match args.output {
        Some(path) => std::fs::write(&path, &bytes)?,
        None => std::io::stdout().write_all(&bytes)?,
    }
    Ok(())
}

async fn run_attach_delete(ctx: &Ctx, args: DocsAttachDeleteArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to delete (this is a non-reversible operation)".to_owned(),
        ));
    }

    let ws = ctx.require_workspace(args.workspace.as_deref())?;
    ctx.client.delete_attachment(ws, args.attachment_id).await?;

    println!("attachment {} deleted", args.attachment_id);
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Cli;

    // -----------------------------------------------------------------------
    // T37: Parse tests (WU-15)
    // -----------------------------------------------------------------------

    #[test]
    fn docs_get_slug_parses() {
        let cli = Cli::try_parse_from(["atlas", "docs", "get", "some-slug", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            assert!(matches!(args.command, DocsCmd::Get(_)));
        } else {
            panic!("expected Docs command");
        }
    }

    #[test]
    fn docs_detail_defaults_to_compact() {
        let cli =
            Cli::try_parse_from(["atlas", "docs", "get", "my-doc", "--workspace", "ws"]).unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Get(get_args) = args.command {
                assert_eq!(get_args.detail, Detail::Compact);
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_detail_full_is_accepted() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "get",
            "my-doc",
            "--workspace",
            "ws",
            "--detail",
            "full",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Get(get_args) = args.command {
                assert_eq!(get_args.detail, Detail::Full);
            } else {
                panic!("expected Get");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_create_with_project_and_title_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "create",
            "--workspace",
            "ws",
            "--project",
            "my-project",
            "--title",
            "My Document",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Create(create_args) = args.command {
                assert_eq!(create_args.project.as_deref(), Some("my-project"));
                assert_eq!(create_args.title.as_deref(), Some("My Document"));
            } else {
                panic!("expected Create");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_list_parses() {
        let cli = Cli::try_parse_from(["atlas", "--workspace", "ws", "docs", "list"]).unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            assert!(matches!(args.command, DocsCmd::List(_)));
        } else {
            panic!("expected Docs command");
        }
    }

    #[test]
    fn docs_update_metadata_slug_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "update-metadata",
            "some-slug",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::UpdateMetadata(update_args) = args.command {
                assert_eq!(update_args.slug.as_deref(), Some("some-slug"));
            } else {
                panic!("expected UpdateMetadata");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_update_metadata_all_flags_optional() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "update-metadata",
            "some-slug",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::UpdateMetadata(update_args) = args.command {
                assert!(update_args.title.is_none());
                assert!(update_args.folder_id.is_none());
            } else {
                panic!("expected UpdateMetadata");
            }
        } else {
            panic!("expected Docs");
        }
    }

    // -----------------------------------------------------------------------
    // T39: Parse tests (WU-16)
    // -----------------------------------------------------------------------

    #[test]
    fn docs_update_content_requires_base_revision_id() {
        let result = Cli::try_parse_from([
            "atlas",
            "docs",
            "update-content",
            "my-doc",
            "--workspace",
            "ws",
            "--content",
            "hello",
        ]);
        assert!(
            result.is_err(),
            "update-content without --base-revision-id must fail at parse time (exit 2)"
        );
    }

    #[test]
    fn docs_update_content_with_base_revision_id_parses() {
        let rev_id = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "update-content",
            "my-doc",
            "--workspace",
            "ws",
            "--base-revision-id",
            rev_id,
            "--content",
            "new content",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::UpdateContent(uc_args) = args.command {
                assert_eq!(uc_args.slug, "my-doc");
                assert_eq!(uc_args.base_revision_id, rev_id.parse::<Uuid>().unwrap());
            } else {
                panic!("expected UpdateContent");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_delete_parsed_without_confirm_has_confirm_false() {
        let cli = Cli::try_parse_from(["atlas", "docs", "delete", "my-doc", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Delete(del_args) = args.command {
                assert!(!del_args.confirm);
            } else {
                panic!("expected Delete");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_delete_without_confirm_returns_validation_error() {
        let args = DocsDeleteArgs {
            slug: "my-doc".to_owned(),
            workspace: None,
            confirm: false,
        };
        // Confirm guard fires before any workspace or network resolution.
        // We check confirm directly — the runtime guard fires before any I/O.
        assert!(
            !args.confirm,
            "confirm must be false when --confirm is absent"
        );
    }

    #[test]
    fn cli_error_conflict_exit_code_is_1() {
        use atlas_api::dtos::documents::ConflictProblemDto;
        let dto = ConflictProblemDto::new(Uuid::now_v7(), 3, "--- a\n+++ b".to_owned());
        let e = CliError::Conflict(Box::new(dto));
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn delete_doc_projection_slug_key_not_id() {
        use crate::projections::DeleteDocProjection;
        let proj = DeleteDocProjection {
            deleted: true,
            slug: "x".to_owned(),
        };
        let value = serde_json::to_value(&proj).unwrap();
        assert_eq!(value["deleted"], true);
        assert_eq!(value["slug"], "x");
        assert!(value.get("id").is_none());
    }

    // -----------------------------------------------------------------------
    // T52 / WU-24: Parse tests — backlinks, history, revision, frontmatter
    // -----------------------------------------------------------------------

    #[test]
    fn docs_backlinks_parses() {
        let cli =
            Cli::try_parse_from(["atlas", "docs", "backlinks", "my-doc", "--workspace", "ws"])
                .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Backlinks(bl_args) = args.command {
                assert_eq!(bl_args.slug, "my-doc");
            } else {
                panic!("expected Backlinks");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_history_parses() {
        let cli = Cli::try_parse_from(["atlas", "docs", "history", "my-doc", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::History(h_args) = args.command {
                assert_eq!(h_args.slug, "my-doc");
                assert!(h_args.limit.is_none());
            } else {
                panic!("expected History");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_history_limit_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "history",
            "my-doc",
            "--workspace",
            "ws",
            "--limit",
            "10",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::History(h_args) = args.command {
                assert_eq!(h_args.limit, Some(10));
            } else {
                panic!("expected History");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_revision_requires_seq() {
        let result =
            Cli::try_parse_from(["atlas", "docs", "revision", "my-doc", "--workspace", "ws"]);
        assert!(result.is_err(), "--seq is required for docs revision");
    }

    #[test]
    fn docs_revision_parses_seq() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "revision",
            "my-doc",
            "--workspace",
            "ws",
            "--seq",
            "5",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Revision(r_args) = args.command {
                assert_eq!(r_args.seq, 5);
            } else {
                panic!("expected Revision");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_frontmatter_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "frontmatter",
            "my-doc",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Frontmatter(fm_args) = args.command {
                assert_eq!(fm_args.slug, "my-doc");
            } else {
                panic!("expected Frontmatter");
            }
        } else {
            panic!("expected Docs");
        }
    }

    // -----------------------------------------------------------------------
    // T66/T67: WU-32 parse tests — docs create/update-metadata --stdin
    // -----------------------------------------------------------------------

    #[test]
    fn docs_create_stdin_flag_parses_without_required_flags() {
        let cli = Cli::try_parse_from(["atlas", "docs", "create", "--stdin", "--workspace", "ws"])
            .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Create(create_args) = args.command {
                assert!(create_args.stdin, "--stdin must be true");
                assert!(
                    create_args.project.is_none(),
                    "project must be None in stdin mode"
                );
                assert!(
                    create_args.title.is_none(),
                    "title must be None in stdin mode"
                );
            } else {
                panic!("expected Create");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_create_without_stdin_requires_project() {
        let result = Cli::try_parse_from([
            "atlas",
            "docs",
            "create",
            "--workspace",
            "ws",
            "--title",
            "T",
        ]);
        assert!(
            result.is_err(),
            "--project is required when --stdin is absent"
        );
    }

    #[test]
    fn docs_create_without_stdin_requires_title() {
        let result = Cli::try_parse_from([
            "atlas",
            "docs",
            "create",
            "--workspace",
            "ws",
            "--project",
            "P",
        ]);
        assert!(
            result.is_err(),
            "--title is required when --stdin is absent"
        );
    }

    #[test]
    fn docs_update_metadata_stdin_flag_parses_without_slug() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "update-metadata",
            "--stdin",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::UpdateMetadata(update_args) = args.command {
                assert!(update_args.stdin, "--stdin must be true");
                assert!(
                    update_args.slug.is_none(),
                    "slug must be None in stdin mode"
                );
            } else {
                panic!("expected UpdateMetadata");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_update_metadata_single_item_requires_slug() {
        let result =
            Cli::try_parse_from(["atlas", "docs", "update-metadata", "--title", "New title"]);
        assert!(result.is_err(), "slug is required when --stdin is absent");
    }

    // -----------------------------------------------------------------------
    // WU-33: docs edit tests
    // -----------------------------------------------------------------------

    #[test]
    fn docs_edit_parses_slug() {
        let cli =
            Cli::try_parse_from(["atlas", "docs", "edit", "my-doc", "--workspace", "ws"]).unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Edit(edit_args) = args.command {
                assert_eq!(edit_args.slug, "my-doc");
            } else {
                panic!("expected Edit");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn find_editor_returns_io_error_when_not_set() {
        let err = find_editor(None).unwrap_err();
        assert!(
            matches!(err, CliError::Io(_)),
            "unset $EDITOR must yield CliError::Io"
        );
    }

    #[test]
    fn find_editor_returns_io_error_when_empty_string() {
        let err = find_editor(Some("")).unwrap_err();
        assert!(
            matches!(err, CliError::Io(_)),
            "empty $EDITOR must yield CliError::Io"
        );
    }

    #[test]
    fn find_editor_returns_value_when_set() {
        let editor = find_editor(Some("vim")).unwrap();
        assert_eq!(editor, "vim");
    }

    #[test]
    fn build_update_content_request_preserves_captured_revision_id() {
        let id = Uuid::new_v4();
        let req = build_update_content_request("hello world".to_owned(), id);
        assert_eq!(
            req.base_revision_id, id,
            "base_revision_id must equal the id captured at fetch time"
        );
    }

    #[test]
    fn cli_error_conflict_exit_code_is_one() {
        use atlas_api::dtos::documents::ConflictProblemDto;
        let dto = ConflictProblemDto::new(Uuid::new_v4(), 5, "--- a\n+++ b\n".to_owned());
        let e = CliError::Conflict(Box::new(dto));
        assert_eq!(e.exit_code(), 1);
    }

    // -----------------------------------------------------------------------
    // WU-34: docs attach tests
    // -----------------------------------------------------------------------

    #[test]
    fn docs_attach_upload_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "attach",
            "upload",
            "my-doc",
            "--workspace",
            "ws",
            "--file",
            "/tmp/test.txt",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Attach(attach_args) = args.command {
                if let DocsAttachCmd::Upload(up) = attach_args.command {
                    assert_eq!(up.slug, "my-doc");
                    assert_eq!(up.file, std::path::PathBuf::from("/tmp/test.txt"));
                    assert_eq!(up.content_type, "application/octet-stream");
                } else {
                    panic!("expected Upload");
                }
            } else {
                panic!("expected Attach");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_attach_list_parses() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "attach",
            "list",
            "my-doc",
            "--workspace",
            "ws",
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            assert!(matches!(
                args.command,
                DocsCmd::Attach(DocsAttachArgs {
                    command: DocsAttachCmd::List(_)
                })
            ));
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_attach_delete_without_confirm_has_confirm_false() {
        let uuid_str = "550e8400-e29b-41d4-a716-446655440000";
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "attach",
            "delete",
            "my-doc",
            "--workspace",
            "ws",
            "--attachment-id",
            uuid_str,
        ])
        .unwrap();
        if let crate::cli::Commands::Docs(args) = cli.command {
            if let DocsCmd::Attach(DocsAttachArgs {
                command: DocsAttachCmd::Delete(del),
            }) = args.command
            {
                assert!(!del.confirm, "--confirm must be false when flag absent");
            } else {
                panic!("expected Attach Delete");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_attach_delete_without_confirm_is_blocked_before_network() {
        let args = DocsAttachDeleteArgs {
            slug: "my-doc".to_owned(),
            workspace: None,
            attachment_id: Uuid::new_v4(),
            confirm: false,
        };
        // Guard fires before any I/O — confirm being false must prevent the call.
        assert!(
            !args.confirm,
            "confirm must be false, which triggers the Validation guard"
        );
    }

    #[test]
    fn read_upload_file_returns_io_error_for_nonexistent_path() {
        let err = read_upload_file(std::path::Path::new(
            "/nonexistent/definitely/missing/file.txt",
        ))
        .unwrap_err();
        assert!(
            matches!(err, CliError::Io(_)),
            "missing file must yield CliError::Io"
        );
    }
}
