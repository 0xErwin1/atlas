//! Wire shapes for the V2 authorization administration routes
//! (`/api/v2/custos/roles`, `/grants`, `/denies`). Subjects, targets and
//! authorities are tagged unions carrying the canonical string forms the
//! core id parsers accept (`<product>::<kind>::<id>` refs,
//! `<product>::<kind>::<id>/<kind>::<id>` paths, selectors with `*`/`**`,
//! `<product>::<kind>::<action>` actions).

use serde::{Deserialize, Serialize};

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// The subject a grant or deny rule is addressed to: exactly one of a
/// principal, a group or a named principal set (`<scope>::<set>`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SubjectDto {
    Principal { id: uuid::Uuid },
    Group { id: uuid::Uuid },
    PrincipalSet { id: String },
}

/// The resource side of a grant or deny rule, in canonical text form.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TargetDto {
    Ref { value: String },
    Path { value: String },
    Selector { value: String },
}

/// What a grant confers: a versioned built-in role of the target's product,
/// a custom role by id, or an explicit action list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorityDto {
    Builtin { name: String, version: u32 },
    Custom { role_id: uuid::Uuid },
    Actions { actions: Vec<String> },
}

/// Request body for `POST /api/v2/custos/roles`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateRoleRequest {
    pub product: String,
    pub name: String,
    pub actions: Vec<String>,
}

/// Request body for `PATCH /api/v2/custos/roles/{role_id}`. Absent fields
/// keep their current value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct UpdateRoleRequest {
    pub name: Option<String>,
    pub actions: Option<Vec<String>>,
}

/// A product-scoped custom role.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct RoleDto {
    pub id: uuid::Uuid,
    pub product: String,
    pub name: String,
    pub actions: Vec<String>,
    pub created_by: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/v2/custos/grants`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateGrantV2Request {
    pub subject: SubjectDto,
    pub target: TargetDto,
    pub authority: AuthorityDto,
}

/// A V2 grant. `product` is always the target's product.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct GrantV2Dto {
    pub id: uuid::Uuid,
    pub subject: SubjectDto,
    pub target: TargetDto,
    pub product: String,
    pub authority: AuthorityDto,
    pub created_by: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Request body for `POST /api/v2/custos/denies`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct CreateDenyRequest {
    pub subject: SubjectDto,
    pub target: TargetDto,
    pub actions: Vec<String>,
}

/// An explicit deny rule. `product` is always the target's product.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DenyRuleDto {
    pub id: uuid::Uuid,
    pub subject: SubjectDto,
    pub target: TargetDto,
    pub product: String,
    pub actions: Vec<String>,
    pub created_by: uuid::Uuid,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// The outcome of one authorization question. `not_found` is returned both
/// when the target does not exist and when the caller may not know it
/// exists; no reason or evidence is ever included.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AuthorizeDecision {
    Allow,
    Deny,
    NotFound,
}

/// Request body for `POST /api/v2/custos/authorize`: may the caller perform
/// `action` (`<product>::<kind>::<action>`) on `target`
/// (`<product>::<kind>::<id>`)?
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuthorizeRequest {
    pub action: String,
    pub target: String,
}

/// Response body for `POST /api/v2/custos/authorize`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuthorizeResponse {
    pub decision: AuthorizeDecision,
}

/// Request body for `POST /api/v2/custos/authorize/batch`: one action over
/// many targets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuthorizeBatchRequest {
    pub action: String,
    pub targets: Vec<String>,
}

/// One target's outcome in a batch response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuthorizeBatchResult {
    pub target: String,
    pub decision: AuthorizeDecision,
}

/// Response body for `POST /api/v2/custos/authorize/batch`: one result per
/// requested target, in request order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct AuthorizeBatchResponse {
    pub results: Vec<AuthorizeBatchResult>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decisions_use_their_snake_case_wire_names() {
        for (decision, wire) in [
            (AuthorizeDecision::Allow, "allow"),
            (AuthorizeDecision::Deny, "deny"),
            (AuthorizeDecision::NotFound, "not_found"),
        ] {
            assert_eq!(serde_json::to_value(decision).unwrap(), json!(wire));
            assert_eq!(
                serde_json::from_value::<AuthorizeDecision>(json!(wire)).unwrap(),
                decision
            );
        }
    }

    #[test]
    fn an_unknown_decision_is_rejected() {
        assert!(serde_json::from_value::<AuthorizeDecision>(json!("maybe")).is_err());
        assert!(serde_json::from_value::<AuthorizeDecision>(json!("Allow")).is_err());
    }

    #[test]
    fn authorize_request_and_response_round_trip() {
        let request = AuthorizeRequest {
            action: "custos::group::read".to_string(),
            target: "custos::group::g1".to_string(),
        };
        let wire = json!({ "action": "custos::group::read", "target": "custos::group::g1" });

        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<AuthorizeRequest>(wire).unwrap(),
            request
        );

        let response = AuthorizeResponse {
            decision: AuthorizeDecision::NotFound,
        };
        let wire = json!({ "decision": "not_found" });

        assert_eq!(serde_json::to_value(&response).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<AuthorizeResponse>(wire).unwrap(),
            response
        );
    }

    #[test]
    fn batch_request_and_response_round_trip_in_order() {
        let request = AuthorizeBatchRequest {
            action: "custos::group::read".to_string(),
            targets: vec![
                "custos::group::g2".to_string(),
                "custos::group::g1".to_string(),
            ],
        };
        let wire = json!({
            "action": "custos::group::read",
            "targets": ["custos::group::g2", "custos::group::g1"],
        });

        assert_eq!(serde_json::to_value(&request).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<AuthorizeBatchRequest>(wire).unwrap(),
            request
        );

        let response = AuthorizeBatchResponse {
            results: vec![
                AuthorizeBatchResult {
                    target: "custos::group::g2".to_string(),
                    decision: AuthorizeDecision::Allow,
                },
                AuthorizeBatchResult {
                    target: "custos::group::g1".to_string(),
                    decision: AuthorizeDecision::Deny,
                },
            ],
        };
        let wire = json!({
            "results": [
                { "target": "custos::group::g2", "decision": "allow" },
                { "target": "custos::group::g1", "decision": "deny" },
            ],
        });

        assert_eq!(serde_json::to_value(&response).unwrap(), wire);
        assert_eq!(
            serde_json::from_value::<AuthorizeBatchResponse>(wire).unwrap(),
            response
        );
    }

    #[test]
    fn a_request_missing_a_field_is_rejected() {
        assert!(
            serde_json::from_value::<AuthorizeRequest>(json!({ "action": "custos::group::read" }))
                .is_err()
        );
        assert!(serde_json::from_value::<AuthorizeBatchRequest>(json!({ "targets": [] })).is_err());
    }
}
