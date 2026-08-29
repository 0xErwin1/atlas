use async_trait::async_trait;

use crate::ids::action_id::ActionId;
use crate::ids::principal_id::PrincipalId;
use crate::ids::principal_set_id::PrincipalSetId;
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
    async fn members_of(&self, set: &PrincipalSetId) -> Result<Vec<PrincipalId>, CapabilityError>;

    /// Describes this provider's supported resource kinds, actions, role
    /// definitions, and principal set names.
    async fn catalog(&self) -> Result<ProviderCatalog, CapabilityError>;
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
}
