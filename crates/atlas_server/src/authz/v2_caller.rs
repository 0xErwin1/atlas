//! Who is calling a V2 authorization route, resolved once per request, and
//! the 503 every V2 route maps an evaluation failure to.
//!
//! A platform admin or root session keeps the administration path; any
//! other session acts as itself with an unrestricted credential ceiling; an
//! API key is bounded by the ceiling its scopes translate to
//! (`authz::v2_ceiling`).

use atlas_core::Attribution;
use atlas_core::attribution::{ApiKeyAttributionId, UserAttributionId};
use atlas_core::principal::ApiKeyId;
use atlas_custos::authorize::ActorContext;
use atlas_custos::entities::identity::User;
use atlas_custos::eval::{Ceiling, EvalError};
use atlas_custos::ids::PrincipalId;
use atlas_custos_postgres::repos::identity::{ApiKeyRepo, PgApiKeyRepo};

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    authz::v2_ceiling::{api_key_ceiling, session_ceiling},
    error::ApiError,
    persistence::repos::api_key_principal,
    routes::agents::{caller_user_record, is_platform_admin},
    state::AppState,
};

/// Who is calling, resolved once per request. A platform admin or root
/// session keeps the S4 administration path; any other session is a
/// delegate whose authority is computed on the exact target it acts on; an
/// API key is bounded by the ceiling its scopes translate to (`v2_ceiling`).
pub(crate) enum Caller {
    Session {
        user: User,
        admin: bool,
        actor: ActorContext,
    },
    ApiKey {
        key_id: ApiKeyId,
        ceiling: Ceiling,
    },
}

impl Caller {
    pub(crate) fn attribution(&self) -> Attribution {
        match self {
            Caller::Session { user, .. } => actor_of(user),
            Caller::ApiKey { key_id, .. } => Attribution::ApiKey(ApiKeyAttributionId(key_id.0)),
        }
    }
}

pub(crate) async fn caller(state: &AppState, principal: AuthPrincipal) -> Result<Caller, ApiError> {
    match principal {
        AuthPrincipal::User(user_id) => {
            let user = caller_user_record(state, user_id).await?;
            let actor = ActorContext {
                principal: PrincipalId::from(user.id),
                is_root: user.is_root,
                ceiling: session_ceiling(),
            };

            Ok(Caller::Session {
                admin: is_platform_admin(&user),
                user,
                actor,
            })
        }
        AuthPrincipal::ApiKey(key_id) => {
            let key = PgApiKeyRepo {
                conn: (*state.db).clone(),
            }
            .get_by_id(key_id)
            .await
            .map_err(ApiError::Domain)?
            .ok_or(ApiError::Unauthorized)?;

            Ok(Caller::ApiKey {
                key_id,
                ceiling: api_key_ceiling(&key.scopes),
            })
        }
    }
}

/// The actor an authorization question is asked for: a session's own user
/// with its unrestricted ceiling, or the principal an API key acts as,
/// bounded by the key's ceiling. A key is never root.
pub(crate) async fn question_actor(
    state: &AppState,
    caller: Caller,
) -> Result<ActorContext, ApiError> {
    match caller {
        Caller::Session { actor, .. } => Ok(actor),
        Caller::ApiKey { key_id, ceiling } => {
            let principal = api_key_principal(&state.db, key_id)
                .await
                .map_err(ApiError::Domain)?
                .ok_or(ApiError::Unauthorized)?;

            Ok(ActorContext {
                principal,
                is_root: false,
                ceiling,
            })
        }
    }
}

/// The attribution of a human session's user.
pub(crate) fn actor_of(user: &User) -> Attribution {
    Attribution::User(UserAttributionId(user.id.0))
}

/// Every evaluation failure is a 503 with the cause logged, never returned
/// (AVAIL-1).
pub(crate) fn unavailable(error: EvalError) -> ApiError {
    tracing::error!(error = %error, "V2 authorization facts unavailable");
    ApiError::AuthorizationUnavailable
}
