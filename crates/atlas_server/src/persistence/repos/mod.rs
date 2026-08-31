#[allow(unreachable_pub)]
mod attachment_store;
#[allow(unreachable_pub)]
mod automation_rule;
#[allow(unreachable_pub)]
mod comment_attachment_drafts;
#[allow(unreachable_pub)]
mod comment_links;
#[allow(unreachable_pub)]
mod documents;
#[allow(unreachable_pub)]
mod grant_diagnostics;
#[allow(unreachable_pub)]
mod identity;
#[allow(unreachable_pub)]
mod integration_config;
#[allow(unreachable_pub)]
mod lifecycle;
mod permissions;
#[allow(unreachable_pub)]
mod s3_attachment_store;
#[allow(unreachable_pub)]
pub(crate) mod search;
#[allow(unreachable_pub)]
mod search_index_queue;
#[allow(unreachable_pub)]
mod security_audit;
#[allow(unreachable_pub)]
mod semantic_indexer;
#[allow(unreachable_pub)]
mod semantic_search;
#[allow(unreachable_pub)]
mod tags;
#[allow(unreachable_pub)]
mod webhook_delivery;
#[allow(unreachable_pub)]
mod webhook_subscription;
#[allow(unreachable_pub)]
mod workspace_attachments;
#[allow(unreachable_pub)]
mod workspace_core;

pub use identity::{
    ActivationTokenRepo, ApiKey, ApiKeyRepo, NewActivationToken, NewApiKey, NewSession, NewUser,
    PgUiStateRepo, Session, SessionRepo, UiStateRepo, User, UserRepo, UserUiState,
};

pub use attachment_store::DiskAttachmentStore;
pub use comment_attachment_drafts::PgCommentAttachmentDraftRepo;
pub use comment_links::{CommentMutationFault, PgCommentLinkRepo};
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
#[allow(unreachable_pub)]
mod saved_searches;

pub use saved_searches::{PgSavedSearchRepo, SavedSearchRepo};
pub use search::PgSearchRepo;
pub use search_index_queue::{PgSearchIndexQueueRepo, QueuedResource, WorkspaceIndexPlan};
pub use semantic_indexer::{MAX_CHUNK_CHARS, PgSemanticIndexer};
pub use semantic_search::{PgSemanticIndexWriter, PgSemanticSearchRepo, semantic_search_sql};
pub use tags::{PgTagRepo, TagRepo};
#[allow(unreachable_pub)]
mod task_views;
pub use webhook_delivery::PgWebhookDeliveryRepo;
pub use webhook_subscription::{PgWebhookSubscriptionRepo, WebhookSubscriptionPatch};

pub use task_views::{PgTaskViewRepo, TaskViewRepo};
#[allow(unreachable_pub)]
mod status_templates;

pub use status_templates::{
    PgStatusTemplateRepo, StatusTemplateRepo, list_templates_for_workspace,
};
#[allow(unreachable_pub)]
mod platform_status_templates;

pub use platform_status_templates::{PgPlatformStatusTemplateRepo, PlatformStatusTemplateRepo};

pub use automation_rule::{AutomationRulePatch, PgAutomationRuleRepo};
pub use integration_config::PgIntegrationConfigRepo;
pub use lifecycle::{NewPurgeOperation, PgPurgeOperationRepo};
