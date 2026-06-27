use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::dtos::property_definitions::{
    CreatePropertyDefinitionRequest, PropertyDefinitionDto,
};
use atlas_domain::{
    Actor, DomainError, WorkspaceCtx,
    entities::workspace_core::{AppliesTo, NewPropertyDefinition, PropertyDefinition, PropertyKind},
    ids::PropertyDefinitionId,
    permissions::Principal,
};

use crate::{
    authz::{Authorized, EditorMin, ViewerMin, authorized::WorkspaceRes},
    error::ApiError,
    persistence::repos::{PgPropertyDefinitionRepo, PropertyDefinitionRepo},
    routes::validation::validate_name,
    state::AppState,
};

fn principal_to_actor(principal: &Principal) -> Actor {
    match principal {
        Principal::User(uid) => Actor::User(*uid),
        Principal::ApiKey(kid) => Actor::ApiKey(*kid),
        Principal::Group(_) => Actor::User(atlas_domain::ids::UserId(uuid::Uuid::nil())),
    }
}

fn definition_to_dto(d: PropertyDefinition) -> PropertyDefinitionDto {
    PropertyDefinitionDto {
        id: d.id.0,
        key: d.key,
        name: d.name,
        kind: d.kind.as_str().to_string(),
        options: d.options,
        applies_to: d.applies_to.as_str().to_string(),
        created_at: d.created_at,
    }
}

/// Parses one of the six property kinds from its string form, returning a 422 on
/// an unknown value.
fn parse_kind(raw: &str) -> Result<PropertyKind, ApiError> {
    match raw {
        "text" => Ok(PropertyKind::Text),
        "number" => Ok(PropertyKind::Number),
        "boolean" => Ok(PropertyKind::Boolean),
        "date" => Ok(PropertyKind::Date),
        "select" => Ok(PropertyKind::Select),
        "multi_select" => Ok(PropertyKind::MultiSelect),
        other => Err(ApiError::InvalidInput {
            message: format!(
                "unknown kind '{other}'; must be one of text, number, boolean, date, select, multi_select"
            ),
        }),
    }
}

/// Parses an `applies_to` value, defaulting to `task` when absent.
fn parse_applies_to(raw: Option<&str>) -> Result<AppliesTo, ApiError> {
    match raw.unwrap_or("task") {
        "document" => Ok(AppliesTo::Document),
        "task" => Ok(AppliesTo::Task),
        "both" => Ok(AppliesTo::Both),
        other => Err(ApiError::InvalidInput {
            message: format!("unknown applies_to '{other}'; must be one of document, task, both"),
        }),
    }
}

/// Derives a stable machine key from a human name.
///
/// Lowercases, maps every non-alphanumeric run to a single `_`, trims leading and
/// trailing `_`, and drops any leading characters before the first letter so the
/// result satisfies `^[a-z][a-z0-9_]{0,63}$`. Returns `None` when no valid key can
/// be produced (e.g. an empty or symbol-only name).
fn derive_key(name: &str) -> Option<String> {
    let mut key = String::new();
    let mut pending_separator = false;

    for ch in name.trim().to_lowercase().chars() {
        if ch.is_ascii_alphanumeric() {
            if pending_separator && !key.is_empty() {
                key.push('_');
            }
            key.push(ch);
            pending_separator = false;
        } else {
            pending_separator = true;
        }
    }

    let first_letter = key.find(|c: char| c.is_ascii_alphabetic())?;
    let mut key: String = key[first_letter..].chars().take(64).collect();

    while key.ends_with('_') {
        key.pop();
    }

    if is_valid_key(&key) { Some(key) } else { None }
}

fn is_valid_key(key: &str) -> bool {
    let mut chars = key.chars();

    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }

    key.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// Validates the `options` payload against the field kind.
///
/// `select`/`multi_select` require a non-empty JSON array of unique, non-empty
/// strings. Every other kind must omit options entirely. A JSON `null` is treated
/// as "absent".
fn validate_options(
    kind: &PropertyKind,
    options: Option<serde_json::Value>,
) -> Result<Option<serde_json::Value>, ApiError> {
    let options = options.filter(|v| !v.is_null());

    let requires_options = matches!(kind, PropertyKind::Select | PropertyKind::MultiSelect);

    if !requires_options {
        return match options {
            None => Ok(None),
            Some(_) => Err(ApiError::InvalidInput {
                message: format!(
                    "options are only allowed for select and multi_select fields, not {}",
                    kind.as_str()
                ),
            }),
        };
    }

    let value = options.ok_or_else(|| ApiError::InvalidInput {
        message: format!("{} fields require a non-empty options array", kind.as_str()),
    })?;

    let array = value.as_array().ok_or_else(|| ApiError::InvalidInput {
        message: "options must be a JSON array of strings".to_string(),
    })?;

    if array.is_empty() {
        return Err(ApiError::InvalidInput {
            message: "options must contain at least one value".to_string(),
        });
    }

    let mut seen = std::collections::HashSet::new();
    for entry in array {
        let s = entry.as_str().ok_or_else(|| ApiError::InvalidInput {
            message: "every option must be a string".to_string(),
        })?;

        if s.trim().is_empty() {
            return Err(ApiError::InvalidInput {
                message: "options must not contain blank values".to_string(),
            });
        }

        if !seen.insert(s) {
            return Err(ApiError::InvalidInput {
                message: format!("duplicate option '{s}'"),
            });
        }
    }

    Ok(Some(value))
}

#[derive(Debug, Deserialize)]
pub(crate) struct ListPropertyDefinitionsQuery {
    /// Optional applicability filter: `task` | `document` | `both`.
    pub applies_to: Option<String>,
}

/// Returns true when a definition with `def` applicability should appear under the
/// requested `filter`. `task` and `document` include `both`; `both` matches only
/// definitions that apply to both surfaces.
fn applies_to_matches(def: &AppliesTo, filter: &AppliesTo) -> bool {
    match filter {
        AppliesTo::Task => matches!(def, AppliesTo::Task | AppliesTo::Both),
        AppliesTo::Document => matches!(def, AppliesTo::Document | AppliesTo::Both),
        AppliesTo::Both => matches!(def, AppliesTo::Both),
    }
}

// ---------------------------------------------------------------------------
// GET /v1/workspaces/{ws}/property-definitions
// ---------------------------------------------------------------------------

#[utoipa::path(
    get,
    path = "/v1/workspaces/{ws}/property-definitions",
    tag = "property-definitions",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("applies_to" = Option<String>, Query, description = "Filter by applicability: task | document | both"),
    ),
    responses(
        (status = 200, description = "Workspace property definitions", body = [PropertyDefinitionDto]),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 422, description = "Invalid applies_to filter"),
    )
)]
pub(crate) async fn list_property_definitions(
    auth: Authorized<WorkspaceRes, ViewerMin>,
    State(state): State<AppState>,
    Query(query): Query<ListPropertyDefinitionsQuery>,
) -> Result<Json<Vec<PropertyDefinitionDto>>, ApiError> {
    let filter = query
        .applies_to
        .as_deref()
        .map(|raw| parse_applies_to(Some(raw)))
        .transpose()?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgPropertyDefinitionRepo {
        conn: (*state.db).clone(),
    };

    let mut definitions = repo.list(&ctx).await.map_err(ApiError::Domain)?;

    if let Some(filter) = filter {
        definitions.retain(|d| applies_to_matches(&d.applies_to, &filter));
    }

    Ok(Json(definitions.into_iter().map(definition_to_dto).collect()))
}

// ---------------------------------------------------------------------------
// POST /v1/workspaces/{ws}/property-definitions
// ---------------------------------------------------------------------------

#[utoipa::path(
    post,
    path = "/v1/workspaces/{ws}/property-definitions",
    tag = "property-definitions",
    security(("bearer_auth" = [])),
    params(("ws" = String, Path, description = "Workspace slug")),
    request_body = CreatePropertyDefinitionRequest,
    responses(
        (status = 201, description = "Property definition created", body = PropertyDefinitionDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 409, description = "A field with the derived key already exists"),
        (status = 422, description = "Invalid input"),
    )
)]
pub(crate) async fn create_property_definition(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Json(body): Json<CreatePropertyDefinitionRequest>,
) -> Result<impl IntoResponse, ApiError> {
    validate_name("name", &body.name)?;

    let kind = parse_kind(&body.kind)?;
    let applies_to = parse_applies_to(body.applies_to.as_deref())?;
    let options = validate_options(&kind, body.options)?;

    let key = derive_key(&body.name).ok_or_else(|| ApiError::InvalidInput {
        message: "name must contain at least one letter to derive a field key".to_string(),
    })?;

    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgPropertyDefinitionRepo {
        conn: (*state.db).clone(),
    };

    let existing = repo.list(&ctx).await.map_err(ApiError::Domain)?;
    if existing.iter().any(|d| d.key == key) {
        return Err(ApiError::Domain(DomainError::AlreadyExists {
            message: format!("a field with key '{key}' already exists"),
        }));
    }

    let created = repo
        .create(
            &ctx,
            NewPropertyDefinition {
                key,
                name: body.name.trim().to_string(),
                kind,
                options,
                applies_to,
            },
        )
        .await
        .map_err(ApiError::Domain)?;

    Ok((StatusCode::CREATED, Json(definition_to_dto(created))))
}

// ---------------------------------------------------------------------------
// DELETE /v1/workspaces/{ws}/property-definitions/{property_definition_id}
// ---------------------------------------------------------------------------

#[utoipa::path(
    delete,
    path = "/v1/workspaces/{ws}/property-definitions/{property_definition_id}",
    tag = "property-definitions",
    security(("bearer_auth" = [])),
    params(
        ("ws" = String, Path, description = "Workspace slug"),
        ("property_definition_id" = uuid::Uuid, Path, description = "Property definition ID"),
    ),
    responses(
        (status = 204, description = "Property definition soft-deleted"),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "Insufficient permissions"),
        (status = 404, description = "Property definition not found or not in workspace"),
    )
)]
pub(crate) async fn delete_property_definition(
    auth: Authorized<WorkspaceRes, EditorMin>,
    State(state): State<AppState>,
    Path((_ws, property_definition_id)): Path<(String, uuid::Uuid)>,
) -> Result<StatusCode, ApiError> {
    let actor = principal_to_actor(&auth.principal);
    let ctx = WorkspaceCtx::new(auth.workspace.id, actor);
    let repo = PgPropertyDefinitionRepo {
        conn: (*state.db).clone(),
    };

    repo.soft_delete(&ctx, PropertyDefinitionId(property_definition_id))
        .await
        .map_err(ApiError::Domain)?;

    Ok(StatusCode::NO_CONTENT)
}
