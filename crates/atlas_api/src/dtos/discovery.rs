//! DTOs for `GET /api/v2/custos/discover` (E11-S8 design D5).

use serde::{Deserialize, Serialize};
#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// One component's discoverable scopes for the calling principal.
///
/// `scopes` holds flat `ResourceRef` strings (e.g. `acta::workspace::<id>`,
/// `acta::project::<id>`), never slash-composed paths (design spec
/// reconciliation F1) — a grant on a parent resource already covers
/// everything beneath it through the existing chain-walk (D-S8-2), so
/// discover returns the granted refs themselves, not their descendants.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DiscoveredComponentDto {
    pub component: String,
    pub scopes: Vec<String>,
}

/// Response from `GET /api/v2/custos/discover`.
///
/// For a non-admin principal, a component absent from `components` means
/// nothing is discoverable there (INV-ABSENT-NOT-EMPTY) — never an empty
/// `scopes` entry as a substitute. For `admin: true`, every present
/// registry component is listed with an empty `scopes` array: admin reach
/// is total, not scoped, so there is nothing to enumerate per component,
/// and the component is still present rather than absent. `admin` is
/// `is_root || is_system_admin` (INV-ADMIN-FROM-FLAGS), never derived from
/// grant volume. `truncated` surfaces `atlas_custos`'s `GrantedScopes`
/// 500-scope cap (`false` on the admin short-circuit, which reads zero
/// grant rows).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct DiscoverResponseDto {
    pub components: Vec<DiscoveredComponentDto>,
    pub admin: bool,
    pub truncated: bool,
}
