#[allow(unreachable_pub)]
mod attachment_store;
#[allow(unreachable_pub)]
mod automation_rule;
#[allow(unreachable_pub)]
mod boards_tasks;
#[allow(unreachable_pub)]
mod documents;
#[allow(unreachable_pub)]
mod identity;
#[allow(unreachable_pub)]
mod integration_config;
#[allow(unreachable_pub)]
pub(crate) mod outbox;
#[allow(unreachable_pub)]
mod permissions;
#[allow(unreachable_pub)]
mod s3_attachment_store;
#[allow(unreachable_pub)]
mod search;
#[allow(unreachable_pub)]
mod security_audit;
#[allow(unreachable_pub)]
mod tags;
#[allow(unreachable_pub)]
mod webhook_delivery;
#[allow(unreachable_pub)]
mod webhook_subscription;
#[allow(unreachable_pub)]
mod workspace_core;

pub use identity::{
    ActivationTokenRepo, ApiKey, ApiKeyRepo, MembershipRepo, NewActivationToken, NewApiKey,
    NewSession, NewUser, NewWorkspace, PgActivationTokenRepo, PgApiKeyRepo, PgMembershipRepo,
    PgSessionRepo, PgUiStateRepo, PgUserRepo, PgWorkspaceRepo, Session, SessionRepo, UiStateRepo,
    User, UserRepo, UserUiState, Workspace, WorkspaceRepo,
};

pub use attachment_store::DiskAttachmentStore;
pub use boards_tasks::{
    BoardRepo, PgBoardRepo, PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo,
    PgTaskReferenceRepo, PgTaskRepo, TaskActivityRepo, TaskAssigneeRepo, TaskChecklistRepo,
    TaskReferenceRepo, TaskRepo, resequence_column,
};
pub use documents::{
    AttachmentRepo, DocumentLinkRepo, DocumentRepo, PgAttachmentRepo, PgDocumentLinkRepo,
    PgDocumentRepo, create_in as doc_create_in, move_to_in as doc_move_to_in,
    rename_in as doc_rename_in, soft_delete_in as doc_soft_delete_in,
    update_content_in as doc_update_content_in,
};
pub use s3_attachment_store::{S3AttachmentStore, S3Config};
pub use workspace_core::{
    FolderRepo, PgFolderRepo, PgProjectRepo, PgPropertyDefinitionRepo, ProjectRepo,
    PropertyDefinitionRepo,
};

pub use permissions::{PermissionGrantRepo, PgGroupRepo, PgPermissionGrantRepo};
pub use security_audit::{PgSecurityAuditRepo, SecurityAuditRepoTrait as SecurityAuditRepo};
#[allow(unreachable_pub)]
mod saved_searches;

pub use outbox::PgOutboxRepo;
pub use saved_searches::{PgSavedSearchRepo, SavedSearchRepo};
pub use search::PgSearchRepo;
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

pub use automation_rule::{AutomationRulePatch, PgAutomationRuleRepo};
pub use integration_config::PgIntegrationConfigRepo;
