//! `POST /api/v2/custos/authorize` and `POST /api/v2/custos/authorize/batch`
//! (EVAL-1, EVAL-4, CUSTOS-API-1): any authenticated principal asks what it
//! may do itself, answered by the V2 authorization service.
//!
//! ## Policy decisions baked into these handlers
//!
//! - **A principal asks only about itself.** The actor is the caller: a
//!   session's user (root sessions are root) with an unrestricted ceiling,
//!   or the principal an API key acts as, bounded by the key's ceiling
//!   (`authz::v2_ceiling`). The service enforces the ceiling, so the routes
//!   are exempt from capability checks.
//! - **The answer is `allow`, `deny` or `not_found`, nothing more.** No
//!   reason, deny evidence or grant metadata leaves the server
//!   (CUSTOS-DISC-2); audit-mode deny evidence is logged instead.
//! - **Malformed questions are 400, unanswerable ones 422.** An action or
//!   target that does not parse is a bad request; an action or target of a
//!   product that has not published its V2 catalog, or an action of another
//!   product than its target, cannot be evaluated and is 422.
//! - **Unavailable facts are 503 for one target and `not_found` in a
//!   batch.** A single question's evaluation failure is the opaque
//!   `authorization-unavailable`; in a batch the failing target is
//!   `not_found` (EVAL-4) and its cause is logged with the target, never
//!   returned. A failure of the whole batch is still 503.

use std::collections::HashSet;

use axum::{Extension, Json, extract::State};

use atlas_api::dtos::authorization::{
    AuthorizeBatchRequest, AuthorizeBatchResponse, AuthorizeBatchResult, AuthorizeDecision,
    AuthorizeRequest, AuthorizeResponse,
};
use atlas_core::ids::{ActionId, ResourceRef};
use atlas_custos::eval::{Decision, Evaluated, is_delegation_action};

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    authz::v2_caller::{caller, question_actor, unavailable},
    authz::v2_service::product_specs,
    error::ApiError,
    state::AppState,
};

#[utoipa::path(
    post,
    path = "/authorize",
    tag = "authorization",
    security(("bearer_auth" = [])),
    request_body = AuthorizeRequest,
    responses(
        (status = 200, description = "The caller's decision for the action on the target", body = AuthorizeResponse),
        (status = 400, description = "The action or target does not parse"),
        (status = 401, description = "Unauthenticated"),
        (status = 422, description = "The action or target belongs to a product that has not published its V2 catalog, or the action does not apply to the target's product"),
        (status = 503, description = "The authorization facts could not be loaded"),
    )
)]
pub(crate) async fn authorize(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<AuthorizeRequest>,
) -> Result<Json<AuthorizeResponse>, ApiError> {
    let action = parse_action(&body.action)?;
    let target = parse_target(&body.target)?;
    require_answerable(
        &published_products(&state),
        &action,
        std::slice::from_ref(&target),
    )?;

    let actor = question_actor(&state, caller(&state, principal).await?).await?;
    let evaluated = state
        .authorization
        .authorize(&actor, &action, &target)
        .await
        .map_err(unavailable)?;

    Ok(Json(AuthorizeResponse {
        decision: answered(&action, &target, &evaluated),
    }))
}

#[utoipa::path(
    post,
    path = "/authorize/batch",
    tag = "authorization",
    security(("bearer_auth" = [])),
    request_body = AuthorizeBatchRequest,
    responses(
        (status = 200, description = "One decision per target, in request order; a target whose facts could not be loaded is not_found", body = AuthorizeBatchResponse),
        (status = 400, description = "The action or a target does not parse"),
        (status = 401, description = "Unauthenticated"),
        (status = 422, description = "The action or a target belongs to a product that has not published its V2 catalog, or the action does not apply to a target's product"),
        (status = 503, description = "The authorization facts could not be loaded"),
    )
)]
pub(crate) async fn authorize_batch(
    State(state): State<AppState>,
    Extension(principal): Extension<AuthPrincipal>,
    Json(body): Json<AuthorizeBatchRequest>,
) -> Result<Json<AuthorizeBatchResponse>, ApiError> {
    let action = parse_action(&body.action)?;
    let targets = body
        .targets
        .iter()
        .map(|raw| parse_target(raw))
        .collect::<Result<Vec<_>, _>>()?;
    require_answerable(&published_products(&state), &action, &targets)?;

    let actor = question_actor(&state, caller(&state, principal).await?).await?;
    let outcomes = state
        .authorization
        .authorize_batch(&actor, &action, &targets)
        .await
        .map_err(unavailable)?;

    let results = body
        .targets
        .into_iter()
        .zip(&targets)
        .zip(outcomes)
        .map(|((raw, target), outcome)| AuthorizeBatchResult {
            target: raw,
            decision: match outcome {
                Ok(evaluated) => answered(&action, target, &evaluated),
                Err(error) => {
                    tracing::warn!(
                        target = %target,
                        action = %action,
                        error = ?error,
                        "V2 authorization facts unavailable for one batch target; answered not_found"
                    );
                    AuthorizeDecision::NotFound
                }
            },
        })
        .collect();

    Ok(Json(AuthorizeBatchResponse { results }))
}

fn parse_action(raw: &str) -> Result<ActionId, ApiError> {
    raw.parse().map_err(|_| ApiError::BadRequest {
        message: format!("`{raw}` is not an action of the form <product>::<kind>::<action>"),
    })
}

fn parse_target(raw: &str) -> Result<ResourceRef, ApiError> {
    raw.parse().map_err(|_| ApiError::BadRequest {
        message: format!("`{raw}` is not a resource of the form <product>::<kind>::<id>"),
    })
}

/// The products that have published their V2 catalog in the registry.
fn published_products(state: &AppState) -> HashSet<String> {
    product_specs(&state.registry)
        .into_iter()
        .map(|spec| spec.product)
        .collect()
}

/// Rejects a question the evaluator cannot answer: an action or target of a
/// product outside `published`, or a product action asked about a target of
/// another product. A delegation action applies to targets of any product.
fn require_answerable(
    published: &HashSet<String>,
    action: &ActionId,
    targets: &[ResourceRef],
) -> Result<(), ApiError> {
    let unpublished = |product: &str| ApiError::InvalidInput {
        message: format!(
            "product `{product}` has not published its V2 authorization catalog, so no \
             authorization question about it can be answered yet"
        ),
    };

    if !published.contains(action.product()) {
        return Err(unpublished(action.product()));
    }

    for target in targets {
        if !published.contains(target.product()) {
            return Err(unpublished(target.product()));
        }

        if !is_delegation_action(action) && action.product() != target.product() {
            return Err(ApiError::InvalidInput {
                message: format!(
                    "action `{action}` does not apply to `{target}`, a resource of another product"
                ),
            });
        }
    }

    Ok(())
}

/// The wire decision for an evaluation, logging audit-mode deny evidence
/// instead of returning it.
fn answered(action: &ActionId, target: &ResourceRef, evaluated: &Evaluated) -> AuthorizeDecision {
    if !evaluated.would_block.is_empty() {
        tracing::info!(
            target = %target,
            action = %action,
            would_block = ?evaluated.would_block,
            "audit mode: deny rules would have changed this authorization decision"
        );
    }

    match evaluated.decision {
        Decision::Allowed => AuthorizeDecision::Allow,
        Decision::Denied { .. } => AuthorizeDecision::Deny,
        Decision::NotFound => AuthorizeDecision::NotFound,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_custos::eval::DenyCause;

    fn published(products: &[&str]) -> HashSet<String> {
        products.iter().map(|product| product.to_string()).collect()
    }

    fn target(raw: &str) -> ResourceRef {
        raw.parse().expect("valid ref")
    }

    fn action(raw: &str) -> ActionId {
        raw.parse().expect("valid action")
    }

    fn evaluated(decision: Decision) -> Evaluated {
        Evaluated {
            decision,
            would_block: Vec::new(),
        }
    }

    #[test]
    fn malformed_actions_and_targets_are_bad_requests() {
        for raw in ["custos::group", "custos::group::read::x", ""] {
            assert!(
                matches!(parse_action(raw), Err(ApiError::BadRequest { .. })),
                "{raw}"
            );
        }

        for raw in [
            "custos::group",
            "custos::workspace::w1/group::g1",
            "not a ref",
        ] {
            assert!(
                matches!(parse_target(raw), Err(ApiError::BadRequest { .. })),
                "{raw}"
            );
        }

        assert!(parse_action("custos::group::read").is_ok());
        assert!(parse_target("custos::group::g1").is_ok());
    }

    #[test]
    fn a_question_about_an_unpublished_product_is_unanswerable() {
        let custos = published(&["custos"]);

        for (raw_action, raw_target) in [
            ("acta::document::read", "acta::document::d1"),
            ("custos::group::read", "acta::document::d1"),
            ("custos::grant::create", "acta::document::d1"),
        ] {
            assert!(
                matches!(
                    require_answerable(&custos, &action(raw_action), &[target(raw_target)]),
                    Err(ApiError::InvalidInput { .. })
                ),
                "{raw_action} on {raw_target}"
            );
        }
    }

    #[test]
    fn a_product_action_must_match_its_targets_product_unless_it_delegates() {
        let both = published(&["custos", "acta"]);
        let document = [target("acta::document::d1")];

        assert!(matches!(
            require_answerable(&both, &action("custos::group::read"), &document),
            Err(ApiError::InvalidInput { .. })
        ));
        assert!(require_answerable(&both, &action("custos::grant::create"), &document).is_ok());
        assert!(require_answerable(&both, &action("acta::document::read"), &document).is_ok());
    }

    #[test]
    fn every_target_of_a_batch_is_checked() {
        let custos = published(&["custos"]);
        let targets = [target("custos::group::g1"), target("acta::document::d1")];

        assert!(matches!(
            require_answerable(&custos, &action("custos::group::read"), &targets),
            Err(ApiError::InvalidInput { .. })
        ));
        assert!(require_answerable(&custos, &action("custos::group::read"), &[]).is_ok());
    }

    #[test]
    fn decisions_map_onto_the_wire_without_their_reasons() {
        let read = action("custos::group::read");
        let group = target("custos::group::g1");

        assert_eq!(
            answered(&read, &group, &evaluated(Decision::Allowed)),
            AuthorizeDecision::Allow
        );
        for because in [DenyCause::NotGranted, DenyCause::DenyRuleEnforced] {
            assert_eq!(
                answered(&read, &group, &evaluated(Decision::Denied { because })),
                AuthorizeDecision::Deny
            );
        }
        assert_eq!(
            answered(&read, &group, &evaluated(Decision::NotFound)),
            AuthorizeDecision::NotFound
        );
    }
}
