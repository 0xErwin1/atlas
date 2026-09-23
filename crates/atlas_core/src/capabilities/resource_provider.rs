use async_trait::async_trait;

use crate::ids::action_id::ActionId;
use crate::ids::principal_id::PrincipalId;
use crate::ids::principal_set_id::PrincipalSetId;
use crate::ids::resource_path::{PathSegment, ResourcePath};
use crate::ids::resource_ref::ResourceRef;

use super::error::CapabilityError;

/// The set of resource kinds, actions, role definitions, and principal set
/// names a `ResourceProvider` declares support for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderCatalog {
    /// The resource kinds this provider recognizes.
    pub resource_kinds: Vec<String>,
    /// The actions this provider can validate.
    pub actions: Vec<ActionId>,
    /// The role definitions this provider recognizes.
    pub role_definitions: Vec<String>,
    /// The declared principal set names (e.g. `acta.members`), not concrete
    /// instances. Matches `registry::Authorization.principal_sets` so
    /// V2-E3's SHELL-REG-4 cross-check compares like with like.
    pub principal_sets: Vec<String>,
    /// The versioned built-in roles this provider declares, with their
    /// actions. `role_definitions` keeps the plain names the registry
    /// cross-check compares.
    pub role_definitions_v2: Vec<RoleDefinition>,
}

/// One version of a built-in role a provider declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleDefinition {
    pub name: String,
    pub version: u32,
    pub actions: Vec<ActionId>,
}

/// Whether a provider found a resource.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceExistence {
    Exists,
    Missing,
}

/// A provider's answer for one resource: whether it exists and, when it
/// does, its current canonical path from the product root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceFacts {
    pub resource: ResourceRef,
    pub existence: ResourceExistence,
    pub path: Option<ResourcePath>,
}

/// Resolves and validates resources belonging to one product, implementing
/// CUSTOS-PROV-1.
#[async_trait]
pub trait ResourceProvider: Send + Sync {
    /// Confirms whether `resource` currently exists.
    ///
    /// Returns `Ok(false)` for a definitive negative answer, and
    /// `Err(CapabilityError::Unavailable)` when the provider could not
    /// reach its backend to answer at all.
    async fn validate_ref(&self, resource: &ResourceRef) -> Result<bool, CapabilityError>;

    /// Returns the human-readable path of `resource` (e.g. breadcrumbs).
    async fn path_of(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError>;

    /// Returns the ancestor chain of `resource`, nearest first.
    async fn ancestors(&self, resource: &ResourceRef) -> Result<Vec<ResourceRef>, CapabilityError>;

    /// Resolves the flattened, kind-erased members of `set`.
    ///
    /// Custos compares each returned identity with the actor's id as
    /// canonical lowercase hyphenated UUID text. A provider returning
    /// another spelling makes the actor a nonmember: safe for grants, but a
    /// deny addressed to the set then does not reach the actor.
    async fn members_of(&self, set: &PrincipalSetId) -> Result<Vec<PrincipalId>, CapabilityError>;

    /// Describes this provider's supported resource kinds, actions, role
    /// definitions, and principal set names.
    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError>;

    /// Reports existence and current path for every resource, one
    /// [`ResourceFacts`] per input in input order (PROV-1). A provider
    /// answers the whole slice in one call, so an authorization request
    /// costs one provider round trip however many targets it has.
    ///
    /// The default serves legacy providers through [`Self::validate_ref`]
    /// and [`Self::path_of`], which costs up to two calls per resource;
    /// providers should override it. `path_of` must return the breadcrumbs
    /// from the product root down to the resource itself.
    async fn resource_facts(
        &self,
        resources: &[ResourceRef],
    ) -> Result<Vec<ResourceFacts>, CapabilityError> {
        let mut facts = Vec::with_capacity(resources.len());

        for resource in resources {
            if !self.validate_ref(resource).await? {
                facts.push(ResourceFacts {
                    resource: resource.clone(),
                    existence: ResourceExistence::Missing,
                    path: None,
                });
                continue;
            }

            let breadcrumbs = self.path_of(resource).await?;

            facts.push(ResourceFacts {
                resource: resource.clone(),
                existence: ResourceExistence::Exists,
                path: Some(path_from_breadcrumbs(resource, &breadcrumbs)?),
            });
        }

        Ok(facts)
    }
}

/// Builds the canonical path of `resource` from its root-first breadcrumbs,
/// which must end at the resource and stay within its product.
fn path_from_breadcrumbs(
    resource: &ResourceRef,
    breadcrumbs: &[ResourceRef],
) -> Result<ResourcePath, CapabilityError> {
    let invalid = || {
        CapabilityError::invalid(format!(
            "breadcrumbs of {resource} do not form its path within its product"
        ))
    };

    let Some((root, rest)) = breadcrumbs.split_first() else {
        return Err(invalid());
    };

    if breadcrumbs.last() != Some(resource)
        || breadcrumbs
            .iter()
            .any(|crumb| crumb.product() != resource.product())
    {
        return Err(invalid());
    }

    let segment =
        |crumb: &ResourceRef| PathSegment::new(crumb.kind(), crumb.id()).map_err(|_| invalid());
    let rest = rest.iter().map(segment).collect::<Result<Vec<_>, _>>()?;

    ResourcePath::new(resource.product(), segment(root)?, rest).map_err(|_| invalid())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::test_support::block_on;

    struct StubProvider {
        fail: bool,
    }

    #[async_trait]
    impl ResourceProvider for StubProvider {
        async fn validate_ref(&self, resource: &ResourceRef) -> Result<bool, CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("backend unreachable"));
            }

            Ok(resource.id() == "42")
        }

        async fn path_of(
            &self,
            resource: &ResourceRef,
        ) -> Result<Vec<ResourceRef>, CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("backend unreachable"));
            }

            Ok(vec![resource.clone()])
        }

        async fn ancestors(
            &self,
            resource: &ResourceRef,
        ) -> Result<Vec<ResourceRef>, CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("backend unreachable"));
            }

            Ok(vec![resource.clone()])
        }

        async fn members_of(
            &self,
            _set: &PrincipalSetId,
        ) -> Result<Vec<PrincipalId>, CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("backend unreachable"));
            }

            Ok(vec![PrincipalId::new("u_1").expect("valid principal id")])
        }

        async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("backend unreachable"));
            }

            Ok(ProviderCatalog {
                resource_kinds: vec!["document".to_string()],
                actions: vec!["acta::document::read".parse().expect("valid action id")],
                role_definitions: vec!["owner".to_string()],
                principal_sets: vec!["acta.members".to_string()],
                role_definitions_v2: vec![RoleDefinition {
                    name: "owner".to_string(),
                    version: 1,
                    actions: vec!["acta::document::read".parse().expect("valid action id")],
                }],
            })
        }
    }

    #[test]
    fn resource_provider_is_object_safe() {
        let _: Option<Box<dyn ResourceProvider>> = None;
    }

    #[test]
    fn provider_failure_maps_to_unavailable() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: true });
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");

        let error = block_on(provider.validate_ref(&resource)).unwrap_err();

        assert_eq!(error, CapabilityError::unavailable("backend unreachable"));
    }

    #[test]
    fn validate_ref_distinguishes_no_from_could_not_answer() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: false });
        let missing: ResourceRef = "acta::document::99".parse().expect("valid resource ref");

        let result = block_on(provider.validate_ref(&missing)).expect("provider answers");

        assert!(!result);
    }

    #[test]
    fn members_of_resolves_a_principal_set() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: false });
        let set: PrincipalSetId = "acta::workspace::w_01::members"
            .parse()
            .expect("valid set id");

        let members = block_on(provider.members_of(&set)).expect("members resolve");

        assert_eq!(
            members,
            vec![PrincipalId::new("u_1").expect("valid principal id")]
        );
    }

    #[test]
    fn catalog_exposes_provider_capabilities() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: false });

        let catalog = block_on(provider.catalog()).expect("catalog resolves");

        assert_eq!(catalog.principal_sets, vec!["acta.members".to_string()]);
        assert_eq!(catalog.resource_kinds, vec!["document".to_string()]);
    }

    /// A legacy provider that only implements the per-resource methods:
    /// every resource exists except `missing` ids, under `breadcrumbs`.
    struct LegacyProvider {
        breadcrumbs: Vec<ResourceRef>,
    }

    #[async_trait]
    impl ResourceProvider for LegacyProvider {
        async fn validate_ref(&self, resource: &ResourceRef) -> Result<bool, CapabilityError> {
            Ok(resource.id() != "missing")
        }

        async fn path_of(
            &self,
            _resource: &ResourceRef,
        ) -> Result<Vec<ResourceRef>, CapabilityError> {
            Ok(self.breadcrumbs.clone())
        }

        async fn ancestors(
            &self,
            _resource: &ResourceRef,
        ) -> Result<Vec<ResourceRef>, CapabilityError> {
            Ok(Vec::new())
        }

        async fn members_of(
            &self,
            _set: &PrincipalSetId,
        ) -> Result<Vec<PrincipalId>, CapabilityError> {
            Ok(Vec::new())
        }

        async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError> {
            Err(CapabilityError::unavailable("not needed"))
        }
    }

    fn resource(raw: &str) -> ResourceRef {
        raw.parse().expect("valid resource ref")
    }

    #[test]
    fn default_resource_facts_answers_every_resource_in_input_order() {
        let provider: Box<dyn ResourceProvider> = Box::new(LegacyProvider {
            breadcrumbs: vec![
                resource("acta::workspace::w1"),
                resource("acta::document::d1"),
            ],
        });
        let resources = [
            resource("acta::document::d1"),
            resource("acta::document::missing"),
        ];

        let facts = block_on(provider.resource_facts(&resources)).expect("facts resolve");

        assert_eq!(
            facts,
            vec![
                ResourceFacts {
                    resource: resource("acta::document::d1"),
                    existence: ResourceExistence::Exists,
                    path: Some(
                        "acta::workspace::w1/document::d1"
                            .parse()
                            .expect("valid path")
                    ),
                },
                ResourceFacts {
                    resource: resource("acta::document::missing"),
                    existence: ResourceExistence::Missing,
                    path: None,
                },
            ]
        );
    }

    #[test]
    fn default_resource_facts_propagates_an_unavailable_backend() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: true });

        let error = block_on(provider.resource_facts(&[resource("acta::document::42")]))
            .expect_err("backend is down");

        assert_eq!(error, CapabilityError::unavailable("backend unreachable"));
    }

    #[test]
    fn default_resource_facts_rejects_breadcrumbs_that_do_not_end_at_the_resource() {
        for breadcrumbs in [
            Vec::new(),
            vec![resource("acta::workspace::w1")],
            vec![
                resource("custos::platform::atlas"),
                resource("acta::document::d1"),
            ],
        ] {
            let provider: Box<dyn ResourceProvider> = Box::new(LegacyProvider { breadcrumbs });

            let error = block_on(provider.resource_facts(&[resource("acta::document::d1")]))
                .expect_err("breadcrumbs are inconsistent");

            assert!(
                matches!(error, CapabilityError::Invalid { .. }),
                "{error:?}"
            );
        }
    }

    #[test]
    fn catalog_carries_versioned_role_definitions_beside_the_plain_names() {
        let provider: Box<dyn ResourceProvider> = Box::new(StubProvider { fail: false });

        let catalog = block_on(provider.catalog()).expect("catalog resolves");

        assert_eq!(catalog.role_definitions, vec!["owner".to_string()]);
        assert_eq!(catalog.role_definitions_v2.len(), 1);
        assert_eq!(catalog.role_definitions_v2[0].version, 1);
    }
}
