pub mod authorized;
#[allow(
    dead_code,
    reason = "WU2-C1a establishes the server-only projection boundary before route wiring"
)]
pub(crate) mod batch_authorization;
#[cfg(test)]
mod batch_authorization_tests;
pub mod extractors;

pub use authorized::{
    AdminMin, AdminMinAgentEditor, Authorized, BoardRes, BoardsCreate, BoardsDelete, BoardsRead,
    BoardsUpdate, ConfigCreate, ConfigDelete, ConfigRead, ConfigUpdate, DocsCreate, DocsDelete,
    DocsRead, DocsUpdate, EditorMin, FolderRes, FoldersCreate, FoldersDelete, FoldersRead,
    FoldersUpdate, GrantsRead, MinRole, NoScope, ProjectRes, ProjectsCreate, ProjectsDelete,
    ProjectsRead, ProjectsUpdate, ReadScopeSet, RequiredScope, TaskRes, TasksCreate, TasksDelete,
    TasksRead, TasksUpdate, ViewerMin, WebhooksCreate, WebhooksDelete, WebhooksRead,
    WebhooksUpdate, WorkspaceRes, authorize_board_destination, authorize_folder_destination,
    build_board_chain, build_document_chain, build_folder_chain, enforce_api_key_scope,
    resolve_effective_role, resolve_folder_ancestry,
};
pub use extractors::{
    CallerClass, RequireRoot, RequireUserAdmin, WorkspaceAccess, WorkspaceMember,
    WorkspaceOwnerOrAdmin,
};
