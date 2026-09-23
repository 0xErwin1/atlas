//! The pure authorization catalog and grant validation (GRANT-1/2/5).
//!
//! A [`Catalog`] is built from plain per-product data: resource kinds,
//! actions, versioned built-in roles and declared principal set names. It
//! validates grant targets and product-scoped custom roles, and resolves a
//! [`GrantSpec`] into the evaluator's [`Grant`]. Selector syntax is already
//! enforced by the `atlas_core` parser, so a parsed selector only needs its
//! product and literal kinds checked here.

use std::collections::{HashMap, HashSet};
use std::fmt;

use crate::entities::authorization::RoleId;
use crate::eval::EvalError;
use crate::eval::model::{
    ActionSet, CUSTOS_PRODUCT, Grant, GrantTarget, Subject, is_delegation_action,
};
use crate::ids::{GroupId, PrincipalId};
use atlas_core::ids::{ActionId, PrincipalSetId, SelectorSegment};

/// The plain catalog data one product declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductSpec {
    pub product: String,
    pub kinds: Vec<String>,
    pub actions: Vec<ActionId>,
    pub roles: Vec<RoleSpec>,
    /// Declared principal set names (for example `members`), not instances.
    pub principal_sets: Vec<String>,
}

/// The plain data of one versioned built-in role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleSpec {
    pub name: String,
    pub version: u32,
    pub actions: Vec<ActionId>,
}

/// A validated, versioned built-in role of one product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuiltinRole {
    product: String,
    name: String,
    version: u32,
    actions: ActionSet,
}

impl BuiltinRole {
    pub fn product(&self) -> &str {
        &self.product
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn version(&self) -> u32 {
        self.version
    }

    pub fn actions(&self) -> &ActionSet {
        &self.actions
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProductEntry {
    kinds: HashSet<String>,
    actions: HashSet<ActionId>,
    roles: Vec<BuiltinRole>,
    principal_sets: HashSet<String>,
}

/// The validated catalog of every product's authorization vocabulary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Catalog {
    products: HashMap<String, ProductEntry>,
}

/// A reference to one version of a product's built-in role.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RoleRef {
    pub product: String,
    pub name: String,
    pub version: u32,
}

/// A custom role validated by [`Catalog::custom_role`]: non-empty, one
/// product, catalog actions only and never a Custos action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRole {
    actions: ActionSet,
}

impl CustomRole {
    pub fn actions(&self) -> &ActionSet {
        &self.actions
    }
}

/// An unvalidated grant request. A valid spec names exactly one subject,
/// one target and exactly one authority: a built-in role version, a custom
/// role or an explicit action set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSpec {
    pub principal: Option<PrincipalId>,
    pub group: Option<GroupId>,
    pub principal_set: Option<PrincipalSetId>,
    pub target: Option<GrantTarget>,
    pub builtin_role: Option<RoleRef>,
    pub custom_role: Option<CustomRole>,
    pub actions: Option<ActionSet>,
}

/// Why catalog data, a target, a custom role or a grant spec is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// The plain catalog data contradicts itself.
    InvalidCatalog {
        detail: String,
    },
    UnknownProduct {
        product: String,
    },
    UnknownKind {
        product: String,
        kind: String,
    },
    UnknownAction {
        action: ActionId,
    },
    /// A custom role or explicit action set has no action.
    EmptyActions,
    MixedProducts {
        first: String,
        second: String,
    },
    CustosActionInCustomRole {
        action: ActionId,
    },
    /// A grant spec must name exactly one subject.
    SubjectCount {
        found: usize,
    },
    MissingTarget,
    /// A grant spec must name exactly one authority.
    AuthorityCount {
        found: usize,
    },
    UnknownRole {
        product: String,
        name: String,
        version: u32,
    },
    ProductMismatch {
        target_product: String,
        authority_product: String,
    },
    UndeclaredPrincipalSet {
        set: PrincipalSetId,
    },
    /// A stored grant references a custom role whose row was not supplied.
    UnknownCustomRole {
        id: RoleId,
    },
    /// A stored custom role's `product` differs from the product of its
    /// actions.
    CustomRoleProductMismatch {
        id: RoleId,
        stored_product: String,
        actions_product: String,
    },
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidCatalog { detail } => write!(f, "invalid catalog: {detail}"),
            Self::UnknownProduct { product } => write!(f, "unknown product `{product}`"),
            Self::UnknownKind { product, kind } => {
                write!(f, "unknown resource kind `{kind}` in product `{product}`")
            }
            Self::UnknownAction { action } => write!(f, "unknown action `{action}`"),
            Self::EmptyActions => write!(f, "an action set must not be empty"),
            Self::MixedProducts { first, second } => {
                write!(f, "actions mix products `{first}` and `{second}`")
            }
            Self::CustosActionInCustomRole { action } => {
                write!(
                    f,
                    "custom roles cannot contain the Custos action `{action}`"
                )
            }
            Self::SubjectCount { found } => {
                write!(f, "a grant needs exactly one subject, found {found}")
            }
            Self::MissingTarget => write!(f, "a grant needs a target"),
            Self::AuthorityCount { found } => write!(
                f,
                "a grant needs exactly one of a built-in role, a custom role or actions, found {found}"
            ),
            Self::UnknownRole {
                product,
                name,
                version,
            } => write!(f, "unknown built-in role `{product}::{name}@{version}`"),
            Self::ProductMismatch {
                target_product,
                authority_product,
            } => write!(
                f,
                "authority product `{authority_product}` does not match target product `{target_product}`"
            ),
            Self::UndeclaredPrincipalSet { set } => {
                write!(f, "principal set `{set}` is not declared")
            }
            Self::UnknownCustomRole { id } => write!(f, "custom role `{id}` was not supplied"),
            Self::CustomRoleProductMismatch {
                id,
                stored_product,
                actions_product,
            } => write!(
                f,
                "custom role `{id}` is stored for product `{stored_product}` but its actions belong to `{actions_product}`"
            ),
        }
    }
}

impl std::error::Error for CatalogError {}

/// The single authority a grant spec names.
enum Authority {
    Builtin(RoleRef),
    Custom(CustomRole),
    Explicit(ActionSet),
}

impl Catalog {
    /// Builds a catalog from plain product data, rejecting duplicate
    /// products, actions of another product or of an undeclared kind, and
    /// built-in roles that are empty, duplicated by name and version, or use
    /// undeclared actions. Built-in roles may also carry Custos delegation
    /// actions, which need no declaration.
    pub fn new(products: impl IntoIterator<Item = ProductSpec>) -> Result<Self, CatalogError> {
        let mut entries = HashMap::new();

        for spec in products {
            let product = spec.product.clone();
            let entry = product_entry(spec)?;

            if entries.insert(product.clone(), entry).is_some() {
                return Err(invalid(format!("product `{product}` is declared twice")));
            }
        }

        Ok(Self { products: entries })
    }

    /// The built-in role `name` at `version` of `product`, if declared.
    pub fn builtin_role(&self, product: &str, name: &str, version: u32) -> Option<&BuiltinRole> {
        self.products
            .get(product)?
            .roles
            .iter()
            .find(|role| role.name == name && role.version == version)
    }

    /// Checks that a grant or deny target names a declared product and only
    /// declared kinds. Selector wildcards carry no kind.
    pub fn validate_target(&self, target: &GrantTarget) -> Result<(), CatalogError> {
        let product = target.product();
        let entry = self.product(product)?;

        let kinds: Vec<&str> = match target {
            GrantTarget::Ref(reference) => vec![reference.kind()],
            GrantTarget::Path(path) => path.segments().map(|segment| segment.kind()).collect(),
            GrantTarget::Selector(selector) => selector
                .segments()
                .iter()
                .filter_map(|segment| match segment {
                    SelectorSegment::Literal(literal) => Some(literal.kind()),
                    SelectorSegment::Any => None,
                })
                .collect(),
        };

        match kinds.into_iter().find(|kind| !entry.kinds.contains(*kind)) {
            Some(kind) => Err(CatalogError::UnknownKind {
                product: product.to_string(),
                kind: kind.to_string(),
            }),
            None => Ok(()),
        }
    }

    /// Validates a custom role: non-empty, one product, never a Custos
    /// action, and every action declared in the catalog.
    pub fn custom_role(
        &self,
        actions: impl IntoIterator<Item = ActionId>,
    ) -> Result<CustomRole, CatalogError> {
        let actions = ActionSet::new(actions).map_err(mixed_products)?;

        if let Some(action) = actions
            .iter()
            .find(|action| action.product() == CUSTOS_PRODUCT)
        {
            return Err(CatalogError::CustosActionInCustomRole {
                action: action.clone(),
            });
        }

        self.validate_actions(&actions)?;

        Ok(CustomRole { actions })
    }

    /// Resolves a grant spec into an evaluator [`Grant`], rejecting every
    /// shape, catalog or product violation with a typed error.
    pub fn resolve_grant(&self, spec: GrantSpec) -> Result<Grant, CatalogError> {
        let GrantSpec {
            principal,
            group,
            principal_set,
            target,
            builtin_role,
            custom_role,
            actions,
        } = spec;

        let subject = single_subject(principal, group, principal_set)?;

        let Some(target) = target else {
            return Err(CatalogError::MissingTarget);
        };
        self.validate_target(&target)?;

        if let Subject::PrincipalSet(set) = &subject {
            self.validate_principal_set(set)?;
        }

        let authority = single_authority(builtin_role, custom_role, actions)?;
        let actions = self.authority_actions(authority)?;

        let target_product = target.product().to_string();
        let authority_product = actions.product().unwrap_or_default().to_string();

        Grant::new(subject, target, actions).map_err(|_| CatalogError::ProductMismatch {
            target_product,
            authority_product,
        })
    }

    fn product(&self, product: &str) -> Result<&ProductEntry, CatalogError> {
        self.products
            .get(product)
            .ok_or_else(|| CatalogError::UnknownProduct {
                product: product.to_string(),
            })
    }

    /// Checks a non-empty action set against the catalog. Delegation
    /// actions form a fixed Custos list and need no product declaration.
    fn validate_actions(&self, actions: &ActionSet) -> Result<(), CatalogError> {
        if actions.is_empty() {
            return Err(CatalogError::EmptyActions);
        }

        let unknown = actions.iter().find(|action| {
            !is_delegation_action(action)
                && self
                    .products
                    .get(action.product())
                    .is_none_or(|entry| !entry.actions.contains(*action))
        });

        match unknown {
            Some(action) => Err(CatalogError::UnknownAction {
                action: action.clone(),
            }),
            None => Ok(()),
        }
    }

    fn validate_principal_set(&self, set: &PrincipalSetId) -> Result<(), CatalogError> {
        let declared = self
            .products
            .get(set.scope().product())
            .is_some_and(|entry| {
                entry.kinds.contains(set.scope().kind()) && entry.principal_sets.contains(set.set())
            });

        if declared {
            Ok(())
        } else {
            Err(CatalogError::UndeclaredPrincipalSet { set: set.clone() })
        }
    }

    fn authority_actions(&self, authority: Authority) -> Result<ActionSet, CatalogError> {
        match authority {
            Authority::Builtin(role) => self
                .builtin_role(&role.product, &role.name, role.version)
                .map(|builtin| builtin.actions.clone())
                .ok_or(CatalogError::UnknownRole {
                    product: role.product,
                    name: role.name,
                    version: role.version,
                }),
            Authority::Custom(role) => {
                self.validate_actions(&role.actions)?;
                Ok(role.actions)
            }
            Authority::Explicit(actions) => {
                self.validate_actions(&actions)?;
                Ok(actions)
            }
        }
    }
}

fn single_subject(
    principal: Option<PrincipalId>,
    group: Option<GroupId>,
    principal_set: Option<PrincipalSetId>,
) -> Result<Subject, CatalogError> {
    let subjects: Vec<Subject> = principal
        .map(Subject::Principal)
        .into_iter()
        .chain(group.map(Subject::Group))
        .chain(principal_set.map(Subject::PrincipalSet))
        .collect();

    let [subject] =
        <[Subject; 1]>::try_from(subjects).map_err(|subjects| CatalogError::SubjectCount {
            found: subjects.len(),
        })?;

    Ok(subject)
}

fn single_authority(
    builtin_role: Option<RoleRef>,
    custom_role: Option<CustomRole>,
    actions: Option<ActionSet>,
) -> Result<Authority, CatalogError> {
    let authorities: Vec<Authority> = builtin_role
        .map(Authority::Builtin)
        .into_iter()
        .chain(custom_role.map(Authority::Custom))
        .chain(actions.map(Authority::Explicit))
        .collect();

    let [authority] = <[Authority; 1]>::try_from(authorities).map_err(|authorities| {
        CatalogError::AuthorityCount {
            found: authorities.len(),
        }
    })?;

    Ok(authority)
}

/// Validates one product's plain data into its catalog entry.
fn product_entry(spec: ProductSpec) -> Result<ProductEntry, CatalogError> {
    let kinds: HashSet<String> = spec.kinds.into_iter().collect();

    if let Some(action) = spec
        .actions
        .iter()
        .find(|action| action.product() != spec.product || !kinds.contains(action.kind()))
    {
        return Err(invalid(format!(
            "action `{action}` is not of a declared kind of product `{}`",
            spec.product
        )));
    }

    let actions: HashSet<ActionId> = spec.actions.into_iter().collect();
    let mut roles: Vec<BuiltinRole> = Vec::with_capacity(spec.roles.len());

    for role in spec.roles {
        let builtin = builtin_role(&spec.product, &actions, role)?;

        if roles
            .iter()
            .any(|existing| existing.name == builtin.name && existing.version == builtin.version)
        {
            return Err(invalid(format!(
                "built-in role `{}@{}` is declared twice",
                builtin.name, builtin.version
            )));
        }

        roles.push(builtin);
    }

    Ok(ProductEntry {
        kinds,
        actions,
        roles,
        principal_sets: spec.principal_sets.into_iter().collect(),
    })
}

fn builtin_role(
    product: &str,
    declared: &HashSet<ActionId>,
    role: RoleSpec,
) -> Result<BuiltinRole, CatalogError> {
    if role.actions.is_empty() {
        return Err(invalid(format!("built-in role `{}` is empty", role.name)));
    }

    if let Some(action) = role
        .actions
        .iter()
        .find(|action| !declared.contains(*action) && !is_delegation_action(action))
    {
        return Err(invalid(format!(
            "built-in role `{}` uses undeclared action `{action}`",
            role.name
        )));
    }

    let actions = ActionSet::new(role.actions).map_err(mixed_products)?;

    Ok(BuiltinRole {
        product: product.to_string(),
        name: role.name,
        version: role.version,
        actions,
    })
}

fn invalid(detail: String) -> CatalogError {
    CatalogError::InvalidCatalog { detail }
}

/// Maps the action-set construction error, which only reports mixed
/// products, into the catalog vocabulary.
pub(super) fn mixed_products(error: EvalError) -> CatalogError {
    match error {
        EvalError::CrossProductActions { first, second } => {
            CatalogError::MixedProducts { first, second }
        }
        other => invalid(other.to_string()),
    }
}
