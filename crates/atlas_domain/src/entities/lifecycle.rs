use std::str::FromStr;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    DomainError,
    actor::WorkspaceCtx,
    ids::{PurgeOperationId, SecurityAuditId, UserId, WorkspaceId},
};

/// Resource kinds that have independent Trash, restore, and purge lifecycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrashKind {
    Project,
    Folder,
    Document,
    Comment,
    Attachment,
}

impl TrashKind {
    pub const ALL: [Self; 5] = [
        Self::Project,
        Self::Folder,
        Self::Document,
        Self::Comment,
        Self::Attachment,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::Folder => "folder",
            Self::Document => "document",
            Self::Comment => "comment",
            Self::Attachment => "attachment",
        }
    }
}

impl FromStr for TrashKind {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "project" => Ok(Self::Project),
            "folder" => Ok(Self::Folder),
            "document" => Ok(Self::Document),
            "comment" => Ok(Self::Comment),
            "attachment" => Ok(Self::Attachment),
            _ => Err("unsupported trash kind"),
        }
    }
}

/// Durable state of a confirmed purge operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PurgeStatus {
    DbCommitted,
    CleanupPending,
    CleanupFailed,
    Complete,
}

impl PurgeStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::DbCommitted => "db_committed",
            Self::CleanupPending => "cleanup_pending",
            Self::CleanupFailed => "cleanup_failed",
            Self::Complete => "complete",
        }
    }
}

impl FromStr for PurgeStatus {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "db_committed" => Ok(Self::DbCommitted),
            "cleanup_pending" => Ok(Self::CleanupPending),
            "cleanup_failed" => Ok(Self::CleanupFailed),
            "complete" => Ok(Self::Complete),
            _ => Err("unsupported purge status"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TrashItem {
    pub workspace_id: WorkspaceId,
    pub kind: TrashKind,
    pub target_id: Uuid,
    pub deleted_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct RestoreTarget {
    pub kind: TrashKind,
    pub target_id: Uuid,
}

#[derive(Debug, Clone)]
pub struct PurgeOperation {
    pub id: PurgeOperationId,
    pub workspace_id: WorkspaceId,
    pub target: RestoreTarget,
    pub original_actor_user_id: UserId,
    pub commit_audit_id: SecurityAuditId,
    pub status: PurgeStatus,
    pub attempts: u32,
    pub last_action: String,
    pub last_executor: String,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct PurgeDigest {
    pub operation_id: PurgeOperationId,
    pub digest: String,
    pub status: PurgeStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub last_attempt_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default)]
pub struct TrashFilter {
    pub workspace_id: Option<WorkspaceId>,
    pub kind: Option<TrashKind>,
}

#[async_trait::async_trait]
pub trait TrashLifecycleRepo: Send + Sync {
    async fn list_trash(
        &self,
        ctx: &WorkspaceCtx,
        filter: TrashFilter,
        after: Option<Uuid>,
        limit: u64,
    ) -> Result<Vec<TrashItem>, DomainError>;

    async fn restore(
        &self,
        ctx: &WorkspaceCtx,
        target: RestoreTarget,
    ) -> Result<TrashItem, DomainError>;

    async fn find_purge_operation(
        &self,
        ctx: &WorkspaceCtx,
        target: RestoreTarget,
    ) -> Result<Option<PurgeOperation>, DomainError>;
}
