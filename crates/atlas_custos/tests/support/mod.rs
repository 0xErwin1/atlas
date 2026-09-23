//! Shared builders for the pure V2 evaluator integration tests. Each test
//! binary uses a different subset, so unused items are expected.
#![allow(dead_code)]

use atlas_core::ids::{ActionId, ResourcePath, ResourceRef};
use atlas_custos::eval::{
    ActionSet, Catalog, Ceiling, DenyMode, DenyRule, EvalRequest, EvaluationFacts, Existence,
    Grant, GrantTarget, MembershipFacts, ProductSpec, RoleRef, RoleSpec, Subject,
};
use atlas_custos::ids::PrincipalId;

pub(crate) const DOC: &str = "acta::document::d1";
pub(crate) const READ: &str = "acta::document::read";
pub(crate) const UPDATE: &str = "acta::document::update";
pub(crate) const DELETE: &str = "acta::document::delete";

pub(crate) fn action(raw: &str) -> ActionId {
    raw.parse().unwrap()
}

pub(crate) fn actions(raw: &[&str]) -> ActionSet {
    ActionSet::new(raw.iter().map(|raw| action(raw))).unwrap()
}

pub(crate) fn ceiling(raw: &[&str]) -> Ceiling {
    Ceiling::Restricted(raw.iter().map(|raw| action(raw)).collect())
}

pub(crate) fn ref_target(raw: &str) -> GrantTarget {
    GrantTarget::Ref(raw.parse().unwrap())
}

pub(crate) fn path_target(raw: &str) -> GrantTarget {
    GrantTarget::Path(raw.parse().unwrap())
}

pub(crate) fn selector_target(raw: &str) -> GrantTarget {
    GrantTarget::Selector(raw.parse().unwrap())
}

pub(crate) fn grant(subject: Subject, target: GrantTarget, granted: &[&str]) -> Grant {
    Grant::new(subject, target, actions(granted)).unwrap()
}

pub(crate) fn deny(subject: Subject, target: GrantTarget, denied: &[&str]) -> DenyRule {
    DenyRule::new(subject, target, actions(denied)).unwrap()
}

/// One test fixture: the acting principal, its subject form for grants and
/// denies, and the membership facts bound to it.
pub(crate) struct Fixture {
    pub(crate) actor: PrincipalId,
    pub(crate) alice: Subject,
    pub(crate) membership: MembershipFacts,
}

impl Fixture {
    pub(crate) fn new() -> Self {
        let actor = PrincipalId::new();

        Self {
            actor,
            alice: Subject::Principal(actor),
            membership: MembershipFacts::new(actor),
        }
    }

    pub(crate) fn facts<'a>(
        &'a self,
        grants: &'a [Grant],
        denies: &'a [DenyRule],
    ) -> EvaluationFacts<'a> {
        EvaluationFacts {
            grants,
            denies,
            membership: &self.membership,
        }
    }
}

pub(crate) fn request(actor: PrincipalId, action_id: ActionId, target: ResourceRef) -> EvalRequest {
    EvalRequest {
        actor,
        is_root: false,
        action: action_id,
        target,
        path: Some(doc_path()),
        existence: Existence::Exists,
        ceiling: Ceiling::Unrestricted,
        deny_mode: DenyMode::Enforced,
    }
}

pub(crate) fn doc_path() -> ResourcePath {
    "acta::workspace::w1/folder::f1/document::d1"
        .parse()
        .unwrap()
}

pub(crate) fn read_request(actor: PrincipalId) -> EvalRequest {
    request(actor, action(READ), DOC.parse().unwrap())
}

pub(crate) const CUSTOS_READ: &str = "custos::grant::read";

pub(crate) fn role(name: &str, version: u32, granted: &[&str]) -> RoleSpec {
    RoleSpec {
        name: name.to_string(),
        version,
        actions: granted.iter().map(|raw| action(raw)).collect(),
    }
}

pub(crate) fn acta() -> ProductSpec {
    ProductSpec {
        product: "acta".to_string(),
        kinds: ["workspace", "folder", "document"]
            .map(String::from)
            .to_vec(),
        actions: [READ, UPDATE, "acta::folder::read"].map(action).to_vec(),
        roles: vec![
            role("viewer", 1, &[READ, "acta::folder::read"]),
            role("editor", 1, &[READ, UPDATE]),
            role("editor", 2, &[READ, UPDATE, "acta::folder::read"]),
        ],
        principal_sets: vec!["members".to_string()],
    }
}

pub(crate) fn custos() -> ProductSpec {
    ProductSpec {
        product: "custos".to_string(),
        kinds: vec!["grant".to_string()],
        actions: vec![action(CUSTOS_READ)],
        roles: vec![role("admin", 1, &[CUSTOS_READ])],
        principal_sets: Vec::new(),
    }
}

pub(crate) fn catalog() -> Catalog {
    Catalog::new([acta(), custos()]).unwrap()
}

pub(crate) fn role_ref(product: &str, name: &str, version: u32) -> RoleRef {
    RoleRef {
        product: product.to_string(),
        name: name.to_string(),
        version,
    }
}
