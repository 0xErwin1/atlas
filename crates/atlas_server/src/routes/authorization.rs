//! Platform-admin administration of the V2 authorization records
//! (`v2-e5-s4-authz-routes`): custom roles, grants and deny rules over the
//! S3 storage, validated through the S2 catalog.
//!
//! ## Policy decisions baked into these handlers
//!
//! - **Platform admin or root sessions only.** Every route answers 403 to a
//!   non-admin user and to any API key, regardless of the key's scopes: the
//!   delegated authority model (a workspace admin granting within its own
//!   ceiling) arrives in S5.
//! - **Deny administration follows `ATLAS_EXPLICIT_DENY_MODE`.** Creating
//!   or deleting a deny rule answers 409 while the mode is `disabled`;
//!   listing works in every mode, and existing rows persist across mode
//!   changes.
//! - **Validation is catalog-driven.** Grants, deny rules and custom roles
//!   are resolved through the [`Catalog`] built from the registry's
//!   `Authorization` declarations, the same rules the evaluator applies. A
//!   product enters the catalog only once it declares V2 resource kinds;
//!   today only Custos does, so an Acta target answers 422 until E7
//!   publishes Acta's catalog, after which these routes accept it without
//!   code changes. No product declares built-in roles yet, and custom roles
//!   may never carry Custos actions (GRANT-5), so in this release the only
//!   usable authority on a Custos target is an explicit action set.
//! - **Every mutation writes its audit row in the same transaction** as the
//!   row it changes (CUSTOS-DB-1).

use std::collections::HashSet;

use axum::{
    Extension, Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use chrono::Utc;
use sea_orm::{DatabaseTransaction, TransactionTrait};
use serde::Deserialize;

use atlas_api::dtos::authorization::{
    AuthorityDto, CreateDenyRequest, CreateGrantV2Request, CreateRoleRequest, DenyRuleDto,
    GrantV2Dto, RoleDto, SubjectDto, TargetDto, UpdateRoleRequest,
};
use atlas_core::Attribution;
use atlas_core::attribution::UserAttributionId;
use atlas_core::error::DomainError;
use atlas_core::ids::ActionId;
use atlas_core::registry::Registry;
use atlas_custos::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantAuthority, GrantId, GrantRecord, NewCustomRole,
    NewDenyRecord, NewGrantRecord, RoleId, SubjectRecord, TargetKind, TargetRecord,
};
use atlas_custos::entities::identity::User;
use atlas_custos::entities::security_audit::{NewSecurityAuditEvent, SecurityAction};
use atlas_custos::eval::{
    ActionSet, Catalog, CatalogError, GrantSpec, GrantTarget, ProductSpec, Subject, grant_spec,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo, PgRoleRepo};
use atlas_custos_postgres::repos::security_audit::PgSecurityAuditRepo;

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    config::DenyModeConfig,
    error::{ApiError, custos_conflict},
    routes::agents::{caller_user_record, is_platform_admin},
    state::AppState,
};

/// `?product=<product>` on every list route.
#[derive(Debug, Deserialize)]
pub(crate) struct ProductQuery {
    pub(crate) product: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct RolePath {
    pub(crate) role_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
pub(crate) struct GrantPath {
    pub(crate) grant_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
pub(crate) struct DenyPath {
    pub(crate) deny_id: uuid::Uuid,
}

// ---------------------------------------------------------------------------
// Gate
// ---------------------------------------------------------------------------

/// Resolves the calling platform admin (root or `is_system_admin`). An API
/// key or a plain user answers 403: S4 has no delegated authority model.
async fn platform_admin(state: &AppState, principal: AuthPrincipal) -> Result<User, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(user_id) => user_id,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot administer authorization; authenticate as a \
                          platform admin"
                    .into(),
            });
        }
    };

    let user = caller_user_record(state, user_id).await?;
    if !is_platform_admin(&user) {
        return Err(ApiError::Forbidden {
            message: "only a platform admin or root can administer roles, grants and deny rules"
                .into(),
        });
    }

    Ok(user)
}

fn actor_of(user: &User) -> Attribution {
    Attribution::User(UserAttributionId(user.id.0))
}

fn require_deny_administration(state: &AppState) -> Result<(), ApiError> {
    if state.explicit_deny_mode == DenyModeConfig::Disabled {
        return Err(ApiError::Domain(DomainError::ComponentConflict {
            code: custos_conflict::DENY_MODE_DISABLED,
            message: None,
        }));
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

fn invalid(message: impl Into<String>) -> ApiError {
    ApiError::InvalidInput {
        message: message.into(),
    }
}

/// A custom role name: trimmed and non-empty.
fn role_name(raw: &str) -> Result<String, ApiError> {
    let name = raw.trim();

    if name.is_empty() {
        return Err(invalid("a role name must not be empty"));
    }

    Ok(name.to_string())
}

#[cfg(test)]
mod role_name_tests {
    use super::role_name;

    #[test]
    fn a_name_is_trimmed() {
        assert_eq!(role_name("  reviewer ").unwrap(), "reviewer");
    }

    #[test]
    fn an_empty_or_whitespace_only_name_is_rejected() {
        assert!(role_name("").is_err());
        assert!(role_name("   \t").is_err());
    }
}

/// Catalog-backed validation of grants, deny rules and custom roles. The
/// [`Catalog`] is built from the registry's `Authorization` declarations:
/// a product enters it only once it declares V2 resource kinds, and only
/// its actions of a declared (singular, V2) kind are taken, so a V1 plural
/// scope such as `custos::grants::read` never becomes a grantable action.
/// A target or role for a product outside the catalog is rejected until
/// that product publishes its catalog.
struct Validation {
    catalog: Catalog,
    published: HashSet<String>,
}

impl Validation {
    fn new(registry: &Registry) -> Result<Self, ApiError> {
        let specs: Vec<ProductSpec> = registry
            .entries()
            .iter()
            .filter(|entry| !entry.authorization.resource_kinds.is_empty())
            .map(|entry| {
                let kinds = entry.authorization.resource_kinds.clone();
                let actions = entry
                    .authorization
                    .actions
                    .iter()
                    .filter(|action| kinds.iter().any(|kind| kind == action.kind()))
                    .cloned()
                    .collect();

                ProductSpec {
                    product: entry.identity.stable_id.as_str().to_string(),
                    kinds,
                    actions,
                    roles: vec![],
                    principal_sets: entry.authorization.principal_sets.clone(),
                }
            })
            .collect();
        let published = specs.iter().map(|spec| spec.product.clone()).collect();
        let catalog = Catalog::new(specs).map_err(|e| ApiError::Internal {
            message: format!("registry authorization declarations do not form a catalog: {e}"),
        })?;

        Ok(Self { catalog, published })
    }

    fn not_published(product: &str) -> ApiError {
        invalid(format!(
            "product `{product}` has not published its V2 authorization catalog; grants, deny \
             rules and custom roles can target it once it declares resource kinds and actions"
        ))
    }

    fn catalog_error(error: CatalogError) -> ApiError {
        match error {
            CatalogError::UnknownProduct { product } => Self::not_published(&product),
            other => invalid(other.to_string()),
        }
    }

    /// Resolves the candidate grant through the catalog exactly as the
    /// evaluator would (`eval::records::grant_spec`): declared product and
    /// kinds, a declared principal set, and one authority whose actions
    /// exist and match the target's product. A custom-role authority is
    /// revalidated from its stored row.
    async fn validate_grant(
        &self,
        candidate: &GrantRecord,
        roles: &PgRoleRepo,
    ) -> Result<(), ApiError> {
        let custom_roles: Vec<CustomRole> = match &candidate.authority {
            GrantAuthority::CustomRole(role_id) => PgRoleRepo::get_in(&roles.conn, *role_id)
                .await
                .map_err(ApiError::Domain)?
                .into_iter()
                .collect(),
            GrantAuthority::Builtin { .. } | GrantAuthority::Actions(_) => Vec::new(),
        };

        let spec =
            grant_spec(candidate, &self.catalog, &custom_roles).map_err(Self::catalog_error)?;

        self.catalog
            .resolve_grant(spec)
            .map(|_| ())
            .map_err(Self::catalog_error)
    }

    /// A deny rule is validated like a grant with an explicit action set
    /// (same target, subject and action rules), so an undeclared kind or
    /// action is refused here and not only at evaluation time.
    fn validate_deny(&self, candidate: &DenyRecord) -> Result<(), ApiError> {
        let mut spec = GrantSpec {
            target: Some(GrantTarget::from(&candidate.target)),
            actions: Some(action_set(&candidate.actions)?),
            ..GrantSpec::default()
        };

        match Subject::from(&candidate.subject) {
            Subject::Principal(id) => spec.principal = Some(id),
            Subject::Group(id) => spec.group = Some(id),
            Subject::PrincipalSet(set) => spec.principal_set = Some(set),
        }

        self.catalog
            .resolve_grant(spec)
            .map(|_| ())
            .map_err(Self::catalog_error)
    }

    /// A custom role of `product`: the product has published its catalog,
    /// every action is declared there, none is a Custos action, and they all
    /// belong to `product`.
    fn validate_custom_role(&self, product: &str, actions: &[ActionId]) -> Result<(), ApiError> {
        if !self.published.contains(product) {
            return Err(Self::not_published(product));
        }

        let role = self
            .catalog
            .custom_role(actions.iter().cloned())
            .map_err(Self::catalog_error)?;

        match role.actions().product() {
            Some(actions_product) if actions_product != product => Err(invalid(format!(
                "actions belong to product `{actions_product}`, not `{product}`"
            ))),
            _ => Ok(()),
        }
    }
}

fn action_set(actions: &[ActionId]) -> Result<ActionSet, ApiError> {
    ActionSet::new(actions.iter().cloned()).map_err(|e| invalid(e.to_string()))
}

// ---------------------------------------------------------------------------
// Wire <-> record mapping
// ---------------------------------------------------------------------------

fn parse_subject(dto: &SubjectDto) -> Result<SubjectRecord, ApiError> {
    match dto {
        SubjectDto::Principal { id } => Ok(SubjectRecord::Principal(PrincipalId(*id))),
        SubjectDto::Group { id } => Ok(SubjectRecord::Group(GroupId(*id))),
        SubjectDto::PrincipalSet { id } => id
            .parse()
            .map(SubjectRecord::PrincipalSet)
            .map_err(|e| invalid(format!("invalid principal set `{id}`: {e}"))),
    }
}

fn parse_target(dto: &TargetDto) -> Result<TargetRecord, ApiError> {
    let (kind, value) = match dto {
        TargetDto::Ref { value } => (TargetKind::Ref, value),
        TargetDto::Path { value } => (TargetKind::Path, value),
        TargetDto::Selector { value } => (TargetKind::Selector, value),
    };

    TargetRecord::parse(kind, value).map_err(invalid)
}

fn parse_actions(raw: &[String]) -> Result<Vec<ActionId>, ApiError> {
    raw.iter()
        .map(|action| {
            action
                .parse::<ActionId>()
                .map_err(|e| invalid(format!("invalid action `{action}`: {e}")))
        })
        .collect()
}

fn parse_authority(dto: &AuthorityDto) -> Result<GrantAuthority, ApiError> {
    match dto {
        AuthorityDto::Builtin { name, version } => Ok(GrantAuthority::Builtin {
            name: name.clone(),
            version: *version,
        }),
        AuthorityDto::Custom { role_id } => Ok(GrantAuthority::CustomRole(RoleId(*role_id))),
        AuthorityDto::Actions { actions } => parse_actions(actions).map(GrantAuthority::Actions),
    }
}

fn subject_dto(subject: &SubjectRecord) -> SubjectDto {
    match subject {
        SubjectRecord::Principal(id) => SubjectDto::Principal { id: id.0 },
        SubjectRecord::Group(id) => SubjectDto::Group { id: id.0 },
        SubjectRecord::PrincipalSet(set) => SubjectDto::PrincipalSet {
            id: set.to_string(),
        },
    }
}

fn target_dto(target: &TargetRecord) -> TargetDto {
    let value = target.canonical();

    match target.kind() {
        TargetKind::Ref => TargetDto::Ref { value },
        TargetKind::Path => TargetDto::Path { value },
        TargetKind::Selector => TargetDto::Selector { value },
    }
}

fn actions_dto(actions: &[ActionId]) -> Vec<String> {
    actions.iter().map(ToString::to_string).collect()
}

fn authority_dto(authority: &GrantAuthority) -> AuthorityDto {
    match authority {
        GrantAuthority::Builtin { name, version } => AuthorityDto::Builtin {
            name: name.clone(),
            version: *version,
        },
        GrantAuthority::CustomRole(id) => AuthorityDto::Custom { role_id: id.0 },
        GrantAuthority::Actions(actions) => AuthorityDto::Actions {
            actions: actions_dto(actions),
        },
    }
}

fn role_dto(role: &CustomRole) -> RoleDto {
    RoleDto {
        id: role.id.0,
        product: role.product.clone(),
        name: role.name.clone(),
        actions: actions_dto(&role.actions),
        created_by: role.created_by.0,
        created_at: role.created_at,
        updated_at: role.updated_at,
    }
}

fn grant_dto(grant: &GrantRecord) -> GrantV2Dto {
    GrantV2Dto {
        id: grant.id.0,
        subject: subject_dto(&grant.subject),
        target: target_dto(&grant.target),
        product: grant.target.product().to_string(),
        authority: authority_dto(&grant.authority),
        created_by: grant.created_by.0,
        created_at: grant.created_at,
    }
}

fn deny_dto(deny: &DenyRecord) -> DenyRuleDto {
    DenyRuleDto {
        id: deny.id.0,
        subject: subject_dto(&deny.subject),
        target: target_dto(&deny.target),
        product: deny.target.product().to_string(),
        actions: actions_dto(&deny.actions),
        created_by: deny.created_by.0,
        created_at: deny.created_at,
    }
}

// ---------------------------------------------------------------------------
// Audit
// ---------------------------------------------------------------------------

fn role_audit_metadata(role: &CustomRole) -> serde_json::Value {
    serde_json::json!({
        "product": role.product,
        "name": role.name,
        "actions": actions_dto(&role.actions),
    })
}

/// `role.updated` records the post-state plus whatever it replaced, so the
/// log shows the transition and not only the outcome.
fn role_update_audit_metadata(previous: &CustomRole, role: &CustomRole) -> serde_json::Value {
    let mut metadata = serde_json::Map::new();
    metadata.insert("product".into(), serde_json::json!(role.product));
    metadata.insert("name".into(), serde_json::json!(role.name));
    metadata.insert(
        "actions".into(),
        serde_json::json!(actions_dto(&role.actions)),
    );

    if previous.name != role.name {
        metadata.insert("previous_name".into(), serde_json::json!(previous.name));
    }
    if previous.actions != role.actions {
        metadata.insert(
            "previous_actions".into(),
            serde_json::json!(actions_dto(&previous.actions)),
        );
    }

    serde_json::Value::Object(metadata)
}

/// The subject's own identity: the principal or group id, or the principal
/// set name.
fn subject_identity(subject: &SubjectRecord) -> String {
    match subject {
        SubjectRecord::Principal(id) => id.0.to_string(),
        SubjectRecord::Group(id) => id.0.to_string(),
        SubjectRecord::PrincipalSet(set) => set.to_string(),
    }
}

fn grant_audit_metadata(grant: &GrantRecord) -> serde_json::Value {
    serde_json::json!({
        "product": grant.target.product(),
        "subject_kind": grant.subject.kind().as_str(),
        "subject": subject_identity(&grant.subject),
        "target_kind": grant.target.kind().as_str(),
        "target": grant.target.canonical(),
        "authority_kind": grant.authority.kind().as_str(),
    })
}

fn deny_audit_metadata(deny: &DenyRecord) -> serde_json::Value {
    serde_json::json!({
        "product": deny.target.product(),
        "subject_kind": deny.subject.kind().as_str(),
        "subject": subject_identity(&deny.subject),
        "target_kind": deny.target.kind().as_str(),
        "target": deny.target.canonical(),
        "actions": actions_dto(&deny.actions),
    })
}

async fn audit(
    txn: &DatabaseTransaction,
    actor: &User,
    action: SecurityAction,
    target_type: &str,
    target_id: uuid::Uuid,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    PgSecurityAuditRepo::append_in(
        txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: actor_of(actor),
            action,
            target_type: target_type.to_string(),
            target_id: Some(target_id),
            metadata,
        },
    )
    .await
    .map_err(ApiError::Domain)
}

async fn begin(state: &AppState) -> Result<DatabaseTransaction, ApiError> {
    (*state.db).begin().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })
}

async fn commit(txn: DatabaseTransaction) -> Result<(), ApiError> {
    txn.commit().await.map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })
}

fn role_repo(state: &AppState) -> PgRoleRepo {
    PgRoleRepo {
        conn: (*state.db).clone(),
    }
}

// ---------------------------------------------------------------------------
// Roles
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/roles",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("product" = String, Query, description = "Product whose custom roles to list")),
    responses(
        (status = 200, description = "The product's custom roles", body = Vec<RoleDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
    )
)]
pub(crate) async fn list_roles(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(query): Query<ProductQuery>,
) -> Result<Json<Vec<RoleDto>>, ApiError> {
    platform_admin(&state, principal).await?;

    let roles = PgRoleRepo::list_by_product_in(&*state.db, &query.product)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(roles.iter().map(role_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/roles",
    tag = "authorization",
    security(("bearer_auth" = [])),
    request_body = CreateRoleRequest,
    responses(
        (status = 201, description = "Custom role created", body = RoleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 409, description = "A role with this name already exists in the product"),
        (status = 422, description = "Unknown product, unknown action, or an action outside the product"),
    )
)]
pub(crate) async fn create_role(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateRoleRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = platform_admin(&state, principal).await?;

    let name = role_name(&body.name)?;
    let actions = parse_actions(&body.actions)?;
    Validation::new(&state.registry)?.validate_custom_role(&body.product, &actions)?;

    let txn = begin(&state).await?;
    let role = PgRoleRepo::create_in(
        &txn,
        NewCustomRole {
            id: RoleId::new(),
            product: body.product,
            name,
            actions,
            created_by: PrincipalId::from(admin.id),
        },
    )
    .await
    .map_err(ApiError::Domain)?;
    audit(
        &txn,
        &admin,
        SecurityAction::RoleCreated,
        "role",
        role.id.0,
        role_audit_metadata(&role),
    )
    .await?;
    commit(txn).await?;

    Ok((StatusCode::CREATED, Json(role_dto(&role))))
}

#[utoipa::path(
    patch,
    path = "/roles/{role_id}",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("role_id" = uuid::Uuid, Path, description = "Custom role id")),
    request_body = UpdateRoleRequest,
    responses(
        (status = 200, description = "The updated custom role", body = RoleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 404, description = "No such role"),
        (status = 409, description = "A role with this name already exists in the product"),
        (status = 422, description = "Unknown action or an action outside the role's product"),
    )
)]
pub(crate) async fn update_role(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(path): Path<RolePath>,
    Json(body): Json<UpdateRoleRequest>,
) -> Result<Json<RoleDto>, ApiError> {
    let admin = platform_admin(&state, principal).await?;
    let role_id = RoleId(path.role_id);

    let existing = PgRoleRepo::get_in(&*state.db, role_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;

    let name = body.name.as_deref().map(role_name).transpose()?;
    let actions = body.actions.as_deref().map(parse_actions).transpose()?;
    if let Some(actions) = &actions {
        Validation::new(&state.registry)?.validate_custom_role(&existing.product, actions)?;
    }

    let txn = begin(&state).await?;
    let role = PgRoleRepo::update_in(&txn, role_id, name, actions)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    audit(
        &txn,
        &admin,
        SecurityAction::RoleUpdated,
        "role",
        role.id.0,
        role_update_audit_metadata(&existing, &role),
    )
    .await?;
    commit(txn).await?;

    Ok(Json(role_dto(&role)))
}

#[utoipa::path(
    delete,
    path = "/roles/{role_id}",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("role_id" = uuid::Uuid, Path, description = "Custom role id")),
    responses(
        (status = 204, description = "Custom role deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 404, description = "No such role"),
        (status = 409, description = "The role is still referenced by a grant"),
    )
)]
pub(crate) async fn delete_role(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(path): Path<RolePath>,
) -> Result<StatusCode, ApiError> {
    let admin = platform_admin(&state, principal).await?;
    let role_id = RoleId(path.role_id);

    let txn = begin(&state).await?;
    let role = PgRoleRepo::get_in(&txn, role_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    if !PgRoleRepo::delete_in(&txn, role_id)
        .await
        .map_err(ApiError::Domain)?
    {
        return Err(ApiError::NotFound);
    }
    audit(
        &txn,
        &admin,
        SecurityAction::RoleDeleted,
        "role",
        role.id.0,
        role_audit_metadata(&role),
    )
    .await?;
    commit(txn).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Grants
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/grants",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("product" = String, Query, description = "Product whose grants to list")),
    responses(
        (status = 200, description = "The product's V2 grants", body = Vec<GrantV2Dto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
    )
)]
pub(crate) async fn list_grants_v2(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(query): Query<ProductQuery>,
) -> Result<Json<Vec<GrantV2Dto>>, ApiError> {
    platform_admin(&state, principal).await?;

    let grants = PgGrantV2Repo::list_by_product_in(&*state.db, &query.product)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(grants.iter().map(grant_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/grants",
    tag = "authorization",
    security(("bearer_auth" = [])),
    request_body = CreateGrantV2Request,
    responses(
        (status = 201, description = "Grant created", body = GrantV2Dto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 422, description = "Invalid subject, target or authority for the target's product"),
    )
)]
pub(crate) async fn create_grant_v2(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateGrantV2Request>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = platform_admin(&state, principal).await?;

    let candidate = GrantRecord {
        id: GrantId::new(),
        subject: parse_subject(&body.subject)?,
        target: parse_target(&body.target)?,
        authority: parse_authority(&body.authority)?,
        created_by: PrincipalId::from(admin.id),
        created_at: Utc::now(),
    };
    Validation::new(&state.registry)?
        .validate_grant(&candidate, &role_repo(&state))
        .await?;

    let txn = begin(&state).await?;
    let grant = PgGrantV2Repo::create_in(
        &txn,
        NewGrantRecord {
            id: candidate.id,
            subject: candidate.subject,
            target: candidate.target,
            authority: candidate.authority,
            created_by: candidate.created_by,
        },
    )
    .await
    .map_err(ApiError::Domain)?;
    audit(
        &txn,
        &admin,
        SecurityAction::GrantCreated,
        "grant_v2",
        grant.id.0,
        grant_audit_metadata(&grant),
    )
    .await?;
    commit(txn).await?;

    Ok((StatusCode::CREATED, Json(grant_dto(&grant))))
}

#[utoipa::path(
    delete,
    path = "/grants/{grant_id}",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("grant_id" = uuid::Uuid, Path, description = "Grant id")),
    responses(
        (status = 204, description = "Grant revoked"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 404, description = "No such grant"),
    )
)]
pub(crate) async fn delete_grant_v2(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(path): Path<GrantPath>,
) -> Result<StatusCode, ApiError> {
    let admin = platform_admin(&state, principal).await?;
    let grant_id = GrantId(path.grant_id);

    let txn = begin(&state).await?;
    let grant = PgGrantV2Repo::get_in(&txn, grant_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    if !PgGrantV2Repo::delete_in(&txn, grant_id)
        .await
        .map_err(ApiError::Domain)?
    {
        return Err(ApiError::NotFound);
    }
    audit(
        &txn,
        &admin,
        SecurityAction::GrantRevoked,
        "grant_v2",
        grant.id.0,
        grant_audit_metadata(&grant),
    )
    .await?;
    commit(txn).await?;

    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Denies
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/denies",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("product" = String, Query, description = "Product whose deny rules to list")),
    responses(
        (status = 200, description = "The product's deny rules (readable in every deny mode)", body = Vec<DenyRuleDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
    )
)]
pub(crate) async fn list_denies(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Query(query): Query<ProductQuery>,
) -> Result<Json<Vec<DenyRuleDto>>, ApiError> {
    platform_admin(&state, principal).await?;

    let denies = PgDenyRuleRepo::list_by_product_in(&*state.db, &query.product)
        .await
        .map_err(ApiError::Domain)?;

    Ok(Json(denies.iter().map(deny_dto).collect()))
}

#[utoipa::path(
    post,
    path = "/denies",
    tag = "authorization",
    security(("bearer_auth" = [])),
    request_body = CreateDenyRequest,
    responses(
        (status = 201, description = "Deny rule created", body = DenyRuleDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 409, description = "ATLAS_EXPLICIT_DENY_MODE is disabled"),
        (status = 422, description = "Invalid subject, target or actions for the target's product"),
    )
)]
pub(crate) async fn create_deny(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateDenyRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let admin = platform_admin(&state, principal).await?;
    require_deny_administration(&state)?;

    let candidate = DenyRecord {
        id: DenyRuleId::new(),
        subject: parse_subject(&body.subject)?,
        target: parse_target(&body.target)?,
        actions: parse_actions(&body.actions)?,
        created_by: PrincipalId::from(admin.id),
        created_at: Utc::now(),
    };
    Validation::new(&state.registry)?.validate_deny(&candidate)?;

    let txn = begin(&state).await?;
    let deny = PgDenyRuleRepo::create_in(
        &txn,
        NewDenyRecord {
            id: candidate.id,
            subject: candidate.subject,
            target: candidate.target,
            actions: candidate.actions,
            created_by: candidate.created_by,
        },
    )
    .await
    .map_err(ApiError::Domain)?;
    audit(
        &txn,
        &admin,
        SecurityAction::DenyCreated,
        "deny_rule",
        deny.id.0,
        deny_audit_metadata(&deny),
    )
    .await?;
    commit(txn).await?;

    Ok((StatusCode::CREATED, Json(deny_dto(&deny))))
}

#[utoipa::path(
    delete,
    path = "/denies/{deny_id}",
    tag = "authorization",
    security(("bearer_auth" = [])),
    params(("deny_id" = uuid::Uuid, Path, description = "Deny rule id")),
    responses(
        (status = 204, description = "Deny rule deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Caller is not a platform admin"),
        (status = 404, description = "No such deny rule"),
        (status = 409, description = "ATLAS_EXPLICIT_DENY_MODE is disabled"),
    )
)]
pub(crate) async fn delete_deny(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(path): Path<DenyPath>,
) -> Result<StatusCode, ApiError> {
    let admin = platform_admin(&state, principal).await?;
    require_deny_administration(&state)?;
    let deny_id = DenyRuleId(path.deny_id);

    let txn = begin(&state).await?;
    let deny = PgDenyRuleRepo::get_in(&txn, deny_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    if !PgDenyRuleRepo::delete_in(&txn, deny_id)
        .await
        .map_err(ApiError::Domain)?
    {
        return Err(ApiError::NotFound);
    }
    audit(
        &txn,
        &admin,
        SecurityAction::DenyDeleted,
        "deny_rule",
        deny.id.0,
        deny_audit_metadata(&deny),
    )
    .await?;
    commit(txn).await?;

    Ok(StatusCode::NO_CONTENT)
}
