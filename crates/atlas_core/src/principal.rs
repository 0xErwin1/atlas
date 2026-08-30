use crate::define_id;

define_id!(UserId);
define_id!(ApiKeyId);
define_id!(GroupId);

/// The auth-time actor vocabulary: identifies who is acting, as a human user,
/// an api-key-authenticated agent, or a group (a grant target, never an
/// authenticating actor on its own).
///
/// Distinct from `Attribution`, which records who performed a past action for
/// audit purposes using a separate id space (`UserAttributionId` /
/// `ApiKeyAttributionId`, S2c). `Principal` is the live authorization-time
/// identity; unifying the two id spaces is deferred to E13/D4.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Principal {
    User(UserId),
    ApiKey(ApiKeyId),
    Group(GroupId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_variants_round_trip_their_ids() {
        let user_id = UserId::new();
        let key_id = ApiKeyId::new();
        let group_id = GroupId::new();

        assert_eq!(Principal::User(user_id), Principal::User(user_id));
        assert_eq!(Principal::ApiKey(key_id), Principal::ApiKey(key_id));
        assert_eq!(Principal::Group(group_id), Principal::Group(group_id));
        assert_ne!(Principal::User(user_id), Principal::ApiKey(key_id));
    }
}
