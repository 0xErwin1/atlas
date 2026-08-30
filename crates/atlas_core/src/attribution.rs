use crate::define_id;

define_id!(UserAttributionId);
define_id!(ApiKeyAttributionId);

/// The V1 attribution vocabulary: identifies who performed an action, as
/// either a human user or an api-key-authenticated principal.
///
/// This is V1 compatibility vocabulary, carrying id-space-scoped payloads
/// rather than a unified principal reference. `UserAttributionId` and
/// `ApiKeyAttributionId` are distinct newtypes with no conversion between
/// them, so a `User`/`ApiKey` pairing across the wrong id space is a
/// compile error rather than a silent value mistake. Revisit when
/// `custos.principals` exists (E13/D4 — `PrincipalId` unification), at
/// which point `User`/`ApiKey` should collapse into a single validated
/// `PrincipalId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
pub enum Attribution {
    User(UserAttributionId),
    /// An api-key-authenticated principal. In E13/D4, non-global api keys
    /// become Agent principals under `PrincipalId`; this variant is V1
    /// compatibility vocabulary, not the end-state shape.
    ApiKey(ApiKeyAttributionId),
}

/// Discriminates an `Attribution` by actor type, without carrying the id.
/// Used by audit/task-view filters (`AuditFilters`, `TaskViewFilters`) that
/// need to filter "created by any user" vs. "created by any api key" without
/// pinning a specific actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ::serde::Serialize, ::serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActorTypeFilter {
    User,
    ApiKey,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::any::TypeId;

    #[test]
    fn user_and_api_key_attribution_ids_are_distinct_types() {
        assert_ne!(
            TypeId::of::<UserAttributionId>(),
            TypeId::of::<ApiKeyAttributionId>(),
            "UserAttributionId and ApiKeyAttributionId must be distinct types \
             so cross-id-space construction is a compile error, not a silent \
             value mistake"
        );
    }
}
