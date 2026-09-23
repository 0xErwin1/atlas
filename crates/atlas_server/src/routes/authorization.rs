//! Administration of the V2 authorization records (`v2-e5-s4-authz-routes`,
//! `v2-e5-s5b-grant-authority`): custom roles, grants and deny rules over
//! the S3 storage, validated through the S2 catalog and, for delegated
//! grants, gated by the V2 authorization service.
//!
//! ## Policy decisions baked into these handlers
//!
//! - **Roles, deny rules and listings are platform-admin only.** A plain
//!   session and any API key answer 403 there.
//! - **Grants may be delegated (GRANT-3/GRANT-4).** A platform admin or
//!   root keeps the unrestricted administration path. Any other session is
//!   a delegate: `POST /grants` needs effective `custos::grant::create` on
//!   the exact target (a `Ref`; paths and selectors stay platform-admin
//!   only) and may only confer actions the delegate holds there;
//!   `DELETE /grants/{id}` needs effective `custos::grant::delete` on the
//!   grant's own target, after the grant was resolved (an unknown id is 404
//!   for everyone). An API key is refused on its ceiling first: no key scope
//!   translates to a delegation action (`authz::v2_ceiling`). Every refusal
//!   writes a `grant.denied` row with a reason code, in its own committed
//!   transaction, and answers 403 without saying whether the target exists.
//! - **Unavailable facts are 503, never a decision (AVAIL-1).** Every
//!   `EvalError` maps to `authorization-unavailable` with the cause logged.
//! - **Deny administration follows `ATLAS_EXPLICIT_DENY_MODE`.** Creating
//!   or deleting a deny rule answers 409 while the mode is `disabled`;
//!   listing works in every mode, and existing rows persist across mode
//!   changes. The mode the routes read is the one the service was built
//!   with (`AppState::with_deny_mode` rebuilds both together).
//! - **Validation is catalog-driven.** Grants, deny rules and custom roles
//!   are resolved through the [`Catalog`] the service also uses
//!   (`authz::v2_service::validation_catalog`), built from the registry's
//!   `Authorization` declarations. A product enters the catalog only once it
//!   declares V2 resource kinds; today only Custos does, so an Acta target
//!   answers 422 until E7 publishes Acta's catalog, after which these routes
//!   accept it without code changes. No product declares built-in roles
//!   yet, and custom roles may never carry Custos actions (GRANT-5), so in
//!   this release the only usable authority on a Custos target is an
//!   explicit action set.
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
use atlas_core::error::DomainError;
use atlas_core::ids::ActionId;
use atlas_core::registry::Registry;
use atlas_custos::authorize::ActorContext;
use atlas_custos::entities::authorization::{
    CustomRole, DenyRecord, DenyRuleId, GrantAuthority, GrantId, GrantRecord, NewCustomRole,
    NewDenyRecord, NewGrantRecord, RoleId, SubjectRecord, TargetKind, TargetRecord,
};
use atlas_custos::entities::identity::User;
use atlas_custos::entities::security_audit::{NewSecurityAuditEvent, SecurityAction};
use atlas_custos::eval::{
    ActionSet, Catalog, CatalogError, Ceiling, DelegationRefused, GrantSpec, GrantTarget, Subject,
    can_delegate, grant_spec,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use atlas_custos_postgres::repos::authorization::{PgDenyRuleRepo, PgGrantV2Repo, PgRoleRepo};
use atlas_custos_postgres::repos::security_audit::PgSecurityAuditRepo;

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    authz::v2_caller::{Caller, actor_of, caller, unavailable},
    authz::v2_service::{product_specs, validation_catalog},
    config::DenyModeConfig,
    error::{ApiError, custos_conflict},
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
/// key or a plain user answers 403: roles, deny rules and listings have no
/// delegated authority model in this release.
async fn platform_admin(state: &AppState, principal: AuthPrincipal) -> Result<User, ApiError> {
    match caller(state, principal).await? {
        Caller::Session {
            user, admin: true, ..
        } => Ok(user),
        Caller::Session { admin: false, .. } => Err(ApiError::Forbidden {
            message: "only a platform admin or root can administer roles, deny rules and \
                      listings; grants may be delegated within your own authority"
                .into(),
        }),
        Caller::ApiKey { .. } => Err(ApiError::Forbidden {
            message: "API keys cannot administer authorization; authenticate as a \
                      platform admin"
                .into(),
        }),
    }
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
        let published = product_specs(registry)
            .into_iter()
            .map(|spec| spec.product)
            .collect();
        let catalog = validation_catalog(registry).map_err(|e| ApiError::Internal {
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
    ) -> Result<ActionSet, ApiError> {
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
            .map(|grant| grant.actions().clone())
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
    actor: Attribution,
    action: SecurityAction,
    target_type: &str,
    target_id: uuid::Uuid,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    PgSecurityAuditRepo::append_in(
        txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor,
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
// Delegation (GRANT-3/GRANT-4)
// ---------------------------------------------------------------------------

/// Why a delegate's request was refused, as recorded in `grant.denied`.
#[derive(Debug, Clone, Copy)]
enum RefusalReason {
    CeilingLacksGrantCreate,
    CeilingLacksGrantDelete,
    NotExactTarget,
    MissingGrantCreate,
    MissingGrantDelete,
    BeyondAuthority,
}

impl RefusalReason {
    fn code(self) -> &'static str {
        match self {
            Self::CeilingLacksGrantCreate => "ceiling_lacks_grant_create",
            Self::CeilingLacksGrantDelete => "ceiling_lacks_grant_delete",
            Self::NotExactTarget => "not_exact_target",
            Self::MissingGrantCreate => "missing_grant_create",
            Self::MissingGrantDelete => "missing_grant_delete",
            Self::BeyondAuthority => "beyond_authority",
        }
    }
}

/// What a delegate asked for, for the refusal row.
struct DelegationRequest<'a> {
    target: &'a TargetRecord,
    granted: Vec<ActionId>,
    grant_id: Option<GrantId>,
}

/// Records the refusal in its own committed transaction and answers 403
/// with a reason that never reveals whether the target exists.
async fn refuse(
    state: &AppState,
    caller: &Caller,
    request: &DelegationRequest<'_>,
    reason: RefusalReason,
    beyond: Vec<ActionId>,
) -> Result<ApiError, ApiError> {
    let mut metadata = serde_json::Map::new();
    metadata.insert(
        "product".into(),
        serde_json::json!(request.target.product()),
    );
    metadata.insert(
        "target_kind".into(),
        serde_json::json!(request.target.kind().as_str()),
    );
    metadata.insert(
        "target".into(),
        serde_json::json!(request.target.canonical()),
    );
    metadata.insert(
        "granted".into(),
        serde_json::json!(actions_dto(&request.granted)),
    );
    metadata.insert("reason".into(), serde_json::json!(reason.code()));
    if !beyond.is_empty() {
        metadata.insert("beyond".into(), serde_json::json!(actions_dto(&beyond)));
    }
    if let Some(grant_id) = request.grant_id {
        metadata.insert("grant_id".into(), serde_json::json!(grant_id.0));
    }

    let txn = begin(state).await?;
    PgSecurityAuditRepo::append_in(
        &txn,
        NewSecurityAuditEvent {
            workspace_id: None,
            actor: caller.attribution(),
            action: SecurityAction::GrantDenied,
            target_type: "grant_v2".to_string(),
            target_id: None,
            metadata: serde_json::Value::Object(metadata),
        },
    )
    .await
    .map_err(ApiError::Domain)?;
    commit(txn).await?;

    Ok(ApiError::Forbidden {
        message: match reason {
            RefusalReason::BeyondAuthority => {
                "the grant confers actions beyond your authority on this target".into()
            }
            RefusalReason::NotExactTarget => {
                "delegated grants must name one exact resource; paths and selectors are \
                 platform-admin only"
                    .into()
            }
            _ => "you may not delegate on this target".into(),
        },
    })
}

/// The delegation action a request needs on its target.
#[derive(Debug, Clone, Copy)]
enum DelegationAction {
    Create,
    Delete,
}

impl DelegationAction {
    fn action_id(self) -> Result<ActionId, ApiError> {
        let action = match self {
            Self::Create => "create",
            Self::Delete => "delete",
        };

        ActionId::new("custos", "grant", action).map_err(|e| ApiError::Internal {
            message: format!("the custos grant action vocabulary is malformed: {e}"),
        })
    }
}

/// A key's refusal: its ceiling never carries the delegation action in this
/// release (no scope translates to `custos::grant::create|delete`), and a
/// key that somehow did would still lack the delegate evaluation path.
async fn refuse_key(
    state: &AppState,
    caller: &Caller,
    ceiling: &Ceiling,
    request: &DelegationRequest<'_>,
    needed: DelegationAction,
) -> Result<ApiError, ApiError> {
    let reason = match (needed, ceiling.permits(&needed.action_id()?)) {
        (DelegationAction::Create, false) => RefusalReason::CeilingLacksGrantCreate,
        (DelegationAction::Create, true) => RefusalReason::MissingGrantCreate,
        (DelegationAction::Delete, false) => RefusalReason::CeilingLacksGrantDelete,
        (DelegationAction::Delete, true) => RefusalReason::MissingGrantDelete,
    };

    refuse(state, caller, request, reason, vec![]).await
}

/// GRANT-3/GRANT-4 for a delegate's grant creation: effective
/// `custos::grant::create` on the exact target, and only actions it holds
/// there.
async fn authorize_creation(
    state: &AppState,
    caller: &Caller,
    actor: &ActorContext,
    candidate: &GrantRecord,
    granted: &ActionSet,
) -> Result<(), ApiError> {
    let mut granted_list: Vec<ActionId> = granted.iter().cloned().collect();
    granted_list.sort();
    let request = DelegationRequest {
        target: &candidate.target,
        granted: granted_list,
        grant_id: None,
    };

    let TargetRecord::Ref(target) = &candidate.target else {
        return Err(refuse(
            state,
            caller,
            &request,
            RefusalReason::NotExactTarget,
            vec![],
        )
        .await?);
    };

    let effective = state
        .authorization
        .effective_actions(actor, target)
        .await
        .map_err(unavailable)?;

    match can_delegate(&effective, granted) {
        Ok(()) => Ok(()),
        Err(DelegationRefused::MissingGrantCreate) => Err(refuse(
            state,
            caller,
            &request,
            RefusalReason::MissingGrantCreate,
            vec![],
        )
        .await?),
        Err(DelegationRefused::BeyondAuthority { actions }) => Err(refuse(
            state,
            caller,
            &request,
            RefusalReason::BeyondAuthority,
            actions,
        )
        .await?),
    }
}

/// GRANT-3 for a delegate's revocation: effective `custos::grant::delete` on
/// the grant's own target.
async fn authorize_revocation(
    state: &AppState,
    caller: &Caller,
    actor: &ActorContext,
    grant: &GrantRecord,
) -> Result<(), ApiError> {
    let request = DelegationRequest {
        target: &grant.target,
        granted: vec![],
        grant_id: Some(grant.id),
    };

    let TargetRecord::Ref(target) = &grant.target else {
        return Err(refuse(
            state,
            caller,
            &request,
            RefusalReason::NotExactTarget,
            vec![],
        )
        .await?);
    };

    let effective = state
        .authorization
        .effective_actions(actor, target)
        .await
        .map_err(unavailable)?;

    if effective.contains(&DelegationAction::Delete.action_id()?) {
        Ok(())
    } else {
        Err(refuse(
            state,
            caller,
            &request,
            RefusalReason::MissingGrantDelete,
            vec![],
        )
        .await?)
    }
}

/// The actions a request names explicitly, for a refusal recorded before
/// the authority is resolved.
fn explicit_actions(authority: &GrantAuthority) -> Vec<ActionId> {
    match authority {
        GrantAuthority::Actions(actions) => actions.clone(),
        GrantAuthority::Builtin { .. } | GrantAuthority::CustomRole(_) => Vec::new(),
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
        actor_of(&admin),
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
        actor_of(&admin),
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
        actor_of(&admin),
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
        (status = 403, description = "Caller lacks authority to delegate on this target: no effective custos::grant::create there, an action beyond the caller's own authority, a path or selector target (platform-admin only), or an API key (no key scope carries the delegation action); platform admins and root are never refused"),
        (status = 422, description = "Invalid subject, target or authority for the target's product"),
        (status = 503, description = "Authorization facts unavailable (urn:atlas:error:authorization-unavailable); retry shortly"),
    )
)]
pub(crate) async fn create_grant_v2(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<CreateGrantV2Request>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = caller(&state, principal).await?;
    let subject = parse_subject(&body.subject)?;
    let target = parse_target(&body.target)?;
    let authority = parse_authority(&body.authority)?;

    let created_by = match &caller {
        Caller::Session { user, .. } => PrincipalId::from(user.id),
        Caller::ApiKey { ceiling, .. } => {
            let request = DelegationRequest {
                target: &target,
                granted: explicit_actions(&authority),
                grant_id: None,
            };
            return Err(
                refuse_key(&state, &caller, ceiling, &request, DelegationAction::Create).await?,
            );
        }
    };

    let candidate = GrantRecord {
        id: GrantId::new(),
        subject,
        target,
        authority,
        created_by,
        created_at: Utc::now(),
    };
    let granted = Validation::new(&state.registry)?
        .validate_grant(&candidate, &role_repo(&state))
        .await?;
    if let Caller::Session {
        admin: false,
        actor,
        ..
    } = &caller
    {
        authorize_creation(&state, &caller, actor, &candidate, &granted).await?;
    }

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
        caller.attribution(),
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
        (status = 403, description = "Caller lacks authority to delegate on this target: no effective custos::grant::delete on the grant's target, or an API key; platform admins and root are never refused"),
        (status = 404, description = "No such grant (resolved before any authority check)"),
        (status = 503, description = "Authorization facts unavailable (urn:atlas:error:authorization-unavailable); retry shortly"),
    )
)]
pub(crate) async fn delete_grant_v2(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Path(path): Path<GrantPath>,
) -> Result<StatusCode, ApiError> {
    let caller = caller(&state, principal).await?;
    let grant_id = GrantId(path.grant_id);

    let grant = PgGrantV2Repo::get_in(&*state.db, grant_id)
        .await
        .map_err(ApiError::Domain)?
        .ok_or(ApiError::NotFound)?;
    match &caller {
        Caller::Session { admin: true, .. } => {}
        Caller::Session {
            admin: false,
            actor,
            ..
        } => authorize_revocation(&state, &caller, actor, &grant).await?,
        Caller::ApiKey { ceiling, .. } => {
            let request = DelegationRequest {
                target: &grant.target,
                granted: vec![],
                grant_id: Some(grant.id),
            };
            return Err(
                refuse_key(&state, &caller, ceiling, &request, DelegationAction::Delete).await?,
            );
        }
    }

    let txn = begin(&state).await?;
    if !PgGrantV2Repo::delete_in(&txn, grant_id)
        .await
        .map_err(ApiError::Domain)?
    {
        return Err(ApiError::NotFound);
    }
    audit(
        &txn,
        caller.attribution(),
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
        actor_of(&admin),
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
        actor_of(&admin),
        SecurityAction::DenyDeleted,
        "deny_rule",
        deny.id.0,
        deny_audit_metadata(&deny),
    )
    .await?;
    commit(txn).await?;

    Ok(StatusCode::NO_CONTENT)
}
