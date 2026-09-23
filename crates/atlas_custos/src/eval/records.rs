//! Conversions between the stored authorization records
//! (`entities::authorization`) and the evaluator model. The storage enums
//! keep their own shapes because their text forms are the persisted
//! contract; these conversions are the only bridge, so routes and services
//! never map records by hand.

use crate::entities::authorization::{
    CustomRole as StoredCustomRole, DenyRecord, GrantAuthority, GrantRecord, SubjectRecord,
    TargetRecord,
};
use crate::eval::EvalError;
use crate::eval::catalog::{Catalog, CatalogError, GrantSpec, RoleRef, mixed_products};
use crate::eval::model::{ActionSet, DenyRule, GrantTarget, Subject};

impl From<&SubjectRecord> for Subject {
    fn from(record: &SubjectRecord) -> Self {
        match record {
            SubjectRecord::Principal(principal) => Subject::Principal(*principal),
            SubjectRecord::Group(group) => Subject::Group(*group),
            SubjectRecord::PrincipalSet(set) => Subject::PrincipalSet(set.clone()),
        }
    }
}

impl From<&Subject> for SubjectRecord {
    fn from(subject: &Subject) -> Self {
        match subject {
            Subject::Principal(principal) => SubjectRecord::Principal(*principal),
            Subject::Group(group) => SubjectRecord::Group(*group),
            Subject::PrincipalSet(set) => SubjectRecord::PrincipalSet(set.clone()),
        }
    }
}

impl From<&TargetRecord> for GrantTarget {
    fn from(record: &TargetRecord) -> Self {
        match record {
            TargetRecord::Ref(reference) => GrantTarget::Ref(reference.clone()),
            TargetRecord::Path(path) => GrantTarget::Path(path.clone()),
            TargetRecord::Selector(selector) => GrantTarget::Selector(selector.clone()),
        }
    }
}

impl From<&GrantTarget> for TargetRecord {
    fn from(target: &GrantTarget) -> Self {
        match target {
            GrantTarget::Ref(reference) => TargetRecord::Ref(reference.clone()),
            GrantTarget::Path(path) => TargetRecord::Path(path.clone()),
            GrantTarget::Selector(selector) => TargetRecord::Selector(selector.clone()),
        }
    }
}

/// Converts a stored grant into a [`GrantSpec`] for
/// [`Catalog::resolve_grant`].
///
/// A built-in authority names the role of the grant target's product, since
/// a stored grant's product is always its target's. A custom-role authority
/// needs its stored row among `custom_roles`; the row's actions are
/// revalidated through `catalog`, so a role that no longer satisfies the
/// catalog is rejected, and the row's `product` must be its actions'
/// product. An explicit action list becomes an action set.
pub fn grant_spec(
    record: &GrantRecord,
    catalog: &Catalog,
    custom_roles: &[StoredCustomRole],
) -> Result<GrantSpec, CatalogError> {
    let mut spec = GrantSpec {
        target: Some(GrantTarget::from(&record.target)),
        ..GrantSpec::default()
    };

    match Subject::from(&record.subject) {
        Subject::Principal(principal) => spec.principal = Some(principal),
        Subject::Group(group) => spec.group = Some(group),
        Subject::PrincipalSet(set) => spec.principal_set = Some(set),
    }

    match &record.authority {
        GrantAuthority::Builtin { name, version } => {
            spec.builtin_role = Some(RoleRef {
                product: record.target.product().to_string(),
                name: name.clone(),
                version: *version,
            });
        }
        GrantAuthority::CustomRole(id) => {
            let Some(role) = custom_roles.iter().find(|role| role.id == *id) else {
                return Err(CatalogError::UnknownCustomRole { id: *id });
            };

            let custom = catalog.custom_role(role.actions.iter().cloned())?;
            let actions_product = custom.actions().product().unwrap_or_default();

            if actions_product != role.product {
                return Err(CatalogError::CustomRoleProductMismatch {
                    id: role.id,
                    stored_product: role.product.clone(),
                    actions_product: actions_product.to_string(),
                });
            }

            spec.custom_role = Some(custom);
        }
        GrantAuthority::Actions(actions) => {
            spec.actions = Some(ActionSet::new(actions.iter().cloned()).map_err(mixed_products)?);
        }
    }

    Ok(spec)
}

impl TryFrom<&DenyRecord> for DenyRule {
    type Error = EvalError;

    /// Builds the evaluator deny rule through [`DenyRule::new`], so a stored
    /// row whose actions do not fit its target is rejected.
    fn try_from(record: &DenyRecord) -> Result<Self, Self::Error> {
        let actions = ActionSet::new(record.actions.iter().cloned())?;

        DenyRule::new(
            Subject::from(&record.subject),
            GrantTarget::from(&record.target),
            actions,
        )
    }
}
