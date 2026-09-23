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
