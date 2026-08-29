//! A wrapper that keeps a value out of logs and error messages
//! (SHELL-CFG-3). `Secret<T>` never formats `T`: its `Debug` implementation
//! prints a fixed redaction marker regardless of what `T` is.

use std::fmt;

/// A value that must never be printed, logged, serialized, or displayed.
///
/// `expose` is the only accessor. `Secret<T>` deliberately does not
/// implement `Display`, `Serialize`, or `Deserialize` for any `T`: those
/// impls would create a path for the wrapped value to leak.
///
/// Equality, when `T: PartialEq`, is the derived byte-wise comparison and is
/// **not** constant-time. Never use it to authenticate a credential.
pub struct Secret<T>(T);

impl<T> Secret<T> {
    /// Wraps `value` so it can no longer be printed or logged accidentally.
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// Returns a reference to the wrapped value. The sole accessor.
    pub fn expose(&self) -> &T {
        &self.0
    }
}

impl<T: Clone> Clone for Secret<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: PartialEq> PartialEq for Secret<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq> Eq for Secret<T> {}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Secret(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    struct NonDebugType(u8);

    #[test]
    fn debug_output_is_always_redacted_for_string() {
        let secret = Secret::new("super-secret-token".to_string());

        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn debug_output_is_always_redacted_for_byte_array() {
        let secret = Secret::new([7u8; 32]);

        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn expose_returns_the_inner_value() {
        let secret = Secret::new("x".to_string());

        assert_eq!(secret.expose(), &"x".to_string());
    }

    #[test]
    fn secret_of_non_debug_type_compiles() {
        let secret = Secret::new(NonDebugType(1));

        assert_eq!(format!("{secret:?}"), "Secret(<redacted>)");
    }

    #[test]
    fn clone_and_eq_require_bounds_on_impl_not_struct() {
        let original = Secret::new("x".to_string());
        let cloned = original.clone();

        assert_eq!(cloned, original);
    }
}
