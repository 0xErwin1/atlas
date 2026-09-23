//! The authorization service (EVAL-5): loads the facts one request needs
//! and runs the pure evaluator over them.
//!
//! Per request the service makes at most one `resource_facts` call per
//! target product and one store load. The store load is one logical query
//! for grants and deny rules, which an implementation may run as a few
//! statements; the membership source and, for principal sets a loaded row
//! names, the owning provider's `members_of` are consulted alongside it.
//! The visibility filter makes no `resource_facts` call.
//!
//! Every provider call runs under the configured timeout. A provider
//! failure or timeout makes the affected targets unavailable: a single
//! authorization and effective actions fail with
//! `EvalError::FactsUnavailable`, a batch reports those targets as
//! `NotFound`. A store or membership source failure fails the request with
//! `FactsUnavailable`; nothing is ever narrowed into an allow (AVAIL-1).
//!
//! The credential ceiling and the deny mode are supplied, not derived here.

mod loader;
mod timeout;

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use std::time::Duration;

use crate::eval::{
    BatchRequest, BatchTarget, Catalog, Ceiling, DenyMode, EffectiveActions, EffectiveRequest,
    EvalError, EvalRequest, Evaluated, Existence, FactFailure, VisibilityPredicate, evaluate,
    evaluate_batch, is_delegation_action,
};
use crate::ids::PrincipalId;
use crate::ports::authorize::{AuthorizationFactsStore, GroupMembershipSource, ProductScope};
use async_trait::async_trait;
use atlas_core::capabilities::ResourceProvider;
use atlas_core::ids::{ActionId, ResourceRef};
use loader::LoadedFacts;

/// The provider timeout used unless the settings say otherwise.
pub const DEFAULT_PROVIDER_TIMEOUT: Duration = Duration::from_secs(2);

/// The timer provider calls race against. The composition root supplies it
/// from its async runtime.
#[async_trait]
pub trait Sleeper: Send + Sync {
    /// Completes after `duration`.
    async fn sleep(&self, duration: Duration);
}

/// The resource providers the service consults, keyed by product.
#[derive(Clone, Default)]
pub struct ProviderSet {
    providers: HashMap<String, Arc<dyn ResourceProvider>>,
}

impl ProviderSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `provider` for `product`, replacing any earlier one.
    pub fn with(mut self, product: &str, provider: Arc<dyn ResourceProvider>) -> Self {
        self.providers.insert(product.to_string(), provider);
        self
    }

    fn get(&self, product: &str) -> Option<&Arc<dyn ResourceProvider>> {
        self.providers.get(product)
    }
}

/// The configuration of one service instance.
pub struct AuthorizationSettings {
    /// Resolves stored built-in and custom-role grants into actions.
    pub catalog: Catalog,
    /// The configured explicit-deny mode.
    pub deny_mode: DenyMode,
    /// The bound on every provider call.
    pub provider_timeout: Duration,
}

impl AuthorizationSettings {
    /// Settings with the [`DEFAULT_PROVIDER_TIMEOUT`].
    pub fn new(catalog: Catalog, deny_mode: DenyMode) -> Self {
        Self {
            catalog,
            deny_mode,
            provider_timeout: DEFAULT_PROVIDER_TIMEOUT,
        }
    }
}

/// Who is asking: the authenticated principal, whether it is root, and the
/// ceiling of the credential it acts through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorContext {
    pub principal: PrincipalId,
    pub is_root: bool,
    pub ceiling: Ceiling,
}

/// Loads facts through the providers, the store and the membership source,
/// and evaluates them.
pub struct AuthorizationService<S, M> {
    providers: ProviderSet,
    store: S,
    membership: M,
    sleeper: Arc<dyn Sleeper>,
    settings: AuthorizationSettings,
}

impl<S: AuthorizationFactsStore, M: GroupMembershipSource> AuthorizationService<S, M> {
    /// Builds the service. `sleeper` must be backed by a real timer: the
    /// provider timeout is only as good as it, and a sleeper that never
    /// fires lets a hanging provider hang the request forever.
    pub fn new(
        providers: ProviderSet,
        store: S,
        membership: M,
        sleeper: Arc<dyn Sleeper>,
        settings: AuthorizationSettings,
    ) -> Self {
        Self {
            providers,
            store,
            membership,
            sleeper,
            settings,
        }
    }

    /// Authorizes `action` on `target`. An unavailable target is a
    /// `FactsUnavailable` error carrying the provider cause.
    pub async fn authorize(
        &self,
        actor: &ActorContext,
        action: &ActionId,
        target: &ResourceRef,
    ) -> Result<Evaluated, EvalError> {
        let Some(facts) = self.target_facts(std::slice::from_ref(target)).await.pop() else {
            return Err(EvalError::FactsUnavailable {
                cause: FactFailure::Internal,
            });
        };

        let request = EvalRequest {
            actor: actor.principal,
            is_root: actor.is_root,
            action: action.clone(),
            target: target.clone(),
            path: facts.path,
            existence: facts.existence,
            ceiling: actor.ceiling.clone(),
            deny_mode: self.settings.deny_mode,
        };

        if actor.is_root || facts.existence != Existence::Exists {
            return evaluate(&request, &LoadedFacts::empty(actor.principal).facts());
        }

        let loaded = self
            .load_facts(actor.principal, products_of([target]))
            .await?;

        evaluate(&request, &loaded.facts())
    }

    /// Authorizes `action` on every target, returning one result per target
    /// in input order. An unavailable target is `NotFound` (EVAL-4); a
    /// store or membership failure fails the whole batch.
    pub async fn authorize_batch(
        &self,
        actor: &ActorContext,
        action: &ActionId,
        targets: &[ResourceRef],
    ) -> Result<Vec<Result<Evaluated, EvalError>>, EvalError> {
        let answers = self.target_facts(targets).await;
        let request = BatchRequest {
            actor: actor.principal,
            is_root: actor.is_root,
            action: action.clone(),
            ceiling: actor.ceiling.clone(),
            deny_mode: self.settings.deny_mode,
            targets: targets
                .iter()
                .zip(answers)
                .map(|(target, facts)| BatchTarget {
                    target: target.clone(),
                    path: facts.path,
                    existence: facts.existence,
                })
                .collect(),
        };

        let existing: Vec<&ResourceRef> = request
            .targets
            .iter()
            .filter(|entry| entry.existence == Existence::Exists)
            .map(|entry| &entry.target)
            .collect();

        if actor.is_root || existing.is_empty() {
            return evaluate_batch(&request, &LoadedFacts::empty(actor.principal).facts());
        }

        let loaded = self
            .load_facts(actor.principal, products_of(existing))
            .await?;

        evaluate_batch(&request, &loaded.facts())
    }

    /// Compiles the list predicate for `action` on resources of `kind`
    /// without any `resource_facts` call. A delegation action loads facts
    /// of every product.
    pub async fn visibility_filter(
        &self,
        actor: &ActorContext,
        kind: &str,
        action: &ActionId,
    ) -> Result<VisibilityPredicate, EvalError> {
        let loaded = if actor.is_root {
            LoadedFacts::empty(actor.principal)
        } else {
            let scope = if is_delegation_action(action) {
                ProductScope::All
            } else {
                ProductScope::Products(vec![action.product().to_string()])
            };

            self.load_facts(actor.principal, scope).await?
        };

        crate::eval::visibility_filter(
            actor.principal,
            actor.is_root,
            kind,
            action,
            &actor.ceiling,
            self.settings.deny_mode,
            &loaded.facts(),
        )
    }

    /// The actor's effective actions on `target`, the only sanctioned input
    /// of [`crate::eval::can_delegate`]. An unavailable target is a
    /// `FactsUnavailable` error.
    pub async fn effective_actions(
        &self,
        actor: &ActorContext,
        target: &ResourceRef,
    ) -> Result<EffectiveActions, EvalError> {
        let Some(facts) = self.target_facts(std::slice::from_ref(target)).await.pop() else {
            return Err(EvalError::FactsUnavailable {
                cause: FactFailure::Internal,
            });
        };

        let request = EffectiveRequest {
            actor: actor.principal,
            is_root: actor.is_root,
            target: target.clone(),
            path: facts.path,
            existence: facts.existence,
            ceiling: actor.ceiling.clone(),
            deny_mode: self.settings.deny_mode,
        };

        if actor.is_root || facts.existence != Existence::Exists {
            return crate::eval::effective_actions(
                &request,
                &LoadedFacts::empty(actor.principal).facts(),
            );
        }

        let loaded = self
            .load_facts(actor.principal, products_of([target]))
            .await?;

        crate::eval::effective_actions(&request, &loaded.facts())
    }
}

/// The distinct products of `targets`, in a stable order.
fn products_of<'a>(targets: impl IntoIterator<Item = &'a ResourceRef>) -> ProductScope {
    let products: BTreeSet<String> = targets
        .into_iter()
        .map(|target| target.product().to_string())
        .collect();

    ProductScope::Products(products.into_iter().collect())
}
