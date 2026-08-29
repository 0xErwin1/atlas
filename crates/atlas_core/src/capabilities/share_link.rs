use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::ids::action_id::ActionId;
use crate::ids::resource_ref::ResourceRef;

use super::error::CapabilityError;

/// The maximum length, in bytes, of a `ShareToken`.
pub const MAX_SHARE_TOKEN_BYTES: usize = 512;

/// An opaque share-link token. Never implements `Display`, `Serialize`, or
/// `Deserialize`: `expose()` is the only, deliberately awkward, way to read
/// the raw value, so every leak site greps (SHELL-CFG-2).
#[derive(Clone, PartialEq, Eq)]
pub struct ShareToken(String);

/// The error returned when a string is not a valid `ShareToken`. Never
/// echoes the rejected value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShareTokenError {
    /// The token was empty.
    #[error("share token is empty")]
    Empty,
    /// The token exceeded `MAX_SHARE_TOKEN_BYTES`.
    #[error("share token exceeds the maximum length of {max} bytes")]
    TooLong {
        /// The maximum allowed length, in bytes.
        max: usize,
    },
}

impl ShareToken {
    /// Validates and wraps `value` as a `ShareToken`.
    pub fn new(value: impl Into<String>) -> Result<Self, ShareTokenError> {
        let value = value.into();

        if value.is_empty() {
            return Err(ShareTokenError::Empty);
        }

        if value.len() > MAX_SHARE_TOKEN_BYTES {
            return Err(ShareTokenError::TooLong {
                max: MAX_SHARE_TOKEN_BYTES,
            });
        }

        Ok(Self(value))
    }

    /// Returns the raw token value. Named to make every call site grep-able.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ShareToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ShareToken(<redacted>)")
    }
}

/// An opaque password hash guarding a share link. Never implements
/// `Display`, `Serialize`, or `Deserialize` (SHELL-CFG-2).
#[derive(Clone, PartialEq, Eq)]
pub struct PasswordHash(String);

/// The error returned when a string is not a valid `PasswordHash`. Never
/// echoes the rejected value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasswordHashError {
    /// The hash was empty.
    #[error("password hash is empty")]
    Empty,
}

impl PasswordHash {
    /// Validates and wraps `value` as a `PasswordHash`.
    pub fn new(value: impl Into<String>) -> Result<Self, PasswordHashError> {
        let value = value.into();

        if value.is_empty() {
            return Err(PasswordHashError::Empty);
        }

        Ok(Self(value))
    }

    /// Returns the raw hash value. Named to make every call site grep-able.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for PasswordHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "PasswordHash(<redacted>)")
    }
}

/// The parameters for issuing a share link.
#[derive(Debug, Clone, PartialEq)]
pub struct ShareLinkGrant {
    /// The resource the link grants access to.
    pub target: ResourceRef,
    /// The action the link permits.
    pub action: ActionId,
    /// The password hash guarding the link, if any.
    pub password: Option<PasswordHash>,
    /// The time after which the link is no longer valid, if any.
    pub expires_at: Option<DateTime<Utc>>,
}

/// Issues, validates, and revokes share-link credentials, implementing
/// CUSTOS-SHARE-1..3.
///
/// `validate` collapses every content-dependent mismatch (unknown token,
/// expired, wrong action, wrong target) into the identical
/// `CapabilityError::NotFound`, revealing nothing about which condition
/// failed.
#[async_trait]
pub trait ShareLinkCredentialPort: Send + Sync {
    /// Issues a new `ShareToken` for `grant`.
    async fn issue(&self, grant: ShareLinkGrant) -> Result<ShareToken, CapabilityError>;

    /// Validates `token` against the required `action` and `target`.
    ///
    /// Every content-dependent mismatch returns the identical
    /// `CapabilityError::NotFound`. A backend failure unrelated to token
    /// content returns `CapabilityError::Unavailable` instead.
    async fn validate(
        &self,
        token: &ShareToken,
        action: &ActionId,
        target: &ResourceRef,
    ) -> Result<(), CapabilityError>;

    /// Revokes `token`, so future `validate` calls collapse to the same
    /// outcome as an unknown token.
    async fn revoke(&self, token: &ShareToken) -> Result<(), CapabilityError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::test_support::block_on;
    use std::sync::Mutex;

    #[test]
    fn debug_never_prints_the_raw_token() {
        let token = ShareToken::new("super-secret").expect("valid token");
        let debug = format!("{token:?}");

        assert_eq!(debug, "ShareToken(<redacted>)");
        assert!(!debug.contains("super-secret"));
    }

    #[test]
    fn debug_never_prints_the_raw_password_hash() {
        let hash = PasswordHash::new("hashed-secret").expect("valid hash");
        let debug = format!("{hash:?}");

        assert_eq!(debug, "PasswordHash(<redacted>)");
        assert!(!debug.contains("hashed-secret"));
    }

    #[test]
    fn expose_returns_the_raw_value() {
        let token = ShareToken::new("super-secret").expect("valid token");
        assert_eq!(token.expose(), "super-secret");

        let hash = PasswordHash::new("hashed-secret").expect("valid hash");
        assert_eq!(hash.expose(), "hashed-secret");
    }

    #[test]
    fn empty_token_and_hash_are_rejected_without_echoing_the_value() {
        let error = ShareToken::new("").unwrap_err();
        assert_eq!(error, ShareTokenError::Empty);
        assert!(!error.to_string().contains("''"));

        let error = PasswordHash::new("").unwrap_err();
        assert_eq!(error, PasswordHashError::Empty);
    }

    struct StubPort {
        issued: Mutex<Vec<(String, ActionId, ResourceRef)>>,
        revoked: Mutex<Vec<String>>,
        fail: bool,
    }

    impl StubPort {
        fn new() -> Self {
            Self {
                issued: Mutex::new(Vec::new()),
                revoked: Mutex::new(Vec::new()),
                fail: false,
            }
        }

        fn failing() -> Self {
            Self {
                issued: Mutex::new(Vec::new()),
                revoked: Mutex::new(Vec::new()),
                fail: true,
            }
        }
    }

    #[async_trait]
    impl ShareLinkCredentialPort for StubPort {
        async fn issue(&self, grant: ShareLinkGrant) -> Result<ShareToken, CapabilityError> {
            let token =
                ShareToken::new(format!("token-{}", self.issued.lock().expect("lock").len()))
                    .expect("valid token");

            self.issued.lock().expect("lock").push((
                token.expose().to_string(),
                grant.action,
                grant.target,
            ));

            Ok(token)
        }

        async fn validate(
            &self,
            token: &ShareToken,
            action: &ActionId,
            target: &ResourceRef,
        ) -> Result<(), CapabilityError> {
            if self.fail {
                return Err(CapabilityError::unavailable("store unreachable"));
            }

            if self
                .revoked
                .lock()
                .expect("lock")
                .contains(&token.expose().to_string())
            {
                return Err(CapabilityError::not_found("share token"));
            }

            let issued = self.issued.lock().expect("lock");
            let matched = issued
                .iter()
                .find(|(value, ..)| value == token.expose())
                .filter(|(_, granted_action, granted_target)| {
                    granted_action == action && granted_target == target
                });

            match matched {
                Some(_) => Ok(()),
                None => Err(CapabilityError::not_found("share token")),
            }
        }

        async fn revoke(&self, token: &ShareToken) -> Result<(), CapabilityError> {
            self.revoked
                .lock()
                .expect("lock")
                .push(token.expose().to_string());
            Ok(())
        }
    }

    #[test]
    fn share_link_credential_port_is_object_safe() {
        let _: Option<Box<dyn ShareLinkCredentialPort>> = None;
    }

    fn grant() -> ShareLinkGrant {
        ShareLinkGrant {
            target: "acta::document::42".parse().expect("valid resource ref"),
            action: "acta::document::read".parse().expect("valid action id"),
            password: None,
            expires_at: None,
        }
    }

    #[test]
    fn issue_then_validate_succeeds() {
        let port: Box<dyn ShareLinkCredentialPort> = Box::new(StubPort::new());
        let grant = grant();

        let token = block_on(port.issue(grant.clone())).expect("issue succeeds");

        block_on(port.validate(&token, &grant.action, &grant.target)).expect("validate succeeds");
    }

    #[test]
    fn all_mismatches_collapse_to_one_outcome() {
        let port: Box<dyn ShareLinkCredentialPort> = Box::new(StubPort::new());
        let grant = grant();
        let token = block_on(port.issue(grant.clone())).expect("issue succeeds");

        let unknown = ShareToken::new("unknown-token").expect("valid token");
        let wrong_action: ActionId = "acta::document::delete".parse().expect("valid action id");
        let wrong_target: ResourceRef = "acta::document::99".parse().expect("valid resource ref");

        let cases = [
            block_on(port.validate(&unknown, &grant.action, &grant.target)).unwrap_err(),
            block_on(port.validate(&token, &wrong_action, &grant.target)).unwrap_err(),
            block_on(port.validate(&token, &grant.action, &wrong_target)).unwrap_err(),
        ];

        for error in cases {
            assert_eq!(error, CapabilityError::not_found("share token"));
        }
    }

    #[test]
    fn infrastructure_failure_is_distinguishable_from_mismatch() {
        let port: Box<dyn ShareLinkCredentialPort> = Box::new(StubPort::failing());
        let grant = grant();
        let token = ShareToken::new("any-token").expect("valid token");

        let error = block_on(port.validate(&token, &grant.action, &grant.target)).unwrap_err();

        assert_eq!(error, CapabilityError::unavailable("store unreachable"));
    }

    #[test]
    fn revoke_invalidates_future_validation() {
        let port: Box<dyn ShareLinkCredentialPort> = Box::new(StubPort::new());
        let grant = grant();
        let token = block_on(port.issue(grant.clone())).expect("issue succeeds");

        block_on(port.revoke(&token)).expect("revoke succeeds");
        let error = block_on(port.validate(&token, &grant.action, &grant.target)).unwrap_err();

        assert_eq!(error, CapabilityError::not_found("share token"));
    }
}
