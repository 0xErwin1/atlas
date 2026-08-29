//! Typed configuration errors that never echo the value they reject
//! (SHELL-CFG-1). Every variant names the variable or a composed message,
//! never the offending value.

/// An error produced while loading or validating component configuration.
///
/// No variant, nor its `Display`/`Debug` output, ever carries the value of an
/// environment variable. `Invalid.reason` is a human-readable description of
/// why the value was rejected, not the value itself.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ConfigError {
    /// A required environment variable was not set (or was empty).
    #[error("required configuration variable `{name}` is not set")]
    Missing {
        /// The name of the missing environment variable.
        name: String,
    },
    /// An environment variable was set but its value could not be parsed
    /// or accepted.
    #[error("configuration variable `{name}` is invalid: {reason}")]
    Invalid {
        /// The name of the invalid environment variable.
        name: String,
        /// A human-readable description of why the value was rejected.
        /// MUST NOT contain the offending value.
        reason: String,
    },
    /// A cross-field or multi-variable invariant was violated, with no
    /// single variable responsible.
    #[error("invalid configuration: {message}")]
    Composition {
        /// A human-readable description of the violated invariant.
        message: String,
    },
}

impl ConfigError {
    /// Builds a `Missing` error naming the unset variable.
    pub fn missing(name: impl Into<String>) -> Self {
        Self::Missing { name: name.into() }
    }

    /// Builds an `Invalid` error naming the variable and the rejection
    /// reason. `reason` MUST NOT contain the offending value.
    pub fn invalid(name: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::Invalid {
            name: name.into(),
            reason: reason.into(),
        }
    }

    /// Builds a `Composition` error for a cross-field invariant violation.
    pub fn composition(message: impl Into<String>) -> Self {
        Self::Composition {
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_names_the_variable_not_a_value() {
        let error = ConfigError::missing("DATABASE_URL");

        assert_eq!(
            error.to_string(),
            "required configuration variable `DATABASE_URL` is not set"
        );
        assert!(!error.to_string().contains("postgres://"));
    }

    #[test]
    fn invalid_reason_carries_no_value() {
        let error = ConfigError::invalid("PORT", "must be a valid u16");

        let display = error.to_string();
        let debug = format!("{error:?}");

        assert!(display.contains("PORT"));
        assert!(display.contains("must be a valid u16"));
        assert!(!display.contains("not-a-port"));
        assert!(!debug.contains("not-a-port"));
    }

    #[test]
    fn composition_has_no_culprit_variable() {
        let error = ConfigError::composition("message");

        match error {
            ConfigError::Composition { message } => assert_eq!(message, "message"),
            _ => panic!("expected ConfigError::Composition"),
        }
    }

    #[test]
    fn each_variant_renders_its_message() {
        let cases = [
            (
                ConfigError::missing("DATABASE_URL"),
                "required configuration variable `DATABASE_URL` is not set",
            ),
            (
                ConfigError::invalid("PORT", "must be a valid u16"),
                "configuration variable `PORT` is invalid: must be a valid u16",
            ),
            (
                ConfigError::composition("min_connections must not exceed max_connections"),
                "invalid configuration: min_connections must not exceed max_connections",
            ),
        ];

        for (error, expected) in cases {
            assert_eq!(error.to_string(), expected);
        }
    }

    #[test]
    fn constructors_equal_struct_literals() {
        assert_eq!(
            ConfigError::missing("X"),
            ConfigError::Missing { name: "X".into() }
        );
        assert_eq!(
            ConfigError::invalid("X", "bad"),
            ConfigError::Invalid {
                name: "X".into(),
                reason: "bad".into(),
            }
        );
        assert_eq!(
            ConfigError::composition("bad combo"),
            ConfigError::Composition {
                message: "bad combo".into(),
            }
        );
    }
}
