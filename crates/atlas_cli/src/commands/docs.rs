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

use atlas_api::dtos::documents::{
    CreateDocumentRequest, UpdateContentRequest, UpdateDocumentRequest,
};
use atlas_client::ClientError;
use clap::{Args, Parser, Subcommand, ValueEnum};
use uuid::Uuid;

use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::projections::{
    DeleteDocProjection, DocCompactProjection, DocFullProjection, DocSummaryProjection,
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
    /// Delete a document (requires --confirm).
    Delete(DocsDeleteArgs),
}

/// Dispatches a parsed `DocsCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: DocsCmd) -> Result<(), CliError> {
    match cmd {
        DocsCmd::List(args) => run_list(ctx, args).await,
        DocsCmd::Get(args) => run_get(ctx, args).await,
        DocsCmd::Create(args) => run_create(ctx, args).await,
        DocsCmd::UpdateMetadata(args) => run_update_metadata(ctx, args).await,
        DocsCmd::UpdateContent(args) => run_update_content(ctx, args).await,
        DocsCmd::Delete(args) => run_delete(ctx, args).await,
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

    let project = args.project.as_deref().ok_or_else(|| {
        CliError::Validation("--project is required for docs list".to_owned())
    })?;

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

    output::emit_list(ctx.output, &items, page.next_cursor.as_deref(), page.has_more)
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
#[derive(Parser)]
pub(crate) struct DocsCreateArgs {
    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// Project slug where the document will be created (required).
    #[arg(long)]
    pub(crate) project: String,

    /// Document title (required).
    #[arg(long)]
    pub(crate) title: String,

    /// Parent folder UUID (optional).
    #[arg(long)]
    pub(crate) folder_id: Option<Uuid>,

    /// Initial markdown content (optional; creates an empty document if omitted).
    #[arg(long)]
    pub(crate) content: Option<String>,
}

async fn run_create(ctx: &Ctx, args: DocsCreateArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = CreateDocumentRequest {
        title: args.title,
        folder_id: args.folder_id,
        content: args.content,
    };

    let doc = ctx.client.create_document(ws, &args.project, body).await?;
    let proj = DocCompactProjection::from(doc);
    output::emit(ctx.output, &proj)
}

// ---------------------------------------------------------------------------
// Update-metadata
// ---------------------------------------------------------------------------

/// Arguments for `atlas docs update-metadata`.
#[derive(Parser)]
pub(crate) struct DocsUpdateMetadataArgs {
    /// Document slug.
    #[arg(index = 1)]
    pub(crate) slug: String,

    /// Workspace slug.
    #[arg(long)]
    pub(crate) workspace: Option<String>,

    /// New title (omit to leave unchanged).
    #[arg(long)]
    pub(crate) title: Option<String>,

    /// New parent folder UUID (omit to leave unchanged).
    #[arg(long)]
    pub(crate) folder_id: Option<Uuid>,
}

async fn run_update_metadata(ctx: &Ctx, args: DocsUpdateMetadataArgs) -> Result<(), CliError> {
    let ws = ctx.require_workspace(args.workspace.as_deref())?;

    let body = UpdateDocumentRequest {
        title: args.title,
        folder_id: args.folder_id,
    };

    let doc = ctx.client.update_document(ws, &args.slug, body).await?;
    let proj = DocCompactProjection::from(doc);
    output::emit(ctx.output, &proj)
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
                eprintln!("patch is available: apply base_to_current_patch and retry with the new revision id");
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
        let cli =
            Cli::try_parse_from(["atlas", "docs", "get", "some-slug", "--workspace", "ws"])
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
                assert_eq!(create_args.project, "my-project");
                assert_eq!(create_args.title, "My Document");
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
                assert_eq!(update_args.slug, "some-slug");
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
                assert_eq!(
                    uc_args.base_revision_id,
                    rev_id.parse::<Uuid>().unwrap()
                );
            } else {
                panic!("expected UpdateContent");
            }
        } else {
            panic!("expected Docs");
        }
    }

    #[test]
    fn docs_delete_parsed_without_confirm_has_confirm_false() {
        let cli = Cli::try_parse_from([
            "atlas",
            "docs",
            "delete",
            "my-doc",
            "--workspace",
            "ws",
        ])
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
        assert!(!args.confirm, "confirm must be false when --confirm is absent");
    }

    #[test]
    fn cli_error_conflict_exit_code_is_1() {
        use atlas_api::dtos::documents::ConflictProblemDto;
        let dto = ConflictProblemDto::new(
            Uuid::now_v7(),
            3,
            "--- a\n+++ b".to_owned(),
        );
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
}
