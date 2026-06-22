#[allow(unreachable_pub)]
mod attachment_store;
#[allow(unreachable_pub)]
mod boards_tasks;
#[allow(unreachable_pub)]
mod documents;
#[allow(unreachable_pub)]
mod identity;
#[allow(unreachable_pub)]
mod permissions;
#[allow(unreachable_pub)]
mod search;
#[allow(unreachable_pub)]
mod tags;
#[allow(unreachable_pub)]
mod workspace_core;

pub use identity::{
    ApiKey, ApiKeyRepo, MembershipRepo, NewApiKey, NewSession, NewUser, NewWorkspace, PgApiKeyRepo,
    PgMembershipRepo, PgSessionRepo, PgUiStateRepo, PgUserRepo, PgWorkspaceRepo, Session,
    SessionRepo, UiStateRepo, User, UserRepo, UserUiState, Workspace, WorkspaceRepo,
};

pub use attachment_store::DiskAttachmentStore;
pub use boards_tasks::{
    BoardRepo, PgBoardRepo, PgTaskActivityRepo, PgTaskAssigneeRepo, PgTaskChecklistRepo,
    PgTaskReferenceRepo, PgTaskRepo, TaskActivityRepo, TaskAssigneeRepo, TaskChecklistRepo,
    TaskReferenceRepo, TaskRepo, resequence_column,
};
pub use documents::{
    AttachmentRepo, DocumentLinkRepo, DocumentRepo, PgAttachmentRepo, PgDocumentLinkRepo,
    PgDocumentRepo,
};
pub use workspace_core::{
    FolderRepo, PgFolderRepo, PgProjectRepo, PgPropertyDefinitionRepo, ProjectRepo,
    PropertyDefinitionRepo,
};

pub use permissions::{PermissionGrantRepo, PgPermissionGrantRepo};
#[allow(unreachable_pub)]
mod saved_searches;

pub use saved_searches::{PgSavedSearchRepo, SavedSearchRepo};
pub use search::PgSearchRepo;
pub use tags::{PgTagRepo, TagRepo};
#[allow(unreachable_pub)]
mod task_views;

pub use task_views::{PgTaskViewRepo, TaskViewRepo};
#[allow(unreachable_pub)]
mod status_templates;

pub use status_templates::{
    PgStatusTemplateRepo, StatusTemplateRepo, list_templates_for_workspace,
};
