//! `GET /api/v2/custos/discover` (E11-S8 design D4/D5).
//!
//! Answers "what can I reach?" for the authenticated principal: per-product
//! resource refs the principal holds a grant on (`atlas_custos`'s
//! [`DiscoveryPort`], D-S8-1), unioned with Acta workspace membership
//! (D-S8-9 — Custos never depends on Acta, so this union happens here, not
//! in either port). Root/system-admin principals short-circuit to
//! `admin: true` and every present registry component, reading zero grant
//! rows (INV-ADMIN-FROM-FLAGS, mirrors `authz/extractors.rs`'s break-glass
//! pattern).
//!
//! Registry membership, not a hardcoded product list, decides which
//! products may appear in `components[]`: for each registry entry, this
//! handler asks the unioned [`GrantedScopes`] whether that product has any
//! non-empty scope set and includes it only then (INV-ABSENT-NOT-EMPTY).

use axum::{
    Json,
    extract::{Extension, State},
};

use atlas_acta::permissions::ResourceRef as ActaResourceRef;
use atlas_acta::permissions::resource_ref_codec::to_core;
use atlas_acta_postgres::repos::identity::{PgWorkspaceRepo, WorkspaceRepo};
use atlas_api::dtos::discovery::{DiscoverResponseDto, DiscoveredComponentDto};
use atlas_custos::ports::discovery::{DiscoveryPort, DiscoveryPrincipal, GrantedScopes};
use atlas_custos_postgres::repos::discovery::PgDiscoveryRepo;
use atlas_custos_postgres::repos::identity::PgUserRepo;

use crate::{
    auth::middleware::Principal, error::ApiError, persistence::repos::UserRepo, state::AppState,
};

/// Resolves the `is_root`/`is_system_admin` flags and the ids `discover`
/// needs from the request's authenticated [`Principal`] (`routes/auth.rs`'s
/// `me()` assembly pattern, D4 step 1). An api-key principal is never an
/// admin (`routes/auth.rs:261-262`'s precedent).
async fn discovery_principal(
    state: &AppState,
    principal: &Principal,
) -> Result<DiscoveryPrincipal, ApiError> {
    match principal {
        Principal::User(user_id) => {
            let user_repo = PgUserRepo {
                conn: (*state.db).clone(),
            };
            let user = user_repo
                .find_by_id(*user_id)
                .await
                .map_err(|_| ApiError::Internal {
                    message: "user lookup failed".into(),
                })?
                .ok_or(ApiError::Unauthorized)?;

            Ok(DiscoveryPrincipal {
                user_id: Some(*user_id),
                api_key_id: None,
                is_root: user.is_root,
                is_system_admin: user.is_system_admin,
            })
        }
        Principal::ApiKey(api_key_id) => Ok(DiscoveryPrincipal {
            user_id: None,
            api_key_id: Some(*api_key_id),
            is_root: false,
            is_system_admin: false,
        }),
    }
}

/// Builds the admin short-circuit response (D4 step 2): every present
/// registry component, empty scopes, `admin: true`, reading zero grant
/// rows.
fn admin_response(state: &AppState) -> DiscoverResponseDto {
    let components = state
        .registry
        .entries()
        .iter()
        .map(|entry| DiscoveredComponentDto {
            component: entry.identity.stable_id.as_str().to_string(),
            scopes: Vec::new(),
        })
        .collect();

    DiscoverResponseDto {
        components,
        admin: true,
        truncated: false,
    }
}

#[utoipa::path(
    get,
    path = "/discover",
    tag = "discover",
    security(("bearer_auth" = [])),
    responses(
        (status = 200, description = "Per-component discoverable scopes and the admin flag", body = DiscoverResponseDto),
        (status = 401, description = "Unauthenticated"),
    )
)]
pub(crate) async fn discover(
    State(state): State<AppState>,
    Extension(principal): Extension<Principal>,
) -> Result<Json<DiscoverResponseDto>, ApiError> {
    let discovery_principal = discovery_principal(&state, &principal).await?;

    if discovery_principal.is_platform_admin() {
        return Ok(Json(admin_response(&state)));
    }

    let discovery_repo = PgDiscoveryRepo {
        conn: (*state.db).clone(),
    };
    let mut scopes: GrantedScopes = discovery_repo
        .granted_scopes(&discovery_principal)
        .await
        .map_err(ApiError::Domain)?;

    // D4 step 4: membership-derived Acta workspaces are a second named
    // source (D-S8-9, INV-SOURCES-NAMED), unioned here — only a user
    // principal has memberships (an api-key principal's ceiling stays
    // direct grants only, D4's R5 decision).
    if let Principal::User(user_id) = &principal {
        let workspace_repo = PgWorkspaceRepo {
            conn: (*state.db).clone(),
        };
        let workspaces = workspace_repo
            .list_for_user(*user_id)
            .await
            .map_err(ApiError::Domain)?;
        for workspace in workspaces {
            scopes.insert(to_core(&ActaResourceRef::Workspace, workspace.id));
        }
    }

    // D4 step 5: emit only non-empty components — registry membership, not
    // a hardcoded product list, decides what may appear.
    let components = state
        .registry
        .entries()
        .iter()
        .filter_map(|entry| {
            let product = entry.identity.stable_id.as_str();
            let set = scopes.for_product(product)?;
            if set.is_empty() {
                return None;
            }
            Some(DiscoveredComponentDto {
                component: product.to_string(),
                scopes: set.iter().map(ToString::to_string).collect(),
            })
        })
        .collect();

    Ok(Json(DiscoverResponseDto {
        components,
        admin: false,
        truncated: scopes.truncated,
    }))
}
