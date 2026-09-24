//! Dual-write of Acta's V1 access management to V2 rows (`v2-e7-s4`,
//! ACTA-WS-2..4).
//!
//! Every V1 write that changes who may do what in a workspace (a membership
//! added, re-roled or removed; a permission grant created or revoked; a
//! project's visibility set) also writes the V2 rows the cutover will read,
//! inside the same transaction as the V1 row:
//!
//! - `owner` membership: `workspaces.owner_principal_id`, a grant of the
//!   custom role `acta:workspace-owner` on the workspace, and a projection
//!   row; `admin`: a grant of the custom role `acta:workspace-admin` on the
//!   workspace and a projection row; `member`: a projection row only (the
//!   `members` principal set is derived from `workspace_memberships`, so
//!   membership needs no grant). Membership grants are custom roles and
//!   shares are built-in roles, so the two never collide on the workspace.
//! - A V1 permission grant `(subject, resource, role)`: a V2 grant of
//!   `role@1` on the same `ResourceRef`, the subject being the user's
//!   principal, the key's principal (a personal key already carries its
//!   owner's), or the group.
//! - Project visibility `workspace(role)`: a grant of `role@1` on the
//!   project to the workspace's `members` set; `private` (and `public`,
//!   MIG-4) revokes it.
//!
//! V1 keeps deciding every request. The V2 delegation check (GRANT-3
//! through `can_delegate`) runs in audit mode only, detached after the
//! commit: a would-be refusal is logged and counted, never returned,
//! because the actor's V2 authority lags V1's until the backfill
//! (D-E7S4-5).

use std::collections::BTreeSet;
use std::time::Duration;

use metrics::counter;
use sea_orm::ConnectionTrait;
use uuid::Uuid;

use atlas_acta::entities::identity::MemberRole;
use atlas_acta::ids::WorkspaceId;
use atlas_acta::permissions::{Visibility, VisibilityRole};
use atlas_acta_postgres::repos::identity::{PgWorkspaceMemberProjection, PgWorkspaceRepo};
use atlas_core::ids::{ActionId, PrincipalSetId, ResourceRef};
use atlas_core::principal::{ApiKeyId, GroupId, Principal, UserId};
use atlas_core::registry::{ComponentId, Registry};
use atlas_custos::entities::authorization::{
    CustomRole, GrantAuthority, GrantId, GrantRecord, NewCustomRole, NewGrantRecord, RoleId,
    SubjectRecord, TargetRecord,
};
use atlas_custos::entities::permissions::ResourceRole;
use atlas_custos::eval::{DelegationRefused, can_delegate};
use atlas_custos::ids::PrincipalId;
use atlas_custos_postgres::repos::authorization::{PgGrantV2Repo, PgRoleRepo};
use atlas_postgres::db_err;
use sea_orm::DbErr;

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    authz::v2_caller::{Caller, caller, question_actor},
    authz::v2_service::{resolve_grant_actions, validation_catalog},
    error::ApiError,
    persistence::repos::api_key_principal,
    state::AppState,
};

/// The prefix of the membership role names. The Custos role routes reserve
/// it, so no admin can create, rename into or rename out of a membership
/// role the dual-write finds by name.
pub const MEMBERSHIP_ROLE_PREFIX: &str = "acta:workspace-";

/// The custom role every workspace owner is granted on the workspace
/// (Custos D1): admin@1's Acta actions plus transfer and delete.
pub const OWNER_ROLE_NAME: &str = "acta:workspace-owner";

/// The custom role every workspace admin is granted on the workspace:
/// admin@1's Acta actions. A custom role rather than `admin@1`, so an admin
/// membership and a built-in share on the workspace never share a row.
pub const ADMIN_ROLE_NAME: &str = "acta:workspace-admin";

/// Counter of V2 delegation checks that would have refused a V1-valid
/// write, labelled `operation` and `reason`.
pub const WOULD_REFUSE_TOTAL: &str = "atlas_v2_access_would_refuse_total";

/// How long the detached audit-mode delegation check may run before it is
/// counted as `unavailable`.
const AUDIT_TIMEOUT: Duration = Duration::from_millis(100);

const PRODUCT: &str = "acta";
const BUILTIN_VERSION: u32 = 1;
const SOURCE_MEMBERSHIP: &str = "membership";

/// `acta::workspace::<id>`.
pub fn workspace_ref(workspace: WorkspaceId) -> Result<ResourceRef, ApiError> {
    ResourceRef::new(PRODUCT, "workspace", &workspace.0.to_string()).map_err(|e| {
        ApiError::Internal {
            message: format!("workspace {} has no valid resource ref: {e}", workspace.0),
        }
    })
}

/// `acta::workspace::<id>::members`, the set every workspace member
/// belongs to (D-E7-4).
pub fn members_set(workspace: WorkspaceId) -> Result<PrincipalSetId, ApiError> {
    PrincipalSetId::new(workspace_ref(workspace)?, "members").map_err(|e| ApiError::Internal {
        message: format!("workspace {} has no valid members set: {e}", workspace.0),
    })
}

/// The built-in role a V1 resource role stands for.
pub fn builtin_authority(role: ResourceRole) -> GrantAuthority {
    let name = match role {
        ResourceRole::Viewer => "viewer",
        ResourceRole::Editor => "editor",
        ResourceRole::Admin => "admin",
    };

    GrantAuthority::Builtin {
        name: name.to_string(),
        version: BUILTIN_VERSION,
    }
}

/// The grant a project's visibility stands for, `None` when it grants
/// nothing (`private`, and `public` until MIG-4 lands).
pub fn visibility_authority(visibility: &Visibility) -> Option<GrantAuthority> {
    match visibility {
        Visibility::Workspace(role) => Some(builtin_authority(match role {
            VisibilityRole::Viewer => ResourceRole::Viewer,
            VisibilityRole::Editor => ResourceRole::Editor,
        })),
        Visibility::Private | Visibility::Public(_) => None,
    }
}

/// The actions of `acta:workspace-admin`: admin@1's Acta actions (never
/// Custos actions, GRANT-5), sorted and deduplicated.
pub fn admin_role_actions(registry: &Registry) -> Result<Vec<ActionId>, ApiError> {
    Ok(admin_acta_actions(registry)?.into_iter().collect())
}

/// The actions of `acta:workspace-owner`: admin@1's Acta actions (never
/// Custos actions, GRANT-5) plus `workspace::transfer` and
/// `workspace::delete`, sorted and deduplicated.
pub fn owner_role_actions(registry: &Registry) -> Result<Vec<ActionId>, ApiError> {
    let mut actions = admin_acta_actions(registry)?;
    for extra in ["acta::workspace::transfer", "acta::workspace::delete"] {
        actions.insert(extra.parse().map_err(|_| ApiError::Internal {
            message: format!("`{extra}` is not a valid action id"),
        })?);
    }

    Ok(actions.into_iter().collect())
}

fn admin_acta_actions(registry: &Registry) -> Result<BTreeSet<ActionId>, ApiError> {
    let acta = ComponentId::new(PRODUCT)
        .ok()
        .and_then(|id| registry.get(&id))
        .ok_or_else(|| ApiError::Internal {
            message: "the registry declares no acta component".into(),
        })?;
    let admin = acta
        .authorization
        .role_definitions_v2
        .iter()
        .filter(|role| role.name == "admin")
        .max_by_key(|role| role.version)
        .ok_or_else(|| ApiError::Internal {
            message: "acta declares no versioned admin role".into(),
        })?;

    Ok(admin
        .actions
        .iter()
        .filter(|action| action.product() == PRODUCT)
        .cloned()
        .collect())
}

/// The V2 subject of a V1 grantee: the user's principal, the key's
/// principal (a personal key carries its owner's), or the group.
pub async fn subject_of(
    state: &AppState,
    user_id: Option<UserId>,
    api_key_id: Option<ApiKeyId>,
    group_id: Option<GroupId>,
) -> Result<SubjectRecord, ApiError> {
    match (user_id, api_key_id, group_id) {
        (Some(user), _, _) => Ok(SubjectRecord::Principal(PrincipalId::from(user))),
        (_, Some(key), _) => api_key_principal(&state.db, key)
            .await
            .map_err(ApiError::Domain)?
            .map(SubjectRecord::Principal)
            .ok_or_else(|| ApiError::Internal {
                message: "the api key names no principal".into(),
            }),
        (_, _, Some(group)) => Ok(SubjectRecord::Group(group)),
        (None, None, None) => Err(ApiError::Internal {
            message: "a grant needs a grantee".into(),
        }),
    }
}

/// The principal a V1 actor writes V2 rows as (`created_by`).
pub async fn created_by(state: &AppState, actor: &Principal) -> Result<PrincipalId, ApiError> {
    match actor {
        Principal::User(user) => Ok(PrincipalId::from(*user)),
        Principal::ApiKey(key) => api_key_principal(&state.db, *key)
            .await
            .map_err(ApiError::Domain)?
            .ok_or_else(|| ApiError::Internal {
                message: "the api key names no principal".into(),
            }),
        Principal::Group(_) => Err(ApiError::Forbidden {
            message: "groups cannot be grant actors".into(),
        }),
    }
}

fn internal(error: DbErr) -> ApiError {
    ApiError::Domain(db_err(error))
}

/// The membership role `name` of the product, created on first use. The
/// creation tolerates a concurrent one without aborting the caller's
/// transaction (`find_or_create_in`).
async fn membership_role<C: ConnectionTrait>(
    conn: &C,
    name: &str,
    actions: Vec<ActionId>,
    created_by: PrincipalId,
) -> Result<CustomRole, ApiError> {
    PgRoleRepo::find_or_create_in(
        conn,
        NewCustomRole {
            id: RoleId::new(),
            product: PRODUCT.to_string(),
            name: name.to_string(),
            actions,
            created_by,
        },
    )
    .await
    .map_err(ApiError::Domain)
}

/// The workspace-level authority a membership role maps to, `None` for a
/// plain member.
async fn membership_authority<C: ConnectionTrait>(
    conn: &C,
    registry: &Registry,
    role: MemberRole,
    created_by: PrincipalId,
) -> Result<Option<GrantAuthority>, ApiError> {
    let role = match role {
        MemberRole::Owner => {
            membership_role(
                conn,
                OWNER_ROLE_NAME,
                owner_role_actions(registry)?,
                created_by,
            )
            .await?
        }
        MemberRole::Admin => {
            membership_role(
                conn,
                ADMIN_ROLE_NAME,
                admin_role_actions(registry)?,
                created_by,
            )
            .await?
        }
        MemberRole::Member => return Ok(None),
    };

    Ok(Some(GrantAuthority::CustomRole(role.id)))
}

/// The ids of the membership roles that exist so far. A role not created
/// yet has no grant to revoke.
async fn membership_role_ids<C: ConnectionTrait>(conn: &C) -> Result<Vec<RoleId>, ApiError> {
    let mut ids = Vec::new();
    for name in [OWNER_ROLE_NAME, ADMIN_ROLE_NAME] {
        if let Some(role) = PgRoleRepo::find_by_name_in(conn, PRODUCT, name)
            .await
            .map_err(ApiError::Domain)?
        {
            ids.push(role.id);
        }
    }

    Ok(ids)
}

async fn create_grant<C: ConnectionTrait>(
    conn: &C,
    subject: SubjectRecord,
    target: ResourceRef,
    authority: GrantAuthority,
    created_by: PrincipalId,
) -> Result<GrantRecord, ApiError> {
    PgGrantV2Repo::create_in(
        conn,
        NewGrantRecord {
            id: GrantId::new(),
            subject,
            target: TargetRecord::Ref(target),
            authority,
            created_by,
        },
    )
    .await
    .map_err(ApiError::Domain)
}

/// Revokes every grant of `subject` on `target` whose authority satisfies
/// `keep_out`, returning how many rows went.
async fn revoke_grants<C: ConnectionTrait>(
    conn: &C,
    subject: &SubjectRecord,
    target: &ResourceRef,
    keep_out: impl Fn(&GrantAuthority) -> bool,
) -> Result<usize, ApiError> {
    let existing =
        PgGrantV2Repo::find_matching_in(conn, subject, &TargetRecord::Ref(target.clone()))
            .await
            .map_err(ApiError::Domain)?;

    let mut revoked = 0;
    for grant in existing.iter().filter(|grant| keep_out(&grant.authority)) {
        PgGrantV2Repo::delete_in(conn, grant.id)
            .await
            .map_err(ApiError::Domain)?;
        revoked += 1;
    }

    Ok(revoked)
}

/// Whether `authority` is one of the membership roles (`membership_roles`,
/// the ids of `acta:workspace-owner` and `acta:workspace-admin`). Only those
/// are touched by membership writes; built-in grants made through the share
/// routes on the workspace stay.
fn is_membership_authority(authority: &GrantAuthority, membership_roles: &[RoleId]) -> bool {
    match authority {
        GrantAuthority::CustomRole(id) => membership_roles.contains(id),
        GrantAuthority::Builtin { .. } | GrantAuthority::Actions(_) => false,
    }
}

/// After a membership row for `user` in `workspace` was written with
/// `role`: replaces the principal's workspace role grant, refreshes the
/// owner metadata and the projection. Returns the role grant it created,
/// if the role maps to one.
pub async fn membership_written<C: ConnectionTrait>(
    conn: &C,
    registry: &Registry,
    workspace: WorkspaceId,
    user: UserId,
    role: MemberRole,
    created_by: PrincipalId,
) -> Result<Option<GrantRecord>, ApiError> {
    let principal = PrincipalId::from(user);
    let subject = SubjectRecord::Principal(principal);
    let target = workspace_ref(workspace)?;

    let membership_roles = membership_role_ids(conn).await?;
    revoke_grants(conn, &subject, &target, |authority| {
        is_membership_authority(authority, &membership_roles)
    })
    .await?;
    let mut created = None;
    if let Some(authority) = membership_authority(conn, registry, role.clone(), created_by).await? {
        created = Some(create_grant(conn, subject, target, authority, created_by).await?);
    }

    PgWorkspaceMemberProjection::upsert_in(
        conn,
        workspace,
        principal.0,
        role.clone(),
        SOURCE_MEMBERSHIP,
    )
    .await
    .map_err(ApiError::Domain)?;

    refresh_owner(conn, workspace, principal.0, role).await?;

    Ok(created)
}

/// After the membership row for `user` in `workspace` was removed: revokes
/// the principal's workspace role grant, the projection row, and the owner
/// metadata when it named them.
pub async fn membership_removed<C: ConnectionTrait>(
    conn: &C,
    workspace: WorkspaceId,
    user: UserId,
) -> Result<(), ApiError> {
    let principal = PrincipalId::from(user);
    let subject = SubjectRecord::Principal(principal);

    let membership_roles = membership_role_ids(conn).await?;
    revoke_grants(conn, &subject, &workspace_ref(workspace)?, |authority| {
        is_membership_authority(authority, &membership_roles)
    })
    .await?;
    PgWorkspaceMemberProjection::remove_in(conn, workspace, principal.0)
        .await
        .map_err(ApiError::Domain)?;

    refresh_owner(conn, workspace, principal.0, MemberRole::Member).await
}

/// Keeps `owner_principal_id` pointing at an owner: a principal that just
/// became owner takes it; a principal that stopped being owner releases it
/// to the oldest remaining owner membership, or to nothing.
async fn refresh_owner<C: ConnectionTrait>(
    conn: &C,
    workspace: WorkspaceId,
    principal: Uuid,
    role: MemberRole,
) -> Result<(), ApiError> {
    if role == MemberRole::Owner {
        return PgWorkspaceRepo::set_owner_principal_in(conn, workspace, Some(principal))
            .await
            .map_err(ApiError::Domain);
    }

    let current = PgWorkspaceRepo::owner_principal_in(conn, workspace)
        .await
        .map_err(ApiError::Domain)?;
    if current != Some(principal) {
        return Ok(());
    }

    let successor = oldest_owner(conn, workspace).await?;
    PgWorkspaceRepo::set_owner_principal_in(conn, workspace, successor)
        .await
        .map_err(ApiError::Domain)
}

async fn oldest_owner<C: ConnectionTrait>(
    conn: &C,
    workspace: WorkspaceId,
) -> Result<Option<Uuid>, ApiError> {
    use sea_orm::{ColumnTrait, EntityTrait, QueryFilter, QueryOrder, QuerySelect};

    use atlas_acta_postgres::entities::identity::membership;

    membership::Entity::find()
        .select_only()
        .column(membership::Column::UserId)
        .filter(membership::Column::WorkspaceId.eq(workspace.0))
        .filter(membership::Column::Role.eq(MemberRole::Owner.as_str()))
        .order_by_asc(membership::Column::CreatedAt)
        .into_tuple::<Uuid>()
        .one(conn)
        .await
        .map_err(internal)
}

/// After a V1 permission grant `(subject, resource, role)` was written:
/// the same grant as `role@1` on the same reference. A repeated V1 upsert
/// replaces the previous built-in twin (same subject and target) rather
/// than adding one; membership grants are custom roles and stay.
pub async fn grant_written<C: ConnectionTrait>(
    conn: &C,
    subject: SubjectRecord,
    resource: ResourceRef,
    role: ResourceRole,
    created_by: PrincipalId,
) -> Result<GrantRecord, ApiError> {
    revoke_grants(conn, &subject, &resource, is_builtin).await?;

    create_grant(conn, subject, resource, builtin_authority(role), created_by).await
}

/// After a V1 permission grant was deleted: revokes its built-in V2 twin
/// (same subject and target).
pub async fn grant_revoked<C: ConnectionTrait>(
    conn: &C,
    subject: SubjectRecord,
    resource: ResourceRef,
) -> Result<usize, ApiError> {
    revoke_grants(conn, &subject, &resource, is_builtin).await
}

fn is_builtin(authority: &GrantAuthority) -> bool {
    matches!(authority, GrantAuthority::Builtin { .. })
}

/// After a project's visibility was written: the `members` set of the
/// workspace holds `role@1` on the project for `workspace(role)`, and
/// nothing otherwise.
pub async fn visibility_written<C: ConnectionTrait>(
    conn: &C,
    workspace: WorkspaceId,
    resource: ResourceRef,
    visibility: &Visibility,
    created_by: PrincipalId,
) -> Result<Option<GrantRecord>, ApiError> {
    let subject = SubjectRecord::PrincipalSet(members_set(workspace)?);

    revoke_grants(conn, &subject, &resource, is_builtin).await?;
    match visibility_authority(visibility) {
        Some(authority) => create_grant(conn, subject, resource, authority, created_by)
            .await
            .map(Some),
        None => Ok(None),
    }
}

/// The V2 delegation check in audit mode (D-E7S4-5): would GRANT-3 have
/// refused `actor` creating `candidate`? Runs detached, after the write
/// committed, so it never delays the response; an evaluation that does not
/// finish within [`AUDIT_TIMEOUT`] counts as `unavailable`. A refusal or an
/// unavailable evaluator is logged under `authz.v2_access` and counted,
/// never returned. Platform admins and root skip the check as they do on
/// the admin route.
pub fn audit_delegation(
    state: &AppState,
    operation: &'static str,
    actor: &Principal,
    candidate: &GrantRecord,
) {
    let actor = match actor {
        Principal::User(user) => AuthPrincipal::User(*user),
        Principal::ApiKey(key) => AuthPrincipal::ApiKey(*key),
        Principal::Group(_) => return,
    };

    let state = state.clone();
    let candidate = candidate.clone();
    tokio::spawn(async move {
        let outcome = tokio::time::timeout(AUDIT_TIMEOUT, would_refuse(&state, actor, &candidate))
            .await
            .unwrap_or_else(|_| {
                Err(ApiError::Internal {
                    message: format!("the delegation check exceeded {AUDIT_TIMEOUT:?}"),
                })
            });
        record_would_refuse(operation, &candidate, outcome);
    });
}

fn record_would_refuse(
    operation: &'static str,
    candidate: &GrantRecord,
    outcome: Result<Option<&'static str>, ApiError>,
) {
    let reason = match outcome {
        Ok(None) => return,
        Ok(Some(reason)) => reason,
        Err(error) => {
            tracing::warn!(
                target: "authz.v2_access",
                operation,
                target = %candidate.target.canonical(),
                error = ?error,
                "v2_access.would_refuse: delegation check unavailable"
            );
            "unavailable"
        }
    };

    tracing::info!(
        target: "authz.v2_access",
        operation,
        target = %candidate.target.canonical(),
        reason,
        "v2_access.would_refuse"
    );
    counter!(WOULD_REFUSE_TOTAL, "operation" => operation, "reason" => reason).increment(1);
}

async fn would_refuse(
    state: &AppState,
    actor: AuthPrincipal,
    candidate: &GrantRecord,
) -> Result<Option<&'static str>, ApiError> {
    let caller = caller(state, actor).await?;
    if let Caller::Session { admin: true, .. } = &caller {
        return Ok(None);
    }
    let actor = question_actor(state, caller).await?;
    if actor.is_root {
        return Ok(None);
    }

    let TargetRecord::Ref(target) = &candidate.target else {
        return Ok(Some("not_exact_target"));
    };

    let catalog = validation_catalog(&state.registry).map_err(|e| ApiError::Internal {
        message: format!("registry authorization declarations do not form a catalog: {e}"),
    })?;
    let granted = resolve_grant_actions(&*state.db, &catalog, candidate, |e| ApiError::Internal {
        message: format!("the dual-written grant does not resolve in the catalog: {e}"),
    })
    .await?;

    let effective = state
        .authorization
        .effective_actions(&actor, target)
        .await
        .map_err(crate::authz::v2_caller::unavailable)?;

    Ok(match can_delegate(&effective, &granted) {
        Ok(()) => None,
        Err(DelegationRefused::MissingGrantCreate) => Some("missing_grant_create"),
        Err(DelegationRefused::BeyondAuthority { .. }) => Some("beyond_authority"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_refs_and_member_sets_are_canonical() {
        let id = Uuid::now_v7();
        let workspace = WorkspaceId(id);

        assert_eq!(
            workspace_ref(workspace).unwrap().to_string(),
            format!("acta::workspace::{id}")
        );
        assert_eq!(
            members_set(workspace).unwrap().to_string(),
            format!("acta::workspace::{id}::members")
        );
    }

    #[test]
    fn membership_role_names_share_the_reserved_prefix() {
        assert!(OWNER_ROLE_NAME.starts_with(MEMBERSHIP_ROLE_PREFIX));
        assert!(ADMIN_ROLE_NAME.starts_with(MEMBERSHIP_ROLE_PREFIX));
    }

    #[test]
    fn resource_roles_map_to_version_one_builtins() {
        for (role, name) in [
            (ResourceRole::Viewer, "viewer"),
            (ResourceRole::Editor, "editor"),
            (ResourceRole::Admin, "admin"),
        ] {
            assert_eq!(
                builtin_authority(role),
                GrantAuthority::Builtin {
                    name: name.to_string(),
                    version: 1
                }
            );
        }
    }

    #[test]
    fn visibility_grants_the_members_set_only_for_workspace_visibility() {
        assert_eq!(
            visibility_authority(&Visibility::Workspace(VisibilityRole::Viewer)),
            Some(builtin_authority(ResourceRole::Viewer))
        );
        assert_eq!(
            visibility_authority(&Visibility::Workspace(VisibilityRole::Editor)),
            Some(builtin_authority(ResourceRole::Editor))
        );
        assert_eq!(visibility_authority(&Visibility::Private), None);
        assert_eq!(
            visibility_authority(&Visibility::Public(VisibilityRole::Editor)),
            None,
            "public is treated as private until MIG-4"
        );
    }

    #[test]
    fn membership_authorities_are_the_membership_roles_only() {
        let owner = RoleId::new();
        let admin = RoleId::new();
        let roles = [owner, admin];

        assert!(is_membership_authority(
            &GrantAuthority::CustomRole(owner),
            &roles
        ));
        assert!(is_membership_authority(
            &GrantAuthority::CustomRole(admin),
            &roles
        ));
        assert!(
            !is_membership_authority(&GrantAuthority::CustomRole(RoleId::new()), &roles),
            "another custom role granted on the workspace is not a membership grant"
        );
        for role in [
            ResourceRole::Viewer,
            ResourceRole::Editor,
            ResourceRole::Admin,
        ] {
            assert!(
                !is_membership_authority(&builtin_authority(role), &roles),
                "a built-in share on the workspace is not a membership grant"
            );
        }
        assert!(!is_membership_authority(
            &GrantAuthority::Actions(vec![]),
            &roles
        ));
    }

    #[test]
    fn the_owner_role_is_admin_plus_transfer_and_delete_without_custos_actions() {
        let registry = atlas_core::registry::build(crate::reg5::reg5_component_entries(
            crate::reg5::StorageBackend::Filesystem,
        ))
        .expect("REG-5 entries build");

        let actions = owner_role_actions(&registry).expect("owner role actions");

        assert!(actions.iter().all(|action| action.product() == "acta"));
        for expected in [
            "acta::workspace::transfer",
            "acta::workspace::delete",
            "acta::workspace::manage_members",
            "acta::document::read",
        ] {
            assert!(
                actions.iter().any(|action| action.to_string() == expected),
                "missing {expected}"
            );
        }
        let mut sorted = actions.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(actions, sorted, "sorted and deduplicated");
    }

    #[test]
    fn the_admin_role_is_admin_acta_actions_without_transfer_or_delete() {
        let registry = atlas_core::registry::build(crate::reg5::reg5_component_entries(
            crate::reg5::StorageBackend::Filesystem,
        ))
        .expect("REG-5 entries build");

        let admin = admin_role_actions(&registry).expect("admin role actions");
        let owner = owner_role_actions(&registry).expect("owner role actions");

        assert!(admin.iter().all(|action| action.product() == "acta"));
        assert!(
            admin
                .iter()
                .any(|action| action.to_string() == "acta::workspace::manage_members")
        );
        for owner_only in ["acta::workspace::transfer", "acta::workspace::delete"] {
            assert!(
                !admin.iter().any(|action| action.to_string() == owner_only),
                "{owner_only} is owner-only"
            );
        }
        assert!(admin.iter().all(|action| owner.contains(action)));
    }
}
