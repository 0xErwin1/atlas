#[allow(unreachable_pub)]
mod boards_tasks;
#[allow(unreachable_pub)]
mod documents;
#[allow(unreachable_pub)]
mod identity;
#[allow(unreachable_pub)]
mod permissions;
#[allow(unreachable_pub)]
mod workspace_core;

pub use identity::{
    ApiKey, ApiKeyRepo, MembershipRepo, NewApiKey, NewSession, NewUser, NewWorkspace, PgApiKeyRepo,
    PgMembershipRepo, PgSessionRepo, PgUserRepo, PgWorkspaceRepo, Session, SessionRepo, User,
    UserRepo, Workspace, WorkspaceRepo,
};

pub use boards_tasks::{
    BoardRepo, PgBoardRepo, PgTaskReferenceRepo, PgTaskRepo, TaskReferenceRepo, TaskRepo,
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
