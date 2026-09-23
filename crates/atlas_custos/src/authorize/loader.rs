//! Fact loading for the authorization service: one provider query per
//! product for the targets, and one store load for the stored facts.

use std::collections::{BTreeMap, BTreeSet};

use super::AuthorizationService;
use super::timeout::{BoxFuture, within};
use crate::eval::{
    DenyRule, EvalError, EvaluationFacts, Existence, FactFailure, Grant, Membership,
    MembershipFacts, Subject, grant_spec,
};
use crate::ids::{GroupId, PrincipalId};
use crate::ports::authorize::{
    AuthorizationFactsStore, DeclaredSet, GroupMembershipSource, ProductScope,
};
use atlas_core::capabilities::{CapabilityError, ResourceExistence};
use atlas_core::ids::{PrincipalSetId, ResourcePath, ResourceRef};

/// What the provider reported for one target.
pub(super) struct TargetFacts {
    pub(super) existence: Existence,
    pub(super) path: Option<ResourcePath>,
}

impl TargetFacts {
    pub(super) fn unavailable(cause: FactFailure) -> Self {
        Self {
            existence: Existence::Unavailable(cause),
            path: None,
        }
    }
}

/// The evaluation facts resolved from one store load, bound to the actor.
pub(super) struct LoadedFacts {
    grants: Vec<Grant>,
    denies: Vec<DenyRule>,
    membership: MembershipFacts,
}

impl LoadedFacts {
    /// No stored facts: what root and requests on absent targets evaluate
    /// against, since neither consults grants.
    pub(super) fn empty(actor: PrincipalId) -> Self {
        Self {
            grants: Vec::new(),
            denies: Vec::new(),
            membership: MembershipFacts::new(actor),
        }
    }

    pub(super) fn facts(&self) -> EvaluationFacts<'_> {
        EvaluationFacts {
            grants: &self.grants,
            denies: &self.denies,
            membership: &self.membership,
        }
    }
}

impl<S: AuthorizationFactsStore, M: GroupMembershipSource> AuthorizationService<S, M> {
    /// Runs one provider call under the configured timeout, mapping a
    /// provider error and a timeout to their typed causes.
    async fn provider_call<'a, T>(
        &'a self,
        work: BoxFuture<'a, Result<T, CapabilityError>>,
    ) -> Result<T, FactFailure> {
        let timer = self.sleeper.sleep(self.settings.provider_timeout);

        match within(work, timer).await {
            Some(Ok(answer)) => Ok(answer),
            Some(Err(_)) => Err(FactFailure::Provider),
            None => Err(FactFailure::Timeout),
        }
    }

    /// Asks each product's provider once for the facts of its targets, in
    /// input order. A failed, timed-out or incomplete answer makes that
    /// product's affected targets unavailable with the typed cause; a
    /// product without a registered provider is an internal fault.
    pub(super) async fn target_facts(&self, targets: &[ResourceRef]) -> Vec<TargetFacts> {
        let mut by_product: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (index, target) in targets.iter().enumerate() {
            by_product.entry(target.product()).or_default().push(index);
        }

        let mut answers: Vec<TargetFacts> = targets
            .iter()
            .map(|_| TargetFacts::unavailable(FactFailure::Internal))
            .collect();

        for (product, indices) in by_product {
            let Some(provider) = self.providers.get(product) else {
                continue;
            };

            let refs: Vec<ResourceRef> = indices
                .iter()
                .filter_map(|index| targets.get(*index).cloned())
                .collect();
            let reported = self.provider_call(provider.resource_facts(&refs)).await;

            for (index, target) in indices.iter().zip(&refs) {
                let fact = match &reported {
                    Err(cause) => TargetFacts::unavailable(*cause),
                    Ok(reported) => match reported.iter().find(|fact| fact.resource == *target) {
                        Some(fact) if fact.existence == ResourceExistence::Exists => TargetFacts {
                            existence: Existence::Exists,
                            path: fact.path.clone(),
                        },
                        Some(_) => TargetFacts {
                            existence: Existence::Missing,
                            path: None,
                        },
                        None => TargetFacts::unavailable(FactFailure::Provider),
                    },
                };

                if let Some(slot) = answers.get_mut(*index) {
                    *slot = fact;
                }
            }
        }

        answers
    }

    /// Loads the actor's stored facts for `scope` once and resolves them
    /// into evaluator facts bound to the actor.
    ///
    /// Group memberships come from the membership source; groups the rows
    /// name that the actor is not in are confirmed nonmembers. Principal
    /// sets are resolved through the owning provider's `members_of` only for
    /// sets the rows name; an unresolvable set stays nonmember (GRANT-7).
    /// Those calls run one after another, each under the provider timeout,
    /// so a request naming `n` sets can wait up to `n` timeouts; a failing
    /// or timed-out set never stops the remaining ones from resolving.
    /// A membership source or store failure fails closed, and a stored row
    /// that no longer resolves against the catalog is inconsistent.
    pub(super) async fn load_facts(
        &self,
        actor: PrincipalId,
        scope: ProductScope,
    ) -> Result<LoadedFacts, EvalError> {
        let internal = |_| EvalError::FactsUnavailable {
            cause: FactFailure::Internal,
        };

        let groups = self.membership.groups_of(actor).await.map_err(internal)?;
        let stored = self
            .store
            .load(&scope, actor, &groups, &self.declared_sets())
            .await
            .map_err(internal)?;

        let grants = stored
            .grants
            .iter()
            .map(|record| {
                grant_spec(record, &self.settings.catalog, &stored.custom_roles)
                    .and_then(|spec| self.settings.catalog.resolve_grant(spec))
                    .map_err(|error| EvalError::InconsistentFacts {
                        detail: format!("stored grant {} does not resolve: {error}", record.id),
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let denies = stored
            .denies
            .iter()
            .map(DenyRule::try_from)
            .collect::<Result<Vec<_>, _>>()?;

        let subjects: Vec<&Subject> = grants
            .iter()
            .map(Grant::subject)
            .chain(denies.iter().map(DenyRule::subject))
            .collect();
        let mut membership = MembershipFacts::new(actor);

        for group in referenced_groups(&subjects) {
            let state = if groups.contains(&group) {
                Membership::Member
            } else {
                Membership::NotMember
            };
            membership.set_group(group, state);
        }

        for set in referenced_sets(&subjects) {
            let state = self.set_membership(actor, &set).await;
            membership.set_principal_set(set, state);
        }

        Ok(LoadedFacts {
            grants,
            denies,
            membership,
        })
    }

    /// Every principal set the catalog declares. The load takes all of them,
    /// not only those of the products in scope: a grant or deny may name a
    /// set declared by another product than its target's, and leaving such a
    /// deny unloaded would stop it from applying.
    fn declared_sets(&self) -> Vec<DeclaredSet> {
        self.settings
            .catalog
            .declared_principal_sets()
            .into_iter()
            .map(|(product, name)| DeclaredSet {
                product: product.to_string(),
                name: name.to_string(),
            })
            .collect()
    }

    /// The actor's membership in `set`, from the provider owning the set's
    /// scope. Provider identities are compared with the actor's id as
    /// canonical lowercase hyphenated UUID text; any other spelling leaves
    /// the actor a nonmember, so a deny addressed to the set would not
    /// reach it.
    async fn set_membership(&self, actor: PrincipalId, set: &PrincipalSetId) -> Membership {
        let Some(provider) = self.providers.get(set.scope().product()) else {
            return Membership::Indeterminate;
        };

        match self.provider_call(provider.members_of(set)).await {
            Ok(members) => {
                let actor_id = actor.0.to_string();

                if members.iter().any(|member| member.as_str() == actor_id) {
                    Membership::Member
                } else {
                    Membership::NotMember
                }
            }
            Err(_) => Membership::Indeterminate,
        }
    }
}

fn referenced_groups(subjects: &[&Subject]) -> BTreeSet<GroupId> {
    subjects
        .iter()
        .filter_map(|subject| match subject {
            Subject::Group(group) => Some(*group),
            Subject::Principal(_) | Subject::PrincipalSet(_) => None,
        })
        .collect()
}

fn referenced_sets(subjects: &[&Subject]) -> Vec<PrincipalSetId> {
    let mut sets: Vec<PrincipalSetId> = Vec::new();

    for subject in subjects {
        if let Subject::PrincipalSet(set) = subject
            && !sets.contains(set)
        {
            sets.push(set.clone());
        }
    }

    sets
}
