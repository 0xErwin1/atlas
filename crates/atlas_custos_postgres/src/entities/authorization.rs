//! SeaORM entities for the V2 authorization tables (`custos.roles`,
//! `custos.grants_v2`, `custos.deny_rules`) and their mapping onto the pure
//! `atlas_custos::entities::authorization` records. A stored discriminator or
//! canonical text that no longer parses is a data-integrity fault and
//! surfaces as an error, never as a defaulted value.

use atlas_core::error::DomainError;
use atlas_core::ids::ActionId;
use atlas_custos::entities::authorization::{
    AuthorityKind, CustomRole, DenyRecord, DenyRuleId, GrantAuthority, GrantId, GrantRecord,
    RoleId, SubjectKind, SubjectRecord, TargetKind, TargetRecord,
};
use atlas_custos::ids::{GroupId, PrincipalId};
use chrono::{DateTime, Utc};
use sea_orm::entity::prelude::*;

pub mod role {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "roles")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub product: String,
        pub name: String,
        pub actions: Vec<String>,
        pub created_by: Uuid,
        pub created_at: DateTime<Utc>,
        pub updated_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod grant_v2 {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "grants_v2")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub subject_kind: String,
        pub subject_principal_id: Option<Uuid>,
        pub subject_group_id: Option<Uuid>,
        pub subject_principal_set: Option<String>,
        pub target_kind: String,
        pub target: String,
        pub product: String,
        pub authority_kind: String,
        pub role_name: Option<String>,
        pub role_version: Option<i32>,
        pub role_id: Option<Uuid>,
        pub actions: Option<Vec<String>>,
        pub created_by: Uuid,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

pub mod deny_rule {
    use super::*;

    #[derive(Clone, Debug, PartialEq, DeriveEntityModel)]
    #[sea_orm(schema_name = "custos", table_name = "deny_rules")]
    pub struct Model {
        #[sea_orm(primary_key, auto_increment = false)]
        pub id: Uuid,
        pub subject_kind: String,
        pub subject_principal_id: Option<Uuid>,
        pub subject_group_id: Option<Uuid>,
        pub subject_principal_set: Option<String>,
        pub target_kind: String,
        pub target: String,
        pub product: String,
        pub actions: Vec<String>,
        pub created_by: Uuid,
        pub created_at: DateTime<Utc>,
    }

    #[derive(Copy, Clone, Debug, EnumIter, DeriveRelation)]
    pub enum Relation {}

    impl ActiveModelBehavior for ActiveModel {}
}

/// A stored row that no longer parses is a server-side fault, never a
/// caller error: it maps to `Internal`, like the V1 adapter's unknown-role
/// path.
fn integrity_fault(message: String) -> DomainError {
    DomainError::Internal { message }
}

/// The subject columns of a grant or deny row, in table order:
/// `(subject_kind, subject_principal_id, subject_group_id, subject_principal_set)`.
pub(crate) type SubjectColumns = (&'static str, Option<Uuid>, Option<Uuid>, Option<String>);

pub(crate) fn subject_to_columns(subject: &SubjectRecord) -> SubjectColumns {
    let kind = subject.kind().as_str();

    match subject {
        SubjectRecord::Principal(id) => (kind, Some(id.0), None, None),
        SubjectRecord::Group(id) => (kind, None, Some(id.0), None),
        SubjectRecord::PrincipalSet(set) => (kind, None, None, Some(set.to_string())),
    }
}

fn subject_from(
    kind: &str,
    principal_id: Option<Uuid>,
    group_id: Option<Uuid>,
    principal_set: Option<String>,
) -> Result<SubjectRecord, DomainError> {
    let kind = kind
        .parse::<SubjectKind>()
        .map_err(|e| integrity_fault(format!("stored subject kind is invalid: {e}")))?;

    match (kind, principal_id, group_id, principal_set) {
        (SubjectKind::Principal, Some(id), None, None) => {
            Ok(SubjectRecord::Principal(PrincipalId(id)))
        }
        (SubjectKind::Group, None, Some(id), None) => Ok(SubjectRecord::Group(GroupId(id))),
        (SubjectKind::PrincipalSet, None, None, Some(set)) => set
            .parse()
            .map(SubjectRecord::PrincipalSet)
            .map_err(|e| integrity_fault(format!("stored principal set is invalid: {e}"))),
        _ => Err(integrity_fault(
            "stored subject columns do not match the subject kind".to_string(),
        )),
    }
}

fn target_from(kind: &str, canonical: &str) -> Result<TargetRecord, DomainError> {
    let kind = kind
        .parse::<TargetKind>()
        .map_err(|e| integrity_fault(format!("stored target kind is invalid: {e}")))?;

    TargetRecord::parse(kind, canonical)
        .map_err(|e| integrity_fault(format!("stored target is invalid: {e}")))
}

pub(crate) fn actions_to_stored(actions: &[ActionId]) -> Vec<String> {
    actions.iter().map(ToString::to_string).collect()
}

fn actions_from_stored(stored: Vec<String>) -> Result<Vec<ActionId>, DomainError> {
    stored
        .into_iter()
        .map(|raw| {
            raw.parse::<ActionId>()
                .map_err(|e| integrity_fault(format!("stored action id is invalid: {e}")))
        })
        .collect()
}

/// The authority columns of a grant row, in table order:
/// `(authority_kind, role_name, role_version, role_id, actions)`.
pub(crate) type AuthorityColumns = (
    &'static str,
    Option<String>,
    Option<i32>,
    Option<Uuid>,
    Option<Vec<String>>,
);

pub(crate) fn authority_to_columns(
    authority: &GrantAuthority,
) -> Result<AuthorityColumns, DomainError> {
    let kind = authority.kind().as_str();

    match authority {
        GrantAuthority::Builtin { name, version } => {
            let version = i32::try_from(*version).map_err(|_| DomainError::InvalidInput {
                message: "built-in role version exceeds the storable range".to_string(),
            })?;

            Ok((kind, Some(name.clone()), Some(version), None, None))
        }
        GrantAuthority::CustomRole(id) => Ok((kind, None, None, Some(id.0), None)),
        GrantAuthority::Actions(actions) => {
            Ok((kind, None, None, None, Some(actions_to_stored(actions))))
        }
    }
}

fn authority_from(
    kind: &str,
    role_name: Option<String>,
    role_version: Option<i32>,
    role_id: Option<Uuid>,
    actions: Option<Vec<String>>,
) -> Result<GrantAuthority, DomainError> {
    let kind = kind
        .parse::<AuthorityKind>()
        .map_err(|e| integrity_fault(format!("stored authority kind is invalid: {e}")))?;

    match (kind, role_name, role_version, role_id, actions) {
        (AuthorityKind::Builtin, Some(name), Some(version), None, None) => {
            let version = u32::try_from(version).map_err(|_| {
                integrity_fault("stored built-in role version is negative".to_string())
            })?;

            Ok(GrantAuthority::Builtin { name, version })
        }
        (AuthorityKind::Custom, None, None, Some(id), None) => {
            Ok(GrantAuthority::CustomRole(RoleId(id)))
        }
        (AuthorityKind::Actions, None, None, None, Some(actions)) => {
            actions_from_stored(actions).map(GrantAuthority::Actions)
        }
        _ => Err(integrity_fault(
            "stored authority columns do not match the authority kind".to_string(),
        )),
    }
}

pub fn role_from(m: role::Model) -> Result<CustomRole, DomainError> {
    Ok(CustomRole {
        id: RoleId(m.id),
        product: m.product,
        name: m.name,
        actions: actions_from_stored(m.actions)?,
        created_by: PrincipalId(m.created_by),
        created_at: m.created_at,
        updated_at: m.updated_at,
    })
}

pub fn grant_from(m: grant_v2::Model) -> Result<GrantRecord, DomainError> {
    Ok(GrantRecord {
        id: GrantId(m.id),
        subject: subject_from(
            &m.subject_kind,
            m.subject_principal_id,
            m.subject_group_id,
            m.subject_principal_set,
        )?,
        target: target_from(&m.target_kind, &m.target)?,
        authority: authority_from(
            &m.authority_kind,
            m.role_name,
            m.role_version,
            m.role_id,
            m.actions,
        )?,
        created_by: PrincipalId(m.created_by),
        created_at: m.created_at,
    })
}

pub fn deny_from(m: deny_rule::Model) -> Result<DenyRecord, DomainError> {
    Ok(DenyRecord {
        id: DenyRuleId(m.id),
        subject: subject_from(
            &m.subject_kind,
            m.subject_principal_id,
            m.subject_group_id,
            m.subject_principal_set,
        )?,
        target: target_from(&m.target_kind, &m.target)?,
        actions: actions_from_stored(m.actions)?,
        created_by: PrincipalId(m.created_by),
        created_at: m.created_at,
    })
}
