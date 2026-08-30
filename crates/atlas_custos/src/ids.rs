use atlas_core::define_id;

/// Principal identity ids. Owned by `atlas_core::principal` (D4); re-exported
/// here so Custos entities/ports can reference them without a second
/// definition.
pub use atlas_core::principal::{ApiKeyId, GroupId, UserId};

define_id!(SessionId);
define_id!(ActivationTokenId);
define_id!(SecurityAuditId);
