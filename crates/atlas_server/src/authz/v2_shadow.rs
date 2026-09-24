//! Shadow authorization (`v2-e7-s2`, D-E7S2-2/D-E7S2-4): beside every V1
//! decision on a route that declares a V2 target, ask the V2 authorization
//! service the declared question and record whether the two agree. The V1
//! decision is returned unchanged; the shadow never writes audit rows,
//! never returns a reason, and stops silently when its budget is spent.
//!
//! Off by default (`ATLAS_CUSTOS_SHADOW_AUTHORIZE=off`): production traffic
//! pays nothing until the mode is `log` (structured line) or `metrics`
//! (line plus counter).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::MatchedPath;
use axum::http::request::Parts;
use metrics::counter;
use uuid::Uuid;

use atlas_acta_postgres::repos::identity::Workspace;
use atlas_core::ids::{ActionId, ResourceRef};
use atlas_core::registry::{Registry, TargetSource, V2Target};
use atlas_custos::eval::Decision;

use crate::{
    auth::middleware::Principal as AuthPrincipal,
    authz::v2_caller::{caller, question_actor},
    config::ShadowMode,
    error::ApiError,
    observability::route_index::RouteTag,
    state::AppState,
};

/// The whole shadow evaluation, actor resolution included, must finish
/// within this budget or be counted as unavailable (D-E7S2-4). Separate
/// from the provider timeout the service enforces internally.
///
/// Exhausting the budget drops the evaluation future, not the queries it
/// started: a checked-out connection returns to the pool only once its
/// in-flight statement finishes on the server side. Under sustained
/// timeouts the shadow keeps checking out connections it never waits for,
/// so a slow provider can churn the pool the V1 path shares with it.
pub const SHADOW_BUDGET: Duration = Duration::from_millis(100);

/// Counter of shadow comparisons, labelled `component`, `operation` (the
/// route, registry-derived) and `outcome`.
pub const SHADOW_TOTAL: &str = "atlas_v2_shadow_total";

/// `operation_id -> V2Target`, built once from the registry: the V2
/// question each declared route asks.
#[derive(Debug, Default)]
pub struct ShadowIndex(HashMap<String, V2Target>);

impl ShadowIndex {
    pub fn from_registry(registry: &Registry) -> Self {
        let mut map = HashMap::new();

        for entry in registry.entries() {
            for route in &entry.api.routes {
                if let Some(target) = &route.v2 {
                    map.insert(route.operation_id.clone(), target.clone());
                }
            }
        }

        Self(map)
    }

    pub fn get(&self, operation_id: &str) -> Option<&V2Target> {
        self.0.get(operation_id)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// What V1 decided, as far as the shadow compares it. Any other error
/// (unauthenticated, internal) is not a decision and is not compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum V1Outcome {
    Allow,
    Deny,
    NotFound,
}

impl V1Outcome {
    pub fn of<T>(outcome: &Result<T, ApiError>) -> Option<Self> {
        match outcome {
            Ok(_) => Some(Self::Allow),
            Err(ApiError::Forbidden { .. }) => Some(Self::Deny),
            Err(ApiError::NotFound) => Some(Self::NotFound),
            Err(_) => None,
        }
    }
}

/// How the two decisions relate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShadowOutcome {
    Agree,
    V1AllowV2Deny,
    V1AllowV2NotFound,
    V1DenyV2Allow,
    V1NotFoundV2Allow,
    V2Unavailable,
    /// The route declares a target but the request never resolved one (or
    /// its principal): V1 refused before the resource was located, so
    /// there is no V2 question to ask.
    Skipped,
}

impl ShadowOutcome {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agree => "agree",
            Self::V1AllowV2Deny => "v1_allow_v2_deny",
            Self::V1AllowV2NotFound => "v1_allow_v2_not_found",
            Self::V1DenyV2Allow => "v1_deny_v2_allow",
            Self::V1NotFoundV2Allow => "v1_notfound_v2_allow",
            Self::V2Unavailable => "v2_unavailable",
            Self::Skipped => "skipped",
        }
    }
}

/// Relates a V1 decision to the V2 one. Two refusals agree whatever their
/// flavour (a deny beside a not-found is still a refusal on both sides);
/// only an allow on one side and a refusal on the other is a mismatch.
pub fn compare(v1: V1Outcome, v2: Option<&Decision>) -> ShadowOutcome {
    match (v1, v2) {
        (_, None) => ShadowOutcome::V2Unavailable,
        (V1Outcome::Allow, Some(Decision::Allowed)) => ShadowOutcome::Agree,
        (V1Outcome::Allow, Some(Decision::Denied { .. })) => ShadowOutcome::V1AllowV2Deny,
        (V1Outcome::Allow, Some(Decision::NotFound)) => ShadowOutcome::V1AllowV2NotFound,
        (V1Outcome::Deny, Some(Decision::Allowed)) => ShadowOutcome::V1DenyV2Allow,
        (V1Outcome::NotFound, Some(Decision::Allowed)) => ShadowOutcome::V1NotFoundV2Allow,
        (V1Outcome::Deny | V1Outcome::NotFound, Some(_)) => ShadowOutcome::Agree,
    }
}

fn decision_name(decision: &Decision) -> &'static str {
    match decision {
        Decision::Allowed => "allowed",
        Decision::Denied { .. } => "denied",
        Decision::NotFound => "not_found",
    }
}

/// The V2 question one request would ask, gathered by the V1 extractor as
/// it resolves the request. Empty (and free) when the shadow is off or the
/// route declares no target.
pub struct ShadowProbe {
    route: Option<(RouteTag, V2Target)>,
    principal: Option<AuthPrincipal>,
    target: Option<ResourceRef>,
}

impl ShadowProbe {
    /// Looks the request's route up in the registry-derived indexes. Only
    /// when the shadow mode is on and the route declares a V2 target does
    /// the probe become active; otherwise it records nothing.
    pub fn new(state: &AppState, parts: &Parts) -> Self {
        match parts.extensions.get::<MatchedPath>() {
            Some(template) => Self::for_route(state, &parts.method, template.as_str()),
            None => Self::inactive(),
        }
    }

    /// The probe for one `(method, mounted template)` pair, the lookup
    /// [`Self::new`] performs on the request's `MatchedPath`.
    pub fn for_route(state: &AppState, method: &axum::http::Method, template: &str) -> Self {
        if state.shadow_authorize == ShadowMode::Off {
            return Self::inactive();
        }

        let Some(tag) = state.route_index.get(method, template) else {
            return Self::inactive();
        };
        let Some(target) = state.shadow_index.get(&tag.operation) else {
            return Self::inactive();
        };

        Self {
            route: Some((tag.clone(), target.clone())),
            principal: None,
            target: None,
        }
    }

    fn inactive() -> Self {
        Self {
            route: None,
            principal: None,
            target: None,
        }
    }

    pub fn active(&self) -> bool {
        self.route.is_some()
    }

    pub fn set_principal(&mut self, principal: AuthPrincipal) {
        if self.active() {
            self.principal = Some(principal);
        }
    }

    /// Resolves the declared target from what the extractor has: the
    /// workspace, the path parameters, and the resource it resolved (for
    /// the sources that name one).
    pub fn set_target(
        &mut self,
        params: &HashMap<String, String>,
        workspace: &Workspace,
        resolved: impl FnOnce() -> Option<ResourceRef>,
    ) {
        let Some((_, declared)) = &self.route else {
            return;
        };

        self.target = match &declared.target {
            TargetSource::Workspace => acta_ref("workspace", workspace.id.0),
            TargetSource::Comment => param_ref("comment", params.get("comment_id")),
            TargetSource::Attachment => param_ref("attachment", params.get("attachment_id")),
            TargetSource::WorkspaceChild { kind, param } => param_ref(kind, params.get(*param)),
            TargetSource::Project
            | TargetSource::Folder
            | TargetSource::Document
            | TargetSource::Board
            | TargetSource::Task => resolved(),
        };
    }
}

/// `acta::<kind>::<canonical uuid>`.
pub fn acta_ref(kind: &str, id: Uuid) -> Option<ResourceRef> {
    ResourceRef::new("acta", kind, &id.to_string()).ok()
}

fn param_ref(kind: &str, raw: Option<&String>) -> Option<ResourceRef> {
    let id = Uuid::parse_str(raw?).ok()?;
    acta_ref(kind, id)
}

/// Runs the shadow question for a finished V1 decision (`None` when the
/// request ended without one) and records the comparison. Never returns
/// an error.
pub async fn observe(state: &AppState, probe: &ShadowProbe, v1: Option<V1Outcome>) {
    let Some((tag, declared)) = &probe.route else {
        return;
    };
    let Some(v1) = v1 else {
        return;
    };
    let (Some(principal), Some(target)) = (&probe.principal, &probe.target) else {
        record(
            state.shadow_authorize,
            tag,
            declared,
            v1,
            None,
            ShadowOutcome::Skipped,
        );
        return;
    };

    let decision = tokio::time::timeout(
        SHADOW_BUDGET,
        shadow_decision(state, principal.clone(), &declared.action, target),
    )
    .await
    .ok()
    .flatten();

    let outcome = compare(v1, decision.as_ref());
    record(
        state.shadow_authorize,
        tag,
        declared,
        v1,
        decision.as_ref(),
        outcome,
    );
}

async fn shadow_decision(
    state: &AppState,
    principal: AuthPrincipal,
    action: &ActionId,
    target: &ResourceRef,
) -> Option<Decision> {
    let caller = caller(state, principal).await.ok()?;
    let actor = question_actor(state, caller).await.ok()?;

    state
        .authorization
        .authorize(&actor, action, target)
        .await
        .ok()
        .map(|evaluated| evaluated.decision)
}

/// Emits the comparison: `agree` at debug, everything else (a mismatch,
/// an unavailable evaluator, a skipped question) at info, and the counter
/// in `Metrics` mode.
fn record(
    mode: ShadowMode,
    tag: &RouteTag,
    declared: &V2Target,
    v1: V1Outcome,
    v2: Option<&Decision>,
    outcome: ShadowOutcome,
) {
    let v2 = match outcome {
        ShadowOutcome::Skipped => "skipped",
        _ => v2.map(decision_name).unwrap_or("unavailable"),
    };

    match outcome {
        ShadowOutcome::Agree => tracing::debug!(
            target: "authz.v2_shadow",
            component = %tag.component,
            operation = %tag.operation,
            kind = declared.kind,
            action = %declared.action,
            v1 = ?v1,
            v2,
            outcome = outcome.as_str(),
            "V2 shadow authorization compared"
        ),
        _ => tracing::info!(
            target: "authz.v2_shadow",
            component = %tag.component,
            operation = %tag.operation,
            kind = declared.kind,
            action = %declared.action,
            v1 = ?v1,
            v2,
            outcome = outcome.as_str(),
            "V2 shadow authorization compared"
        ),
    }

    if mode == ShadowMode::Metrics {
        let component: Arc<str> = tag.component.clone();
        let operation: Arc<str> = tag.operation.clone();
        counter!(
            SHADOW_TOTAL,
            "component" => component.to_string(),
            "operation" => operation.to_string(),
            "outcome" => outcome.as_str(),
        )
        .increment(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use atlas_custos::eval::DenyCause;

    fn denied() -> Decision {
        Decision::Denied {
            because: DenyCause::NotGranted,
        }
    }

    #[test]
    fn agreeing_decisions_and_refusals_of_any_flavour_agree() {
        assert_eq!(
            compare(V1Outcome::Allow, Some(&Decision::Allowed)),
            ShadowOutcome::Agree
        );
        assert_eq!(
            compare(V1Outcome::Deny, Some(&denied())),
            ShadowOutcome::Agree
        );
        assert_eq!(
            compare(V1Outcome::Deny, Some(&Decision::NotFound)),
            ShadowOutcome::Agree
        );
        assert_eq!(
            compare(V1Outcome::NotFound, Some(&denied())),
            ShadowOutcome::Agree
        );
    }

    #[test]
    fn every_mismatch_direction_has_its_own_outcome() {
        assert_eq!(
            compare(V1Outcome::Allow, Some(&denied())),
            ShadowOutcome::V1AllowV2Deny
        );
        assert_eq!(
            compare(V1Outcome::Allow, Some(&Decision::NotFound)),
            ShadowOutcome::V1AllowV2NotFound
        );
        assert_eq!(
            compare(V1Outcome::Deny, Some(&Decision::Allowed)),
            ShadowOutcome::V1DenyV2Allow
        );
        assert_eq!(
            compare(V1Outcome::NotFound, Some(&Decision::Allowed)),
            ShadowOutcome::V1NotFoundV2Allow
        );
    }

    #[test]
    fn a_missing_v2_answer_is_unavailable_whatever_v1_said() {
        for v1 in [V1Outcome::Allow, V1Outcome::Deny, V1Outcome::NotFound] {
            assert_eq!(compare(v1, None), ShadowOutcome::V2Unavailable);
        }
    }

    #[test]
    fn only_v1_decisions_are_compared() {
        assert_eq!(
            V1Outcome::of(&Ok::<(), ApiError>(())),
            Some(V1Outcome::Allow)
        );
        assert_eq!(
            V1Outcome::of(&Err::<(), ApiError>(ApiError::Forbidden {
                message: "x".into()
            })),
            Some(V1Outcome::Deny)
        );
        assert_eq!(
            V1Outcome::of(&Err::<(), ApiError>(ApiError::NotFound)),
            Some(V1Outcome::NotFound)
        );
        assert_eq!(
            V1Outcome::of(&Err::<(), ApiError>(ApiError::Unauthorized)),
            None
        );
    }

    #[test]
    fn the_shadow_index_holds_every_declared_route_once() {
        let registry = atlas_core::registry::build(crate::reg5::reg5_component_entries(
            crate::reg5::StorageBackend::Filesystem,
        ))
        .expect("REG-5 entries build");
        let declared: usize = registry
            .entries()
            .iter()
            .flat_map(|entry| entry.api.routes.iter())
            .filter(|route| route.v2.is_some())
            .count();

        let index = ShadowIndex::from_registry(&registry);

        assert_eq!(index.len(), declared);
        assert!(!index.is_empty());
        let get_document = index.get("get_document").expect("get_document is declared");
        assert_eq!(get_document.kind, "document");
        assert_eq!(get_document.action.to_string(), "acta::document::read");
        assert!(
            index.get("acta_health").is_none(),
            "public routes declare nothing"
        );
    }

    /// An active probe whose request never resolved a target (a 404 before
    /// the resource was located) still leaves a trace: the `skipped`
    /// series, so a route that never asks its question is visible.
    #[test]
    fn a_question_without_a_target_is_counted_as_skipped() {
        let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
        let handle = recorder.handle();
        let _guard = metrics::set_default_local_recorder(&recorder);

        let tag = RouteTag {
            component: Arc::from("acta"),
            operation: Arc::from("get_document"),
        };
        let declared = V2Target {
            kind: "document",
            action: "acta::document::read"
                .parse::<ActionId>()
                .expect("valid action"),
            target: TargetSource::Document,
        };

        record(
            ShadowMode::Metrics,
            &tag,
            &declared,
            V1Outcome::NotFound,
            None,
            ShadowOutcome::Skipped,
        );

        let rendered = handle.render();
        assert!(
            rendered.contains(
                "atlas_v2_shadow_total{component=\"acta\",operation=\"get_document\",outcome=\"skipped\"} 1"
            ),
            "expected one skipped series, got:\n{rendered}"
        );
        assert_eq!(ShadowOutcome::Skipped.as_str(), "skipped");
    }

    #[test]
    fn targets_resolve_to_canonical_acta_refs() {
        let id = Uuid::now_v7();
        let reference = acta_ref("document", id).expect("valid ref");
        assert_eq!(reference.to_string(), format!("acta::document::{id}"));

        let upper = id.to_string().to_uppercase();
        let from_param = param_ref("comment", Some(&upper)).expect("parses");
        assert_eq!(from_param.to_string(), format!("acta::comment::{id}"));
        assert!(param_ref("comment", Some(&"not-a-uuid".to_string())).is_none());
        assert!(param_ref("comment", None).is_none());
    }
}
