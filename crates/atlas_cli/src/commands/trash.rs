#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use atlas_api::dtos::lifecycle::TrashKindDto;
use atlas_client::PurgeTrashResult;
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use crate::commands::common::{LIMIT_DEFAULT, LIMIT_MAX, LIMIT_MIN};
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;
use crate::output::TableRow;

#[derive(Serialize)]
struct TrashItemProjection {
    workspace_id: uuid::Uuid,
    kind: String,
    target_id: uuid::Uuid,
    deleted_at: chrono::DateTime<chrono::Utc>,
}

impl From<atlas_api::dtos::lifecycle::TrashItemDto> for TrashItemProjection {
    fn from(item: atlas_api::dtos::lifecycle::TrashItemDto) -> Self {
        Self {
            workspace_id: item.workspace_id,
            kind: trash_kind_name(item.kind).to_owned(),
            target_id: item.target_id,
            deleted_at: item.deleted_at,
        }
    }
}

impl TableRow for TrashItemProjection {
    fn headers() -> &'static [&'static str] {
        &["Workspace", "Kind", "Target", "Deleted"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.workspace_id.to_string(),
            self.kind.clone(),
            self.target_id.to_string(),
            self.deleted_at.format("%Y-%m-%d %H:%M:%S UTC").to_string(),
        ]
    }
}

#[derive(Serialize)]
struct PurgeStatusProjection {
    operation_id: uuid::Uuid,
    kind: String,
    target_id: uuid::Uuid,
    status: String,
    attempts: u32,
}

impl From<atlas_api::dtos::lifecycle::PurgeStatusDtoResponse> for PurgeStatusProjection {
    fn from(status: atlas_api::dtos::lifecycle::PurgeStatusDtoResponse) -> Self {
        Self {
            operation_id: status.operation_id,
            kind: trash_kind_name(status.kind).to_owned(),
            target_id: status.target_id,
            status: purge_status_name(status.status).to_owned(),
            attempts: status.attempts,
        }
    }
}

impl TableRow for PurgeStatusProjection {
    fn headers() -> &'static [&'static str] {
        &["Operation", "Kind", "Target", "Status", "Attempts"]
    }

    fn row(&self) -> Vec<String> {
        vec![
            self.operation_id.to_string(),
            self.kind.clone(),
            self.target_id.to_string(),
            self.status.clone(),
            self.attempts.to_string(),
        ]
    }
}

#[derive(Serialize)]
struct TrashActionProjection {
    action: &'static str,
    target_id: uuid::Uuid,
}

impl TableRow for TrashActionProjection {
    fn headers() -> &'static [&'static str] {
        &["Action", "Target"]
    }

    fn row(&self) -> Vec<String> {
        vec![self.action.to_owned(), self.target_id.to_string()]
    }
}

/// Root/system-admin human-session Trash administration commands. API keys are
/// rejected by the server for every command in this group.
#[derive(Args)]
pub(crate) struct TrashArgs {
    #[command(subcommand)]
    pub(crate) command: TrashCmd,
}

#[derive(Subcommand)]
pub(crate) enum TrashCmd {
    /// List recoverably deleted resources.
    List(TrashListArgs),
    /// Restore a recoverably deleted resource.
    Restore(TrashTargetArgs),
    /// Permanently purge a deleted resource. Requires --confirm.
    Purge(TrashPurgeArgs),
    /// Read the durable status of a purge operation.
    Status(TrashStatusArgs),
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum TrashKindArg {
    Project,
    Folder,
    Document,
    Comment,
    Attachment,
}

impl From<TrashKindArg> for TrashKindDto {
    fn from(kind: TrashKindArg) -> Self {
        match kind {
            TrashKindArg::Project => Self::Project,
            TrashKindArg::Folder => Self::Folder,
            TrashKindArg::Document => Self::Document,
            TrashKindArg::Comment => Self::Comment,
            TrashKindArg::Attachment => Self::Attachment,
        }
    }
}

#[derive(Parser)]
pub(crate) struct TrashListArgs {
    /// Filter to a workspace UUID.
    #[arg(long)]
    pub(crate) workspace_id: Option<uuid::Uuid>,
    /// Filter to one recoverable resource kind.
    #[arg(long)]
    pub(crate) kind: Option<TrashKindArg>,
    /// Opaque pagination cursor returned by a previous list.
    #[arg(long)]
    pub(crate) cursor: Option<String>,
    /// Maximum number of results to return (clamped to 1..=200).
    #[arg(long)]
    pub(crate) limit: Option<u32>,
}

#[derive(Parser)]
pub(crate) struct TrashTargetArgs {
    /// Recoverable resource kind.
    #[arg(long)]
    pub(crate) kind: TrashKindArg,
    /// UUID of the deleted resource.
    #[arg(long)]
    pub(crate) target_id: uuid::Uuid,
}

#[derive(Parser)]
pub(crate) struct TrashPurgeArgs {
    /// Recoverable resource kind.
    #[arg(long)]
    pub(crate) kind: TrashKindArg,
    /// UUID of the deleted resource.
    #[arg(long)]
    pub(crate) target_id: uuid::Uuid,
    /// Confirm permanent purge. This operation cannot be restored.
    #[arg(long, required = true)]
    pub(crate) confirm: bool,
}

#[derive(Parser)]
pub(crate) struct TrashStatusArgs {
    /// UUID of the purge operation.
    #[arg(long)]
    pub(crate) operation_id: uuid::Uuid,
}

pub(crate) async fn run(ctx: &Ctx, cmd: TrashCmd) -> Result<(), CliError> {
    match cmd {
        TrashCmd::List(args) => run_list(ctx, args).await,
        TrashCmd::Restore(args) => run_restore(ctx, args).await,
        TrashCmd::Purge(args) => run_purge(ctx, args).await,
        TrashCmd::Status(args) => run_status(ctx, args).await,
    }
}

async fn run_list(ctx: &Ctx, args: TrashListArgs) -> Result<(), CliError> {
    let limit = args
        .limit
        .unwrap_or(LIMIT_DEFAULT)
        .clamp(LIMIT_MIN, LIMIT_MAX);
    let page = ctx
        .client
        .list_trash(
            args.workspace_id,
            args.kind.map(Into::into),
            args.cursor.as_deref(),
            Some(limit),
        )
        .await?;

    let items = page
        .items
        .into_iter()
        .map(TrashItemProjection::from)
        .collect::<Vec<_>>();
    output::emit_list(
        ctx.output,
        &items,
        page.next_cursor.as_deref(),
        page.has_more,
    )
}

async fn run_restore(ctx: &Ctx, args: TrashTargetArgs) -> Result<(), CliError> {
    ctx.client
        .restore_trash(args.kind.into(), args.target_id)
        .await?;
    output::emit(
        ctx.output,
        &TrashActionProjection {
            action: "restored",
            target_id: args.target_id,
        },
    )
}

async fn run_purge(ctx: &Ctx, args: TrashPurgeArgs) -> Result<(), CliError> {
    if !args.confirm {
        return Err(CliError::Validation(
            "pass --confirm to permanently purge a recoverable resource".to_owned(),
        ));
    }

    match ctx
        .client
        .purge_trash(args.kind.into(), args.target_id, true)
        .await?
    {
        PurgeTrashResult::Complete => output::emit(
            ctx.output,
            &TrashActionProjection {
                action: "purged",
                target_id: args.target_id,
            },
        ),
        PurgeTrashResult::Pending(status) => {
            output::emit(ctx.output, &PurgeStatusProjection::from(status))
        }
    }
}

async fn run_status(ctx: &Ctx, args: TrashStatusArgs) -> Result<(), CliError> {
    let status = ctx.client.get_purge_status(args.operation_id).await?;
    output::emit(ctx.output, &PurgeStatusProjection::from(status))
}

fn trash_kind_name(kind: TrashKindDto) -> &'static str {
    match kind {
        TrashKindDto::Project => "project",
        TrashKindDto::Folder => "folder",
        TrashKindDto::Document => "document",
        TrashKindDto::Comment => "comment",
        TrashKindDto::Attachment => "attachment",
    }
}

fn purge_status_name(status: atlas_api::dtos::lifecycle::PurgeStatusDto) -> &'static str {
    match status {
        atlas_api::dtos::lifecycle::PurgeStatusDto::DbCommitted => "db_committed",
        atlas_api::dtos::lifecycle::PurgeStatusDto::CleanupPending => "cleanup_pending",
        atlas_api::dtos::lifecycle::PurgeStatusDto::CleanupFailed => "cleanup_failed",
        atlas_api::dtos::lifecycle::PurgeStatusDto::Complete => "complete",
    }
}
