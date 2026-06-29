#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::fmt;

use atlas_api::dtos::documents::ConflictProblemDto;
use atlas_client::ClientError;

#[derive(Debug)]
pub(crate) enum CliError {
    Client(Box<ClientError>),
    // Used starting from B6 (docs edit CAS path); forward-declared here so the
    // error hierarchy is complete and From<ClientError::Conflict> can route correctly.
    #[allow(dead_code)]
    Conflict(Box<ConflictProblemDto>),
    Config(String),
    Io(std::io::Error),
    Validation(String),
}

impl CliError {
    pub(crate) fn exit_code(&self) -> u8 {
        1
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Client(e) => write!(f, "{e}"),
            Self::Conflict(dto) => write!(
                f,
                "revision conflict: current_seq={}, current_revision_id={}",
                dto.current_seq, dto.current_revision_id
            ),
            Self::Config(msg) => write!(f, "config error: {msg}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::Validation(msg) => write!(f, "validation error: {msg}"),
        }
    }
}

impl std::error::Error for CliError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Client(e) => Some(e.as_ref()),
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<ClientError> for CliError {
    fn from(e: ClientError) -> Self {
        Self::Client(Box::new(e))
    }
}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn io_error() -> std::io::Error {
        std::io::Error::new(std::io::ErrorKind::NotFound, "file not found")
    }

    fn client_error() -> ClientError {
        ClientError::Transport(reqwest::Client::new().get("not-a-url").build().unwrap_err())
    }

    #[test]
    fn exit_code_is_always_1_for_config() {
        let e = CliError::Config("bad toml".into());
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn exit_code_is_always_1_for_io() {
        let e = CliError::Io(io_error());
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn exit_code_is_always_1_for_validation() {
        let e = CliError::Validation("bad priority".into());
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn exit_code_is_always_1_for_client() {
        let e = CliError::Client(Box::new(client_error()));
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn display_config_is_non_empty() {
        let e = CliError::Config("bad toml".into());
        let s = e.to_string();
        assert!(!s.is_empty());
        assert!(s.contains("bad toml"));
    }

    #[test]
    fn display_io_is_non_empty() {
        let e = CliError::Io(io_error());
        let s = e.to_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn display_validation_is_non_empty() {
        let e = CliError::Validation("must be positive".into());
        let s = e.to_string();
        assert!(!s.is_empty());
        assert!(s.contains("must be positive"));
    }

    #[test]
    fn display_client_is_non_empty() {
        let e = CliError::Client(Box::new(client_error()));
        let s = e.to_string();
        assert!(!s.is_empty());
    }

    #[test]
    fn from_io_error_produces_io_variant() {
        let e: CliError = io_error().into();
        assert!(matches!(e, CliError::Io(_)));
    }

    #[test]
    fn from_client_error_produces_client_variant() {
        let e: CliError = client_error().into();
        assert!(matches!(e, CliError::Client(_)));
    }
}
