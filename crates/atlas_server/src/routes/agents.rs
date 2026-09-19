//! Agent-principal lifecycle routes (`v2-e4-s3a-agents`): the owner-facing
//! surface over the `agent` rows of `custos.principals`, each carrying its
//! owning human user.
//!
//! Authorization mirrors the existing precedent rather than inventing a gate:
//! the caller must be a human user (an API-key principal answers 403, exactly
//! like every `api_keys` handler), resolved and disability-checked the way
//! `RequireUserAdmin` resolves its user — minus the admin requirement. The
//! platform-admin bypass is the same `is_root || is_system_admin` definition
//! `RequireUserAdmin` and the workspace extractors use.
//!
//! Non-disclosure is structural: the target agent is resolved *inside* the
//! caller's visible set (owner match, or platform admin), so an agent the
//! caller does not own answers 404 — never a 403 that would confirm the agent
//! exists. A nonexistent id and a foreign id are indistinguishable.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;

use atlas_api::dtos::{AgentDto, CreateAgentRequest};
use atlas_custos::entities::identity::User;
use atlas_custos::entities::principals::Agent;
use atlas_custos::ids::PrincipalId;
use atlas_custos_postgres::repos::identity::{AgentRepo, PgAgentRepo, PgUserRepo, UserRepo};

use crate::{auth::middleware::Principal as AuthPrincipal, error::ApiError, state::AppState};

#[derive(Deserialize)]
pub(crate) struct AgentPath {
    pub(crate) agent_id: uuid::Uuid,
}

fn agent_to_dto(agent: &Agent) -> AgentDto {
    AgentDto {
        id: agent.id.0,
        display_name: agent.display_name.clone(),
        deactivated_at: agent.deactivated_at,
        created_at: agent.created_at,
        owner: agent.owner_user_id.0,
    }
}

/// Resolves the calling principal to a non-disabled human user, mirroring
/// `RequireUserAdmin`'s user resolution without its admin requirement.
async fn caller_user(state: &AppState, principal: AuthPrincipal) -> Result<User, ApiError> {
    let user_id = match principal {
        AuthPrincipal::User(uid) => uid,
        AuthPrincipal::ApiKey(_) => {
            return Err(ApiError::Forbidden {
                message: "API keys cannot manage agents".into(),
            });
        }
    };

    let user = PgUserRepo {
        conn: (*state.db).clone(),
    }
    .find_by_id(user_id)
    .await
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?
    .ok_or(ApiError::Unauthorized)?;

    if user.disabled_at.is_some() {
        return Err(ApiError::Unauthorized);
    }

    Ok(user)
}

fn is_platform_admin(user: &User) -> bool {
    user.is_root || user.is_system_admin
}

/// Resolves the target agent inside the caller's visible set: the owner's own
/// agents, or every agent for a platform admin. A foreign or nonexistent id
/// answers the same 404 (structural non-disclosure).
async fn resolve_visible_agent(
    state: &AppState,
    caller: &User,
    agent_id: PrincipalId,
) -> Result<Agent, ApiError> {
    let repo = PgAgentRepo {
        conn: (*state.db).clone(),
    };

    repo.find_by_id(agent_id)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .filter(|agent| is_platform_admin(caller) || agent.owner_user_id == caller.id)
        .ok_or(ApiError::NotFound)
}

#[utoipa::path(
    post,
    path = "/agents",
    tag = "agents",
    security(("bearer_auth" = [])),
    request_body = CreateAgentRequest,
    responses(
        (status = 201, description = "Agent principal created, owned by the caller", body = AgentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot create agents"),
    )
)]
pub(crate) async fn create_agent(
    State(state): State<AppState>,
    axum::extract::Extension(principal): axum::extract::Extension<AuthPrincipal>,
    Json(body): Json<CreateAgentRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let caller = caller_user(&state, principal).await?;

    let repo = PgAgentRepo {
        conn: (*state.db).clone(),
    };
    let agent = repo
        .create(caller.id, body.display_name)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?;

    Ok((StatusCode::CREATED, Json(agent_to_dto(&agent))))
}

#[utoipa::path(
    get,
    path = "/agents",
    tag = "agents",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "The caller's own agents (every agent for a platform admin)", body = Vec<AgentDto>),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot list agents"),
    )
)]
pub(crate) async fn list_agents(
    State(state): State<AppState>,
    axum::extract::Extension(principal): axum::extract::Extension<AuthPrincipal>,
) -> Result<Json<Vec<AgentDto>>, ApiError> {
    let caller = caller_user(&state, principal).await?;

    let repo = PgAgentRepo {
        conn: (*state.db).clone(),
    };
    let agents = if is_platform_admin(&caller) {
        repo.list_all().await
    } else {
        repo.list_for_owner(caller.id).await
    }
    .map_err(|e| ApiError::Internal {
        message: e.to_string(),
    })?;

    Ok(Json(agents.iter().map(agent_to_dto).collect()))
}

#[utoipa::path(
    get,
    path = "/agents/{agent_id}",
    tag = "agents",
    security(("bearer_auth" = [])),
    params(("agent_id" = uuid::Uuid, Path, description = "Agent principal id")),
    responses(
        (status = 200, description = "The agent principal", body = AgentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot view agents"),
        (status = 404, description = "No agent owned by the caller has this id"),
    )
)]
pub(crate) async fn get_agent(
    State(state): State<AppState>,
    axum::extract::Extension(principal): axum::extract::Extension<AuthPrincipal>,
    Path(params): Path<AgentPath>,
) -> Result<Json<AgentDto>, ApiError> {
    let caller = caller_user(&state, principal).await?;
    let agent = resolve_visible_agent(&state, &caller, PrincipalId(params.agent_id)).await?;

    Ok(Json(agent_to_dto(&agent)))
}

#[utoipa::path(
    post,
    path = "/agents/{agent_id}/deactivate",
    tag = "agents",
    security(("bearer_auth" = [])),
    params(("agent_id" = uuid::Uuid, Path, description = "Agent principal id")),
    responses(
        (status = 200, description = "The deactivated agent principal", body = AgentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot manage agents"),
        (status = 404, description = "No agent owned by the caller has this id"),
    )
)]
pub(crate) async fn deactivate_agent(
    State(state): State<AppState>,
    axum::extract::Extension(principal): axum::extract::Extension<AuthPrincipal>,
    Path(params): Path<AgentPath>,
) -> Result<Json<AgentDto>, ApiError> {
    let caller = caller_user(&state, principal).await?;
    let agent_id = PrincipalId(params.agent_id);
    resolve_visible_agent(&state, &caller, agent_id).await?;

    let repo = PgAgentRepo {
        conn: (*state.db).clone(),
    };
    let agent = repo
        .set_deactivated(agent_id, Some(chrono::Utc::now()))
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(agent_to_dto(&agent)))
}

#[utoipa::path(
    post,
    path = "/agents/{agent_id}/reactivate",
    tag = "agents",
    security(("bearer_auth" = [])),
    params(("agent_id" = uuid::Uuid, Path, description = "Agent principal id")),
    responses(
        (status = 200, description = "The reactivated agent principal", body = AgentDto),
        (status = 401, description = "Unauthenticated"),
        (status = 403, description = "API keys cannot manage agents"),
        (status = 404, description = "No agent owned by the caller has this id"),
    )
)]
pub(crate) async fn reactivate_agent(
    State(state): State<AppState>,
    axum::extract::Extension(principal): axum::extract::Extension<AuthPrincipal>,
    Path(params): Path<AgentPath>,
) -> Result<Json<AgentDto>, ApiError> {
    let caller = caller_user(&state, principal).await?;
    let agent_id = PrincipalId(params.agent_id);
    resolve_visible_agent(&state, &caller, agent_id).await?;

    let repo = PgAgentRepo {
        conn: (*state.db).clone(),
    };
    let agent = repo
        .set_deactivated(agent_id, None)
        .await
        .map_err(|e| ApiError::Internal {
            message: e.to_string(),
        })?
        .ok_or(ApiError::NotFound)?;

    Ok(Json(agent_to_dto(&agent)))
}
