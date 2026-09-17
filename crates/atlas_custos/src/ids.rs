use atlas_core::define_id;

/// Principal identity ids. Owned by `atlas_core::principal` (D4); re-exported
/// here so Custos entities/ports can reference them without a second
/// definition.
pub use atlas_core::principal::{ApiKeyId, GroupId, UserId};

define_id!(SessionId);
define_id!(ActivationTokenId);
define_id!(SecurityAuditId);
define_id!(PrincipalId);

/// A user's principal identity is the user row itself: the back-fill (and
/// every later user creation) uses `users.id` as the principal id.
impl From<UserId> for PrincipalId {
    fn from(id: UserId) -> Self {
        Self(id.0)
    }
}
