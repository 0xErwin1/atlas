//! The permission-grant domain: `PermissionGrant`, its identity, and the
//! `ResourceRole` vocabulary shared with `atlas_server::authz::policy::resolve()`.
//!
//! Relocated from `atlas_server::authz::policy` (S3c T6.9, D1). The grant no
//! longer carries typed Acta resource ids (`project_id`/`folder_id`/
//! `document_id`/`board_id`) or an Acta `workspace_id`: `resource_ref` is the
//! opaque `atlas_core::ids::ResourceRef` that `atlas_acta`'s codec produces at
//! the `atlas_server` composition boundary, and `workspace_id` is the same
//! `WorkspaceScope` pattern already used by `GroupRepo`/`ApiKeyRepo`. This
//! crate never depends on `atlas_acta`.

use crate::WorkspaceScope;
use crate::ids::{ApiKeyId, GroupId, UserId};
use atlas_core::ids::ResourceRef;
use chrono::{DateTime, Utc};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionGrantId(pub Uuid);

impl PermissionGrantId {
    pub fn new() -> Self {
        Self(Uuid::now_v7())
    }
}

impl Default for PermissionGrantId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub id: PermissionGrantId,
    pub workspace_id: WorkspaceScope,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub resource_ref: ResourceRef,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPermissionGrant {
    pub workspace_id: WorkspaceScope,
    pub user_id: Option<UserId>,
    pub api_key_id: Option<ApiKeyId>,
    pub group_id: Option<GroupId>,
    pub resource_ref: ResourceRef,
    pub role: ResourceRole,
    pub created_by_user_id: Option<UserId>,
    pub created_by_api_key_id: Option<ApiKeyId>,
}

/// `ResourceRole`, relocated from `atlas_domain` via
/// `atlas_server::authz::policy` (S2e, S3c). `resolve()` and the
/// grant-authorization guards stay in `atlas_server::authz::policy`, the only
/// other consumer of this type, and import it from here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResourceRole {
    Viewer,
    Editor,
    Admin,
}
