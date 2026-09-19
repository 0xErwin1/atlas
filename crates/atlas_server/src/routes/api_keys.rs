use axum::{Json, http::StatusCode, response::IntoResponse};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
pub(crate) struct TopLevelRevokeKeyPath {
    pub(crate) key_id: uuid::Uuid,
}

#[derive(Deserialize)]
pub(crate) struct ApiKeyGrantPath {
    pub(crate) key_id: uuid::Uuid,
    pub(crate) grant_id: uuid::Uuid,
}

use crate::authz::ResourceRole;
use crate::authz::ShareDenied;
use crate::authz::authorize_grant_target;
use atlas_acta::actor::Actor;
use atlas_acta::ids::ProjectId;
use atlas_acta::ids::WorkspaceId;
use atlas_api::{
    dtos::{
        ApiKeyCreated, ApiKeyDto, ApiKeyGrantDto, ApiKeyScope, CreateAgentApiKeyRequest,
        CreatePersonalApiKeyRequest, GrantedByDto, InitialGrantRequest, UpdateApiKeyRequest,
    },
    pagination::{Cursor, Page},
};
use atlas_core::principal::ApiKeyId;
use atlas_core::principal::Principal;
use atlas_core::principal::UserId;
use atlas_custos::capability::Capability;
use atlas_custos::capability::CapabilityAction;
use atlas_custos::capability::CapabilityFamily;
use atlas_custos::entities::identity::ApiKeyType;
use atlas_custos::entities::security_audit::NewSecurityAuditEvent;
use atlas_custos::entities::security_audit::SecurityAction;
use atlas_custos::ids::PrincipalId;
use sea_orm::TransactionTrait;

use crate::{
    auth::{
        middleware::Principal as AuthPrincipal,
        tokens::{generate_api_key_of_kind, hash_token},
    },
    authz::policy::{NewPermissionGrant, PermissionGrant, PermissionGrantId},
    error::ApiError,
    persistence::repos::{
        ApiKeyRepo, NewApiKey, PermissionGrantRepo, PgProjectRepo, ProjectRepo, UserRepo,
    },
    state::AppState,
};
use atlas_acta_postgres::repos::boards_tasks::PgTaskAssigneeRepo;
use atlas_acta_postgres::repos::identity::{PgWorkspaceRepo, WorkspaceRepo};
use atlas_custos::entities::identity::ApiKeyKind;
use atlas_custos_postgres::repos::identity::{PgApiKeyRepo, PgUserRepo};
use atlas_custos_postgres::repos::permissions::PgPermissionGrantRepo;
use atlas_custos_postgres::repos::security_audit::PgSecurityAuditRepo;

#[derive(Deserialize)]
pub(crate) struct PaginationQuery {
    cursor: Option<String>,
    limit: Option<u32>,
}

fn parse_key_type(s: Option<&str>) -> Result<ApiKeyType, ApiError> {
    match s.unwrap_or("agent") {
        "agent" => Ok(ApiKeyType::Agent),
        "cli" => Ok(ApiKeyType::Cli),
        "bot" => Ok(ApiKeyType::Bot),
        "integration" => Ok(ApiKeyType::Integration),
        other => Err(ApiError::InvalidInput {
            message: format!(
                "invalid key type: {other}; expected 'agent', 'cli', 'bot', or 'integration'"
            ),
        }),
    }
}

fn parse_role(role: &str) -> Result<ResourceRole, ApiError> {
    match role {
        "viewer" => Ok(ResourceRole::Viewer),
        "editor" => Ok(ResourceRole::Editor),
        _ => Err(ApiError::InvalidInput {
            message: format!(
                "invalid role: {role}; expected 'viewer' or 'editor' (agent cap prevents admin)"
            ),
        }),
    }
}

/// Maps a wire scope to its domain capability.
fn capability_from_scope(scope: ApiKeyScope) -> Capability {
    let (family, action) = match scope {
        ApiKeyScope::TasksRead => (CapabilityFamily::Tasks, CapabilityAction::Read),
        ApiKeyScope::TasksCreate => (CapabilityFamily::Tasks, CapabilityAction::Create),
        ApiKeyScope::TasksUpdate => (CapabilityFamily::Tasks, CapabilityAction::Update),
        ApiKeyScope::TasksDelete => (CapabilityFamily::Tasks, CapabilityAction::Delete),
        ApiKeyScope::DocsRead => (CapabilityFamily::Docs, CapabilityAction::Read),
        ApiKeyScope::DocsCreate => (CapabilityFamily::Docs, CapabilityAction::Create),
        ApiKeyScope::DocsUpdate => (CapabilityFamily::Docs, CapabilityAction::Update),
        ApiKeyScope::DocsDelete => (CapabilityFamily::Docs, CapabilityAction::Delete),
        ApiKeyScope::BoardsRead => (CapabilityFamily::Boards, CapabilityAction::Read),
        ApiKeyScope::BoardsCreate => (CapabilityFamily::Boards, CapabilityAction::Create),
        ApiKeyScope::BoardsUpdate => (CapabilityFamily::Boards, CapabilityAction::Update),
        ApiKeyScope::BoardsDelete => (CapabilityFamily::Boards, CapabilityAction::Delete),
        ApiKeyScope::FoldersRead => (CapabilityFamily::Folders, CapabilityAction::Read),
        ApiKeyScope::FoldersCreate => (CapabilityFamily::Folders, CapabilityAction::Create),
        ApiKeyScope::FoldersUpdate => (CapabilityFamily::Folders, CapabilityAction::Update),
        ApiKeyScope::FoldersDelete => (CapabilityFamily::Folders, CapabilityAction::Delete),
        ApiKeyScope::ProjectsRead => (CapabilityFamily::Projects, CapabilityAction::Read),
        ApiKeyScope::ProjectsCreate => (CapabilityFamily::Projects, CapabilityAction::Create),
        ApiKeyScope::ProjectsUpdate => (CapabilityFamily::Projects, CapabilityAction::Update),
        ApiKeyScope::ProjectsDelete => (CapabilityFamily::Projects, CapabilityAction::Delete),
        ApiKeyScope::WebhooksRead => (CapabilityFamily::Webhooks, CapabilityAction::Read),
        ApiKeyScope::WebhooksCreate => (CapabilityFamily::Webhooks, CapabilityAction::Create),
        ApiKeyScope::WebhooksUpdate => (CapabilityFamily::Webhooks, CapabilityAction::Update),
        ApiKeyScope::WebhooksDelete => (CapabilityFamily::Webhooks, CapabilityAction::Delete),
        ApiKeyScope::ConfigRead => (CapabilityFamily::Config, CapabilityAction::Read),
        ApiKeyScope::ConfigCreate => (CapabilityFamily::Config, CapabilityAction::Create),
        ApiKeyScope::ConfigUpdate => (CapabilityFamily::Config, CapabilityAction::Update),
        ApiKeyScope::ConfigDelete => (CapabilityFamily::Config, CapabilityAction::Delete),
        ApiKeyScope::GrantsRead => (CapabilityFamily::Grants, CapabilityAction::Read),
        ApiKeyScope::SavedSearchesRead => (CapabilityFamily::SavedSearches, CapabilityAction::Read),
        ApiKeyScope::SavedSearchesCreate => {
            (CapabilityFamily::SavedSearches, CapabilityAction::Create)
        }
        ApiKeyScope::SavedSearchesUpdate => {
            (CapabilityFamily::SavedSearches, CapabilityAction::Update)
        }
        ApiKeyScope::SavedSearchesDelete => {
            (CapabilityFamily::SavedSearches, CapabilityAction::Delete)
        }
        ApiKeyScope::TaskViewsRead => (CapabilityFamily::TaskViews, CapabilityAction::Read),
        ApiKeyScope::TaskViewsCreate => (CapabilityFamily::TaskViews, CapabilityAction::Create),
        ApiKeyScope::TaskViewsUpdate => (CapabilityFamily::TaskViews, CapabilityAction::Update),
        ApiKeyScope::TaskViewsDelete => (CapabilityFamily::TaskViews, CapabilityAction::Delete),
    };
    Capability { family, action }
}

/// Maps a domain capability to its wire scope.
fn scope_from_capability(cap: Capability) -> ApiKeyScope {
    match (cap.family, cap.action) {
        (CapabilityFamily::Tasks, CapabilityAction::Read) => ApiKeyScope::TasksRead,
        (CapabilityFamily::Tasks, CapabilityAction::Create) => ApiKeyScope::TasksCreate,
        (CapabilityFamily::Tasks, CapabilityAction::Update) => ApiKeyScope::TasksUpdate,
        (CapabilityFamily::Tasks, CapabilityAction::Delete) => ApiKeyScope::TasksDelete,
        (CapabilityFamily::Docs, CapabilityAction::Read) => ApiKeyScope::DocsRead,
        (CapabilityFamily::Docs, CapabilityAction::Create) => ApiKeyScope::DocsCreate,
        (CapabilityFamily::Docs, CapabilityAction::Update) => ApiKeyScope::DocsUpdate,
        (CapabilityFamily::Docs, CapabilityAction::Delete) => ApiKeyScope::DocsDelete,
        (CapabilityFamily::Boards, CapabilityAction::Read) => ApiKeyScope::BoardsRead,
        (CapabilityFamily::Boards, CapabilityAction::Create) => ApiKeyScope::BoardsCreate,
        (CapabilityFamily::Boards, CapabilityAction::Update) => ApiKeyScope::BoardsUpdate,
        (CapabilityFamily::Boards, CapabilityAction::Delete) => ApiKeyScope::BoardsDelete,
        (CapabilityFamily::Folders, CapabilityAction::Read) => ApiKeyScope::FoldersRead,
        (CapabilityFamily::Folders, CapabilityAction::Create) => ApiKeyScope::FoldersCreate,
        (CapabilityFamily::Folders, CapabilityAction::Update) => ApiKeyScope::FoldersUpdate,
        (CapabilityFamily::Folders, CapabilityAction::Delete) => ApiKeyScope::FoldersDelete,
        (CapabilityFamily::Projects, CapabilityAction::Read) => ApiKeyScope::ProjectsRead,
        (CapabilityFamily::Projects, CapabilityAction::Create) => ApiKeyScope::ProjectsCreate,
        (CapabilityFamily::Projects, CapabilityAction::Update) => ApiKeyScope::ProjectsUpdate,
        (CapabilityFamily::Projects, CapabilityAction::Delete) => ApiKeyScope::ProjectsDelete,
        (CapabilityFamily::Webhooks, CapabilityAction::Read) => ApiKeyScope::WebhooksRead,
        (CapabilityFamily::Webhooks, CapabilityAction::Create) => ApiKeyScope::WebhooksCreate,
        (CapabilityFamily::Webhooks, CapabilityAction::Update) => ApiKeyScope::WebhooksUpdate,
        (CapabilityFamily::Webhooks, CapabilityAction::Delete) => ApiKeyScope::WebhooksDelete,
        (CapabilityFamily::Config, CapabilityAction::Read) => ApiKeyScope::ConfigRead,
        (CapabilityFamily::Config, CapabilityAction::Create) => ApiKeyScope::ConfigCreate,
        (CapabilityFamily::Config, CapabilityAction::Update) => ApiKeyScope::ConfigUpdate,
        (CapabilityFamily::Config, CapabilityAction::Delete) => ApiKeyScope::ConfigDelete,
        (CapabilityFamily::Grants, CapabilityAction::Read) => ApiKeyScope::GrantsRead,
        // `grants` is read-only: `Capability::ALL` holds only `grants:read` and
        // `canonical_scopes` filters through `ALL`, so the write actions are
        // never constructed here. This wildcard keeps the match total without a
        // panic (workspace clippy denies `unreachable!`/`panic!` via `-D warnings`).
        (CapabilityFamily::Grants, _) => ApiKeyScope::GrantsRead,
        (CapabilityFamily::SavedSearches, CapabilityAction::Read) => ApiKeyScope::SavedSearchesRead,
        (CapabilityFamily::SavedSearches, CapabilityAction::Create) => {
            ApiKeyScope::SavedSearchesCreate
        }
        (CapabilityFamily::SavedSearches, CapabilityAction::Update) => {
            ApiKeyScope::SavedSearchesUpdate
        }
        (CapabilityFamily::SavedSearches, CapabilityAction::Delete) => {
            ApiKeyScope::SavedSearchesDelete
        }
        (CapabilityFamily::TaskViews, CapabilityAction::Read) => ApiKeyScope::TaskViewsRead,
        (CapabilityFamily::TaskViews, CapabilityAction::Create) => ApiKeyScope::TaskViewsCreate,
        (CapabilityFamily::TaskViews, CapabilityAction::Update) => ApiKeyScope::TaskViewsUpdate,
        (CapabilityFamily::TaskViews, CapabilityAction::Delete) => ApiKeyScope::TaskViewsDelete,
    }
}

/// Converts wire scopes into stored capabilities, deduplicated and ordered in
/// the catalog's canonical `family:action` order (`Capability::ALL`'s order).
fn capabilities_from_wire(scopes: Vec<ApiKeyScope>) -> Vec<Capability> {
    let requested: Vec<Capability> = scopes.into_iter().map(capability_from_scope).collect();
    Capability::ALL
        .into_iter()
        .filter(|cap| requested.contains(cap))
        .collect()
}

/// Deduplicates and orders a key's stored capabilities into the catalog's
/// canonical order, then maps each to its wire representation.
pub(crate) fn canonical_scopes(capabilities: &[Capability]) -> Vec<ApiKeyScope> {
    Capability::ALL
        .into_iter()
        .filter(|cap| capabilities.contains(cap))
        .map(scope_from_capability)
        .collect()
}

fn key_to_dto(k: &atlas_custos::entities::identity::ApiKey) -> ApiKeyDto {
    ApiKeyDto {
        id: k.id.0,
        name: k.name.clone(),
        r#type: k.type_.as_str().to_string(),
        key_kind: k.key_kind().as_str().to_string(),
        expires_at: k.expires_at,
        last_used_at: k.last_used_at,
        revoked_at: k.revoked_at,
        created_at: k.created_at,
        is_global: k.is_global,
        scopes: canonical_scopes(&k.scopes),
    }
}

// ---------------------------------------------------------------------------
// Shared key-family handlers (v2-e4-s3b)
//
// Every operation below is kind-parameterized: the two route families
// (`/personal-api-keys`, `/agent-api-keys`) are thin utoipa-annotated
// wrappers passing their expected `ApiKeyKind`. A key addressed through the
// wrong family is invisible (404), never a 403 that would confirm it exists.
// ---------------------------------------------------------------------------

fn require_caller_user(principal: AuthPrincipal, action: &str) -> Result<UserId, ApiError> {
    match principal {
        AuthPrincipal::User(uid) => Ok(uid),
        AuthPrincipal::ApiKey(_) => Err(ApiError::Forbidden {
            message: format!("API keys cannot {action}"),
        }),
    }
}

/// Resolves the target key inside the caller's own visible set for the
/// expected family: the key must exist, be the caller's own, and carry the
/// family's credential kind. Any miss answers the same 404 — a foreign key,
/// a nonexistent id, and a wrong-family key are indistinguishable.
async fn load_owned_key_of_kind(
    state: &AppState,
    key_id: ApiKeyId,
    user_id: UserId,
    expected: ApiKeyKind,
) -> Result<atlas_custos::entities::identity::ApiKey, ApiError> {
    PgApiKeyRepo {
        conn: (*state.db).clone(),
    }
    .get_by_id(key_id)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?
    .filter(|k| k.created_by_user_id == user_id && k.key_kind() == expected)
    .ok_or(ApiError::NotFound)
}

/// Shared creation core: mints the token of the family's kind, persists the
/// key (linked to the caller's user principal or the given agent principal),
/// appends the audit event, and applies the optional initial grant — all as
/// one decision the two families only parameterize.
/// The request fields both families share (everything but the kind and the
/// agent binding).
pub(crate) struct NewKeyFields {
    pub(crate) name: String,
    pub(crate) key_type: Option<String>,
    pub(crate) expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub(crate) scopes: Option<Vec<ApiKeyScope>>,
    pub(crate) initial_grant: Option<InitialGrantRequest>,
}

/// Shared creation core: mints the token of the family's kind, persists the
/// key (linked to the caller's user principal or the given agent principal),
/// appends the audit event, and applies the optional initial grant — all as
/// one decision the two families only parameterize.
pub(crate) async fn create_key_of_kind(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    agent: Option<atlas_custos::entities::principals::Agent>,
    fields: NewKeyFields,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = require_caller_user(principal, "create other API keys")?;

    let key_type = parse_key_type(fields.key_type.as_deref())?;
    // The token prefix and the linked principal come from this single kind
    // decision, so a freshly minted key can never carry a prefix that
    // disagrees with its principal kind.
    let secret = generate_api_key_of_kind(expected);
    let token_hash = hash_token(&secret);

    // Omitted or empty scopes fall back to `Capability::DEFAULT_READ_ONLY`: read
    // access to the five default families (tasks, docs, boards, folders,
    // projects), never an empty set. An explicit non-empty selection is
    // deduplicated and canonically ordered.
    let scopes = match fields.scopes {
        Some(scopes) if !scopes.is_empty() => capabilities_from_wire(scopes),
        _ => Capability::DEFAULT_READ_ONLY.to_vec(),
    };

    let new_key = NewApiKey {
        name: fields.name,
        token_hash,
        type_: key_type,
        expires_at: fields.expires_at,
        scopes,
    };

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let key = match agent {
        Some(agent) => PgApiKeyRepo::create_for_agent_in(&txn, user_id, &agent, new_key)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?,
        None => PgApiKeyRepo::create_for_user_in_with_kind(&txn, user_id, expected, new_key)
            .await
            .map_err(|e| ApiError::Internal {
                message: e.to_string(),
            })?,
    };

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(atlas_acta::actor::UserAttributionId(user_id.0)),
            action: SecurityAction::ApiKeyCreated,
            target_type: "api_key".to_string(),
            target_id: Some(key.id.0),
            metadata: serde_json::json!({
                "key_type": key.type_.as_str(),
                "key_name": key.name,
            }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    if let Some(grant_req) = fields.initial_grant {
        create_initial_grant(&state, user_id, key.id, &grant_req).await?;
    }

    Ok((
        StatusCode::CREATED,
        Json(ApiKeyCreated {
            id: key.id.0,
            name: key.name,
            secret,
            r#type: key.type_.as_str().to_string(),
            expires_at: key.expires_at,
            created_at: key.created_at,
            scopes: canonical_scopes(&key.scopes),
        }),
    ))
}

/// Family core for `POST /personal-api-keys`: no agent involved — the key
/// links to the caller's own user principal.
pub(crate) async fn create_personal_key(
    state: AppState,
    principal: AuthPrincipal,
    body: CreatePersonalApiKeyRequest,
) -> Result<impl IntoResponse, ApiError> {
    create_key_of_kind(
        state,
        principal,
        ApiKeyKind::Personal,
        None,
        NewKeyFields {
            name: body.name,
            key_type: body.r#type,
            expires_at: body.expires_at,
            scopes: body.scopes,
            initial_grant: body.initial_grant,
        },
    )
    .await
}

/// Family core for `POST /agent-api-keys`: the key binds to the existing
/// agent principal named by the mandatory `agent_id`, resolved through the
/// same visible-set rules `/agents` already enforces (owner match, or
/// platform admin) — a foreign or nonexistent agent answers 404.
pub(crate) async fn create_agent_key(
    state: AppState,
    principal: AuthPrincipal,
    body: CreateAgentApiKeyRequest,
) -> Result<impl IntoResponse, ApiError> {
    let user_id = require_caller_user(principal, "create other API keys")?;
    let caller = super::agents::caller_user_record(&state, user_id).await?;
    let agent =
        super::agents::resolve_visible_agent(&state, &caller, PrincipalId(body.agent_id)).await?;

    create_key_of_kind(
        state,
        AuthPrincipal::User(user_id),
        ApiKeyKind::Agent,
        Some(agent),
        NewKeyFields {
            name: body.name,
            key_type: body.r#type,
            expires_at: body.expires_at,
            scopes: body.scopes,
            initial_grant: body.initial_grant,
        },
    )
    .await
}

/// Resolves workspace name/slug and optional project name/slug for a set of grants.
///
/// Returns a map of `workspace_id → (slug, name)` and a map of
/// `project_id → (slug, name)` for every workspace/project referenced in the
/// given grants. Unknown ids are omitted; callers fall back to the raw id.
async fn resolve_resource_labels(
    state: &AppState,
    grants: &[PermissionGrant],
    caller: UserId,
) -> Result<
    (
        HashMap<WorkspaceId, (String, String)>,
        HashMap<ProjectId, (String, String)>,
    ),
    ApiError,
> {
    let ws_ids: Vec<WorkspaceId> = {
        let mut seen = std::collections::HashSet::new();
        grants
            .iter()
            .filter(|g| seen.insert(g.workspace_id.0))
            .map(|g| WorkspaceId(g.workspace_id.0))
            .collect()
    };

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };

    let mut ws_map: HashMap<WorkspaceId, (String, String)> = HashMap::new();
    for ws_id in ws_ids {
        if let Ok(Some(ws)) = ws_repo.find_by_id(ws_id).await {
            ws_map.insert(ws_id, (ws.slug, ws.name));
        }
    }

    // `resource_ref` is opaque; decode it against the grant's own workspace to
    // recover the Acta `ResourceRef` and pull out a project-scope grant's id.
    let project_grants: Vec<(ProjectId, WorkspaceId)> = grants
        .iter()
        .filter_map(|g| {
            let workspace_id = WorkspaceId(g.workspace_id.0);
            match atlas_acta::permissions::resource_ref_codec::from_core(
                &g.resource_ref,
                workspace_id,
            ) {
                Ok(atlas_acta::permissions::ResourceRef::Project(pid)) => Some((pid, workspace_id)),
                _ => None,
            }
        })
        .collect();

    let mut project_map: HashMap<ProjectId, (String, String)> = HashMap::new();
    for (project_id, workspace_id) in project_grants {
        if project_map.contains_key(&project_id) {
            continue;
        }
        let proj_repo = PgProjectRepo {
            conn: (*state.db).clone(),
        };
        let ctx = atlas_acta::actor::WorkspaceCtx::new(
            workspace_id,
            atlas_acta::actor::Actor::User(atlas_acta::actor::UserAttributionId(caller.0)),
        );
        if let Ok(Some(proj)) = proj_repo.find(&ctx, project_id).await {
            project_map.insert(project_id, (proj.slug, proj.name));
        }
    }

    Ok((ws_map, project_map))
}

/// Resolves the display label of every principal that created one of `grants`.
///
/// Returns a map of `user_id → display_name` and a map of `api_key_id → key name`
/// covering each distinct `created_by_user_id` / `created_by_api_key_id` referenced
/// by the grants. Unknown ids are omitted; callers treat a miss as no recorded creator.
async fn resolve_granters(
    state: &AppState,
    grants: &[PermissionGrant],
) -> Result<(HashMap<UserId, String>, HashMap<ApiKeyId, String>), ApiError> {
    let user_ids: Vec<UserId> = {
        let mut seen = std::collections::HashSet::new();
        grants
            .iter()
            .filter_map(|g| g.created_by_user_id)
            .filter(|uid| seen.insert(uid.0))
            .collect()
    };

    let key_ids: Vec<ApiKeyId> = {
        let mut seen = std::collections::HashSet::new();
        grants
            .iter()
            .filter_map(|g| g.created_by_api_key_id)
            .filter(|kid| seen.insert(kid.0))
            .collect()
    };

    let user_repo = PgUserRepo {
        conn: (*state.db).clone(),
    };

    let mut user_map: HashMap<UserId, String> = HashMap::new();
    for uid in user_ids {
        if let Ok(Some(user)) = user_repo.find_by_id(uid).await {
            user_map.insert(uid, user.display_name);
        }
    }

    let key_repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let mut key_map: HashMap<ApiKeyId, String> = HashMap::new();
    for kid in key_ids {
        if let Ok(Some(key)) = key_repo.get_by_id(kid).await {
            key_map.insert(kid, key.name);
        }
    }

    Ok((user_map, key_map))
}

/// Builds the `granted_by` attribution for a grant from precomputed granter maps.
///
/// A `created_by_user_id` resolves to a `"user"` principal, a `created_by_api_key_id`
/// to an `"api_key"` principal; a grant with neither (legacy/system) yields `None`.
fn granted_by_for(
    grant: &PermissionGrant,
    user_map: &HashMap<UserId, String>,
    key_map: &HashMap<ApiKeyId, String>,
) -> Option<GrantedByDto> {
    if let Some(uid) = grant.created_by_user_id {
        return Some(GrantedByDto {
            id: uid.0,
            display: user_map
                .get(&uid)
                .cloned()
                .unwrap_or_else(|| uid.0.to_string()),
            principal_type: "user".to_string(),
        });
    }

    if let Some(kid) = grant.created_by_api_key_id {
        return Some(GrantedByDto {
            id: kid.0,
            display: key_map
                .get(&kid)
                .cloned()
                .unwrap_or_else(|| kid.0.to_string()),
            principal_type: "api_key".to_string(),
        });
    }

    None
}

fn grant_to_api_key_grant_dto(
    grant: &PermissionGrant,
    ws_map: &HashMap<WorkspaceId, (String, String)>,
    project_map: &HashMap<ProjectId, (String, String)>,
    user_map: &HashMap<UserId, String>,
    key_map: &HashMap<ApiKeyId, String>,
) -> ApiKeyGrantDto {
    let granted_by = granted_by_for(grant, user_map, key_map);

    let role = match grant.role {
        ResourceRole::Viewer => "viewer".to_string(),
        ResourceRole::Editor => "editor".to_string(),
        ResourceRole::Admin => "admin".to_string(),
    };

    let workspace_id = WorkspaceId(grant.workspace_id.0);
    let workspace_slug = ws_map
        .get(&workspace_id)
        .map(|(slug, _)| slug.clone())
        .unwrap_or_else(|| workspace_id.0.to_string());

    let resource =
        atlas_acta::permissions::resource_ref_codec::from_core(&grant.resource_ref, workspace_id)
            .ok();

    if let Some(atlas_acta::permissions::ResourceRef::Project(pid)) = resource {
        let (project_slug, project_name) = project_map
            .get(&pid)
            .map(|(slug, name)| (slug.clone(), name.clone()))
            .unwrap_or_else(|| (pid.0.to_string(), pid.0.to_string()));

        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "project".to_string(),
            resource_label: project_name,
            workspace_slug,
            project_slug: Some(project_slug),
            granted_by,
        };
    }

    if let Some(atlas_acta::permissions::ResourceRef::Folder(fid)) = resource {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "folder".to_string(),
            resource_label: format!("folder:{}", fid.0),
            workspace_slug,
            project_slug: None,
            granted_by,
        };
    }

    if let Some(atlas_acta::permissions::ResourceRef::Document(did)) = resource {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "document".to_string(),
            resource_label: format!("document:{}", did.0),
            workspace_slug,
            project_slug: None,
            granted_by,
        };
    }

    if let Some(atlas_acta::permissions::ResourceRef::Board(bid)) = resource {
        return ApiKeyGrantDto {
            id: grant.id.0,
            role,
            resource_kind: "board".to_string(),
            resource_label: format!("board:{}", bid.0),
            workspace_slug,
            project_slug: None,
            granted_by,
        };
    }

    let ws_label = ws_map
        .get(&workspace_id)
        .map(|(_, name)| name.clone())
        .unwrap_or_else(|| workspace_id.0.to_string());

    ApiKeyGrantDto {
        id: grant.id.0,
        role,
        resource_kind: "workspace".to_string(),
        resource_label: ws_label,
        workspace_slug,
        project_slug: None,
        granted_by,
    }
}

pub(crate) async fn list_key_grants_core(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    params: TopLevelRevokeKeyPath,
) -> Result<Json<Vec<ApiKeyGrantDto>>, ApiError> {
    let user_id = require_caller_user(principal, "list grants")?;

    let key_id = ApiKeyId(params.key_id);
    let _key = load_owned_key_of_kind(&state, key_id, user_id, expected).await?;

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let grants = grant_repo
        .list_for_api_key(key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let (ws_map, project_map) = resolve_resource_labels(&state, &grants, user_id).await?;
    let (user_map, key_map) = resolve_granters(&state, &grants).await?;

    let dtos: Vec<ApiKeyGrantDto> = grants
        .iter()
        .map(|g| grant_to_api_key_grant_dto(g, &ws_map, &project_map, &user_map, &key_map))
        .collect();

    Ok(Json(dtos))
}

pub(crate) async fn delete_key_grant_core(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    params: ApiKeyGrantPath,
) -> Result<StatusCode, ApiError> {
    let user_id = require_caller_user(principal, "revoke grants")?;

    let key_id = ApiKeyId(params.key_id);
    let _key = load_owned_key_of_kind(&state, key_id, user_id, expected).await?;

    let grant_id = PermissionGrantId(params.grant_id);
    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };

    let deleted = grant_repo
        .delete_for_api_key(grant_id, key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    if !deleted {
        return Err(ApiError::NotFound);
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Creates a workspace-scope grant for a newly created key. Rejects admin roles
/// (agent cap enforced via `authorize_grant_target`).
async fn create_initial_grant(
    state: &AppState,
    user_id: UserId,
    key_id: ApiKeyId,
    grant_req: &InitialGrantRequest,
) -> Result<(), ApiError> {
    let role = parse_role(&grant_req.role)?;

    authorize_grant_target(&Principal::ApiKey(key_id), role).map_err(|e| {
        let message = match e {
            ShareDenied::AgentCannotBeAdmin => {
                "agents cannot be granted the Admin role".to_string()
            }
            _ => "insufficient permissions to create grant".to_string(),
        };
        ApiError::Forbidden { message }
    })?;

    let ws_repo = PgWorkspaceRepo {
        conn: (*state.db).clone(),
    };
    let workspace = ws_repo
        .find_by_slug(&grant_req.workspace)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    let grant_repo = PgPermissionGrantRepo {
        conn: (*state.db).clone(),
    };
    grant_repo
        .upsert(NewPermissionGrant {
            workspace_id: atlas_custos::WorkspaceScope(workspace.id.0),
            user_id: None,
            api_key_id: Some(key_id),
            group_id: None,
            resource_ref: atlas_acta::permissions::resource_ref_codec::to_core(
                &atlas_acta::permissions::ResourceRef::Workspace,
                workspace.id,
            ),
            role,
            created_by_user_id: Some(user_id),
            created_by_api_key_id: None,
        })
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok(())
}

pub(crate) async fn list_keys_core(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    q: PaginationQuery,
) -> Result<Json<Page<ApiKeyDto>>, ApiError> {
    let user_id = require_caller_user(principal, "list API keys")?;

    let repo = PgApiKeyRepo {
        conn: (*state.db).clone(),
    };

    let limit = q.limit.unwrap_or(50).clamp(1, 200) as u64;
    let after_id = q.cursor.as_deref().and_then(Cursor::decode).map(|c| c.0);

    let all_keys = repo
        .list_for_user(user_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    let mut filtered: Vec<_> = all_keys
        .into_iter()
        .filter(|k| k.key_kind() == expected)
        .filter(|k| after_id.is_none_or(|cursor| k.id.0 > cursor))
        .collect();

    let has_more = filtered.len() > limit as usize;
    if has_more {
        filtered.truncate(limit as usize);
    }

    let next_cursor = if has_more {
        filtered.last().map(|k| Cursor(k.id.0))
    } else {
        None
    };

    let dtos: Vec<ApiKeyDto> = filtered.iter().map(key_to_dto).collect();
    Ok(Json(Page::new(dtos, next_cursor, has_more)))
}

pub(crate) async fn revoke_key_core(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    params: TopLevelRevokeKeyPath,
) -> Result<StatusCode, ApiError> {
    let user_id = require_caller_user(principal, "revoke API keys")?;

    let key_id = ApiKeyId(params.key_id);

    let _key = load_owned_key_of_kind(&state, key_id, user_id, expected).await?;

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let key = PgApiKeyRepo::revoke_for_user_in(&txn, user_id, key_id)
        .await
        .map_err(|e| match e {
            atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
            atlas_core::error::DomainError::Forbidden { message } => {
                ApiError::Forbidden { message }
            }
            other => ApiError::Internal {
                message: other.to_string(),
            },
        })?;

    // Acta-side half of the revoke split (design D4): the Custos row update
    // above and this task-assignee cleanup share `txn`, so they commit or
    // roll back together exactly as they did before the split.
    PgTaskAssigneeRepo::unassign_api_key_in(&txn, key_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: Actor::User(atlas_acta::actor::UserAttributionId(user_id.0)),
            action: SecurityAction::ApiKeyRevoked,
            target_type: "api_key".to_string(),
            target_id: Some(key_id.0),
            metadata: serde_json::json!({ "key_type": key.type_.as_str() }),
        },
    )
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(StatusCode::NO_CONTENT)
}

/// Updates a caller-owned key of the expected family. `is_global` and `scopes`
/// are each PATCH-partial: omitting a field leaves it unchanged; both may be
/// set in the same request.
///
/// Any owner of the key may toggle global reach or replace its scope set; the
/// agent never gains more than its creator can reach (and stays capped at
/// editor) nor more capabilities than the closed catalog allows, so this is
/// bounded by the owner's own permissions rather than being a privilege
/// escalation.
pub(crate) async fn update_key_core(
    state: AppState,
    principal: AuthPrincipal,
    expected: ApiKeyKind,
    params: TopLevelRevokeKeyPath,
    body: UpdateApiKeyRequest,
) -> Result<Json<ApiKeyDto>, ApiError> {
    let user_id = require_caller_user(principal, "manage API keys")?;

    let key_id = ApiKeyId(params.key_id);

    let scopes = match body.scopes {
        None => None,
        Some(scopes) if scopes.is_empty() => {
            return Err(ApiError::BadRequest {
                message: "scopes cannot be empty; revoke the key instead".into(),
            });
        }
        Some(scopes) => Some(capabilities_from_wire(scopes)),
    };

    if body.is_global.is_none() && scopes.is_none() {
        let key = load_owned_key_of_kind(&state, key_id, user_id, expected).await?;

        return Ok(Json(key_to_dto(&key)));
    }

    // The family/ownership gate above already resolved the key inside the
    // caller's visible set; the user-scoped repo writes below re-check the
    // owner under the transaction's snapshot.
    load_owned_key_of_kind(&state, key_id, user_id, expected).await?;

    let txn = (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let mut key = None;

    if let Some(is_global) = body.is_global {
        let updated = PgApiKeyRepo::set_global_for_user_in(&txn, user_id, key_id, is_global)
            .await
            .map_err(|e| match e {
                atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
                other => ApiError::Internal {
                    message: other.to_string(),
                },
            })?;

        PgSecurityAuditRepo::append_in(
            &txn,
            NewSecurityAuditEvent {
                workspace_id: None,
                actor: Actor::User(atlas_acta::actor::UserAttributionId(user_id.0)),
                action: SecurityAction::ApiKeyGlobalChanged,
                target_type: "api_key".to_string(),
                target_id: Some(key_id.0),
                metadata: serde_json::json!({ "is_global": is_global }),
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

        key = Some(updated);
    }

    if let Some(scopes) = scopes {
        let metadata = serde_json::json!({
            "scopes": scopes.iter().map(Capability::as_str).collect::<Vec<_>>(),
        });

        let updated = PgApiKeyRepo::set_scopes_for_user_in(&txn, user_id, key_id, scopes)
            .await
            .map_err(|e| match e {
                atlas_core::error::DomainError::NotFound { .. } => ApiError::NotFound,
                other => ApiError::Internal {
                    message: other.to_string(),
                },
            })?;

        PgSecurityAuditRepo::append_in(
            &txn,
            NewSecurityAuditEvent {
                workspace_id: None,
                actor: Actor::User(atlas_acta::actor::UserAttributionId(user_id.0)),
                action: SecurityAction::ApiKeyScopesChanged,
                target_type: "api_key".to_string(),
                target_id: Some(key_id.0),
                metadata,
            },
        )
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

        key = Some(updated);
    }

    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    let key = key.ok_or(ApiError::Internal {
        message: "update_key_core: no field applied despite entering the update branch".into(),
    })?;

    Ok(Json(key_to_dto(&key)))
}
