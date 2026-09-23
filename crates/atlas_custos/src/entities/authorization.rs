//! The V2 authorization records persisted in `custos.roles`,
//! `custos.grants_v2` and `custos.deny_rules`: custom roles, grants and deny
//! rules. These are storage shapes, not evaluation facts: a grant's authority
//! is stored as written (a built-in `name@version`, a custom role id, or an
//! explicit action list) and resolved to concrete actions by a later slice.
//! Targets keep the core vocabulary (`ResourceRef`/`ResourcePath`/
//! `ResourceSelector`) and are stored as their canonical text form.

use crate::ids::{GroupId, PrincipalId};
use atlas_core::define_id;
use atlas_core::ids::{ActionId, PrincipalSetId, ResourcePath, ResourceRef, ResourceSelector};
use chrono::{DateTime, Utc};

define_id!(RoleId);
define_id!(GrantId);
define_id!(DenyRuleId);

/// Component-conflict code raised when a custom role is deleted while a
/// grant still references it. Surfaces as
/// `DomainError::ComponentConflict { code: ROLE_IN_USE_CONFLICT, .. }`.
pub const ROLE_IN_USE_CONFLICT: &str = "role-in-use";

/// The subject a stored grant or deny rule is addressed to. Mirrors the
/// evaluator's subject vocabulary: exactly one of a principal, a group or a
/// named principal set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubjectRecord {
    Principal(PrincipalId),
    Group(GroupId),
    PrincipalSet(PrincipalSetId),
}

/// The stored `subject_kind` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubjectKind {
    Principal,
    Group,
    PrincipalSet,
}

impl SubjectKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SubjectKind::Principal => "principal",
            SubjectKind::Group => "group",
            SubjectKind::PrincipalSet => "principal_set",
        }
    }
}

impl std::str::FromStr for SubjectKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "principal" => Ok(SubjectKind::Principal),
            "group" => Ok(SubjectKind::Group),
            "principal_set" => Ok(SubjectKind::PrincipalSet),
            other => Err(format!("unknown subject kind: {other}")),
        }
    }
}

impl SubjectRecord {
    pub fn kind(&self) -> SubjectKind {
        match self {
            SubjectRecord::Principal(_) => SubjectKind::Principal,
            SubjectRecord::Group(_) => SubjectKind::Group,
            SubjectRecord::PrincipalSet(_) => SubjectKind::PrincipalSet,
        }
    }
}

/// The resource side of a stored grant or deny rule, carrying the parsed
/// core target so the canonical text written to storage is always the one
/// the core type prints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetRecord {
    Ref(ResourceRef),
    Path(ResourcePath),
    Selector(ResourceSelector),
}

/// The stored `target_kind` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetKind {
    Ref,
    Path,
    Selector,
}

impl TargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            TargetKind::Ref => "ref",
            TargetKind::Path => "path",
            TargetKind::Selector => "selector",
        }
    }
}

impl std::str::FromStr for TargetKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "ref" => Ok(TargetKind::Ref),
            "path" => Ok(TargetKind::Path),
            "selector" => Ok(TargetKind::Selector),
            other => Err(format!("unknown target kind: {other}")),
        }
    }
}

impl TargetRecord {
    pub fn kind(&self) -> TargetKind {
        match self {
            TargetRecord::Ref(_) => TargetKind::Ref,
            TargetRecord::Path(_) => TargetKind::Path,
            TargetRecord::Selector(_) => TargetKind::Selector,
        }
    }

    pub fn product(&self) -> &str {
        match self {
            TargetRecord::Ref(reference) => reference.product(),
            TargetRecord::Path(path) => path.product(),
            TargetRecord::Selector(selector) => selector.product(),
        }
    }

    /// The canonical text form stored in the `target` column.
    pub fn canonical(&self) -> String {
        match self {
            TargetRecord::Ref(reference) => reference.to_string(),
            TargetRecord::Path(path) => path.to_string(),
            TargetRecord::Selector(selector) => selector.to_string(),
        }
    }

    /// Rebuilds a target from its stored `target_kind` and canonical text.
    pub fn parse(kind: TargetKind, canonical: &str) -> Result<Self, String> {
        match kind {
            TargetKind::Ref => canonical
                .parse()
                .map(TargetRecord::Ref)
                .map_err(|e| format!("invalid ref target: {e}")),
            TargetKind::Path => canonical
                .parse()
                .map(TargetRecord::Path)
                .map_err(|e| format!("invalid path target: {e}")),
            TargetKind::Selector => canonical
                .parse()
                .map(TargetRecord::Selector)
                .map_err(|e| format!("invalid selector target: {e}")),
        }
    }
}

/// What a grant confers: a built-in role from a product catalog referenced
/// by `name@version`, a custom role row, or an explicit action list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantAuthority {
    Builtin { name: String, version: u32 },
    CustomRole(RoleId),
    Actions(Vec<ActionId>),
}

/// The stored `authority_kind` discriminator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorityKind {
    Builtin,
    Custom,
    Actions,
}

impl AuthorityKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            AuthorityKind::Builtin => "builtin",
            AuthorityKind::Custom => "custom",
            AuthorityKind::Actions => "actions",
        }
    }
}

impl std::str::FromStr for AuthorityKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "builtin" => Ok(AuthorityKind::Builtin),
            "custom" => Ok(AuthorityKind::Custom),
            "actions" => Ok(AuthorityKind::Actions),
            other => Err(format!("unknown authority kind: {other}")),
        }
    }
}

impl GrantAuthority {
    pub fn kind(&self) -> AuthorityKind {
        match self {
            GrantAuthority::Builtin { .. } => AuthorityKind::Builtin,
            GrantAuthority::CustomRole(_) => AuthorityKind::Custom,
            GrantAuthority::Actions(_) => AuthorityKind::Actions,
        }
    }
}

/// A product-scoped custom role: a named, non-empty action list that grants
/// can reference by id. `(product, name)` is unique.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CustomRole {
    pub id: RoleId,
    pub product: String,
    pub name: String,
    pub actions: Vec<ActionId>,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewCustomRole {
    pub id: RoleId,
    pub product: String,
    pub name: String,
    pub actions: Vec<ActionId>,
    pub created_by: PrincipalId,
}

/// A stored V2 grant: one subject, one target, one authority. The row's
/// `product` column is always the target's product.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantRecord {
    pub id: GrantId,
    pub subject: SubjectRecord,
    pub target: TargetRecord,
    pub authority: GrantAuthority,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewGrantRecord {
    pub id: GrantId,
    pub subject: SubjectRecord,
    pub target: TargetRecord,
    pub authority: GrantAuthority,
    pub created_by: PrincipalId,
}

/// A stored deny rule: one subject, one target, one non-empty action list.
/// Rows persist regardless of the configured deny mode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DenyRecord {
    pub id: DenyRuleId,
    pub subject: SubjectRecord,
    pub target: TargetRecord,
    pub actions: Vec<ActionId>,
    pub created_by: PrincipalId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDenyRecord {
    pub id: DenyRuleId,
    pub subject: SubjectRecord,
    pub target: TargetRecord,
    pub actions: Vec<ActionId>,
    pub created_by: PrincipalId,
}

/// The subject identities one acting principal resolves to: itself, the
/// groups it belongs to, and the principal sets it is a member of. The
/// evaluation load returns every grant or deny row addressed to any of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubjectSet {
    pub principal: PrincipalId,
    pub groups: Vec<GroupId>,
    pub principal_sets: Vec<PrincipalSetId>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_kind_round_trips_through_its_stored_string() {
        let kinds = [
            (SubjectKind::Principal, "principal"),
            (SubjectKind::Group, "group"),
            (SubjectKind::PrincipalSet, "principal_set"),
        ];

        for (kind, stored) in kinds {
            assert_eq!(kind.as_str(), stored);
            assert_eq!(stored.parse::<SubjectKind>().unwrap(), kind);
        }

        assert!("user".parse::<SubjectKind>().is_err());
    }

    #[test]
    fn subject_record_reports_the_kind_of_its_variant() {
        let set: PrincipalSetId = "acta::workspace::w1::members".parse().unwrap();

        assert_eq!(
            SubjectRecord::Principal(PrincipalId::new()).kind(),
            SubjectKind::Principal
        );
        assert_eq!(
            SubjectRecord::Group(GroupId::new()).kind(),
            SubjectKind::Group
        );
        assert_eq!(
            SubjectRecord::PrincipalSet(set).kind(),
            SubjectKind::PrincipalSet
        );
    }

    #[test]
    fn target_kind_round_trips_through_its_stored_string() {
        let kinds = [
            (TargetKind::Ref, "ref"),
            (TargetKind::Path, "path"),
            (TargetKind::Selector, "selector"),
        ];

        for (kind, stored) in kinds {
            assert_eq!(kind.as_str(), stored);
            assert_eq!(stored.parse::<TargetKind>().unwrap(), kind);
        }

        assert!("glob".parse::<TargetKind>().is_err());
    }

    #[test]
    fn target_record_round_trips_every_shape_through_its_canonical_text() {
        let cases = [
            (
                TargetRecord::Ref("acta::document::d1".parse().unwrap()),
                TargetKind::Ref,
                "acta::document::d1",
            ),
            (
                TargetRecord::Path(
                    "acta::workspace::w1/project::p1/document::d1"
                        .parse()
                        .unwrap(),
                ),
                TargetKind::Path,
                "acta::workspace::w1/project::p1/document::d1",
            ),
            (
                TargetRecord::Selector("acta::workspace::w1/project::p1/**".parse().unwrap()),
                TargetKind::Selector,
                "acta::workspace::w1/project::p1/**",
            ),
        ];

        for (target, kind, canonical) in cases {
            assert_eq!(target.kind(), kind);
            assert_eq!(target.product(), "acta");
            assert_eq!(target.canonical(), canonical);
            assert_eq!(TargetRecord::parse(kind, canonical).unwrap(), target);
        }
    }

    #[test]
    fn target_record_parse_rejects_text_that_does_not_match_its_kind() {
        assert!(TargetRecord::parse(TargetKind::Ref, "acta::workspace::w1/**").is_err());
        assert!(TargetRecord::parse(TargetKind::Path, "").is_err());
        assert!(TargetRecord::parse(TargetKind::Selector, "not a selector").is_err());
    }

    #[test]
    fn authority_kind_round_trips_through_its_stored_string() {
        let kinds = [
            (AuthorityKind::Builtin, "builtin"),
            (AuthorityKind::Custom, "custom"),
            (AuthorityKind::Actions, "actions"),
        ];

        for (kind, stored) in kinds {
            assert_eq!(kind.as_str(), stored);
            assert_eq!(stored.parse::<AuthorityKind>().unwrap(), kind);
        }

        assert!("role".parse::<AuthorityKind>().is_err());
    }

    #[test]
    fn grant_authority_reports_the_kind_of_its_variant() {
        assert_eq!(
            GrantAuthority::Builtin {
                name: "editor".to_string(),
                version: 1,
            }
            .kind(),
            AuthorityKind::Builtin
        );
        assert_eq!(
            GrantAuthority::CustomRole(RoleId::new()).kind(),
            AuthorityKind::Custom
        );
        assert_eq!(
            GrantAuthority::Actions(vec!["acta::document::read".parse().unwrap()]).kind(),
            AuthorityKind::Actions
        );
    }
}
