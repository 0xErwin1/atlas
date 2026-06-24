pub(crate) mod activate;
pub(crate) mod api_keys;
pub(crate) mod audit;
pub(crate) mod auth;
pub(crate) mod boards;
pub(crate) mod documents;
pub(crate) mod folders;
pub(crate) mod grants;
pub(crate) mod health;
pub(crate) mod members;
pub mod openapi;
pub(crate) mod projects;
pub mod registry;
pub(crate) mod saved_searches;
pub(crate) mod search;
pub(crate) mod status_templates;
pub(crate) mod tags;
pub(crate) mod task_views;
pub(crate) mod tasks;
pub(crate) mod ui_state;
pub(crate) mod users;
pub(crate) mod validation;
pub(crate) mod workspaces;

/// Derives the account lifecycle state from a user's disable/activation timestamps.
///
/// Precedence: deactivated wins over pending (a disabled, never-activated account
/// reads "deactivated", not "pending"). This is the single source of truth for the
/// derivation rule — call it at every resolution site, never inline the ternary.
///
/// Invariant (keep + mark): disabled and pending users are NOT hidden from assignee
/// reads or member lists; they are marked with this derived state instead. Do NOT
/// add `disabled_at IS NULL` filters to assignee or member SQL queries — that would
/// silently break this contract.
pub(crate) fn account_status(
    disabled_at: Option<chrono::DateTime<chrono::Utc>>,
    activated_at: Option<chrono::DateTime<chrono::Utc>>,
) -> &'static str {
    if disabled_at.is_some() {
        "deactivated"
    } else if activated_at.is_none() {
        "pending"
    } else {
        "active"
    }
}
