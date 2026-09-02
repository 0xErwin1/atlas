#[allow(unreachable_pub)]
mod attachment_store;
#[allow(unreachable_pub)]
mod comment_attachment_drafts;
#[allow(unreachable_pub)]
mod documents;
#[allow(unreachable_pub)]
mod grant_diagnostics;
#[allow(unreachable_pub)]
mod idempotency;
#[allow(unreachable_pub)]
mod identity;
#[allow(unreachable_pub)]
mod integration_config;
mod permissions;
#[allow(unreachable_pub)]
mod s3_attachment_store;
#[allow(unreachable_pub)]
mod security_audit;
#[allow(unreachable_pub)]
mod semantic_indexer;
#[allow(unreachable_pub)]
mod workspace_attachments;
#[allow(unreachable_pub)]
mod workspace_core;

pub use identity::{
    ActivationTokenRepo, ApiKey, ApiKeyRepo, NewActivationToken, NewApiKey, NewSession, NewUser,
    PgUiStateRepo, Session, SessionRepo, UiStateRepo, User, UserRepo, UserUiState,
};

pub use idempotency::{
    CLEANUP_BATCH_LIMIT, CompleteOutcome, IN_FLIGHT_TTL, IdempotencyScope, InsertOutcome,
    PgIdempotencyRepo, StoredResponse,
};

pub use attachment_store::DiskAttachmentStore;
pub use comment_attachment_drafts::PgCommentAttachmentDraftRepo;
pub use documents::{
    AttachmentRepo, AttachmentWriteIntentRepo, PgAttachmentLifecycle, PgAttachmentRepo,
    PgAttachmentWriteIntentRepo,
};
pub use s3_attachment_store::{S3AttachmentStore, S3Config};
pub use workspace_attachments::PgWorkspaceAttachmentRepo;
pub use workspace_core::{FolderRepo, PgFolderRepo, PgProjectRepo, ProjectRepo};

pub use grant_diagnostics::count_orphaned_grants;
pub use permissions::PermissionGrantRepo;
pub use security_audit::{
    append_resource_deleted_in, append_resource_purge_committed_in, append_resource_restored_in,
};
pub use semantic_indexer::{MAX_CHUNK_CHARS, PgSemanticIndexer};

pub use integration_config::PgIntegrationConfigRepo;
