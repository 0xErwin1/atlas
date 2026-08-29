use crate::ids::resource_ref::ResourceRef;

/// The shared error vocabulary returned by every capability trait.
///
/// `NotFound.target` is deliberately an opaque `String`, not a typed
/// `ResourceRef`: share tokens and blob keys are not resource refs, and
/// `ShareLinkCredentialPort::validate` requires an outcome that never reveals
/// whether a token existed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CapabilityError {
    /// The requested target does not exist, or (for secrets) cannot be
    /// distinguished from not existing.
    #[error("{target} not found")]
    NotFound {
        /// An opaque description of what was not found.
        target: String,
    },
    /// The capability could not answer because its backend failed
    /// (timeout, unreachable, internal error). Distinct from a valid
    /// negative answer.
    #[error("capability unavailable: {reason}")]
    Unavailable {
        /// A human-readable description of the failure.
        reason: String,
    },
    /// The caller's request was malformed or out of bounds for this
    /// provider.
    #[error("invalid request: {message}")]
    Invalid {
        /// A human-readable description of the problem.
        message: String,
    },
}

impl CapabilityError {
    /// Builds a `NotFound` error from an opaque target description.
    pub fn not_found(target: impl Into<String>) -> Self {
        Self::NotFound {
            target: target.into(),
        }
    }

    /// Builds a `NotFound` error from a resource ref, using its canonical
    /// string representation as the opaque target.
    pub fn not_found_ref(resource: &ResourceRef) -> Self {
        Self::NotFound {
            target: resource.to_string(),
        }
    }

    /// Builds an `Unavailable` error from a failure reason.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self::Unavailable {
            reason: reason.into(),
        }
    }

    /// Builds an `Invalid` error from a problem description.
    pub fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_variant_renders_its_message() {
        let cases = [
            (
                CapabilityError::not_found("share token"),
                "share token not found",
            ),
            (
                CapabilityError::unavailable("backend timeout"),
                "capability unavailable: backend timeout",
            ),
            (
                CapabilityError::invalid("malformed query"),
                "invalid request: malformed query",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn not_found_ref_renders_canonical_ref_string() {
        let resource: ResourceRef = "acta::document::42".parse().expect("valid resource ref");
        let error = CapabilityError::not_found_ref(&resource);

        assert_eq!(error.to_string(), "acta::document::42 not found");
    }
}
