#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::fmt;

use atlas_api::dtos::documents::ConflictProblemDto;
use atlas_client::{ClientError, helpers::ResolverError};

#[derive(Debug)]
pub(crate) enum CliError {
    Client(Box<ClientError>),
    Conflict(Box<ConflictProblemDto>),
    Resolver(Box<ResolverError>),
    Config(String),
    Io(std::io::Error),
    Validation(String),
}

impl CliError {
    /// Maps the error to the CLI's documented exit-code scheme:
    ///
    /// - 1 — generic failure (API error, I/O, decode, resolution)
    /// - 2 — usage error (clap parse errors also use 2)
    /// - 3 — configuration error
    /// - 4 — network/transport error
    /// - 5 — document revision conflict
    pub(crate) fn exit_code(&self) -> u8 {
        match self {
            Self::Client(e) => match e.as_ref() {
                ClientError::Transport(_) => 4,
                ClientError::Conflict(_) => 5,
                ClientError::Api(_) | ClientError::Decode { .. } => 1,
            },
            Self::Conflict(_) => 5,
            Self::Resolver(_) | Self::Io(_) => 1,
            Self::Validation(_) => 2,
            Self::Config(_) => 3,
        }
    }

    /// Returns the server-assigned request id when the error carries an RFC 9457
    /// problem response, so machine consumers can correlate with server logs.
    pub(crate) fn request_id(&self) -> Option<&str> {
        match self {
            Self::Client(e) => match e.as_ref() {
                ClientError::Api(problem) => problem.request_id.as_deref(),
                _ => None,
            },
            _ => None,
        }
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
            Self::Resolver(e) => write!(f, "{e}"),
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
            Self::Resolver(e) => Some(e.as_ref()),
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

impl From<ResolverError> for CliError {
    fn from(e: ResolverError) -> Self {
        Self::Resolver(Box::new(e))
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

    fn api_problem() -> atlas_api::problem::ProblemDetails {
        atlas_api::problem::ProblemDetails::new("urn:atlas:error:invalid", "Invalid input", 422)
    }

    #[test]
    fn exit_code_is_3_for_config() {
        let e = CliError::Config("bad toml".into());
        assert_eq!(e.exit_code(), 3);
    }

    #[test]
    fn exit_code_is_1_for_io() {
        let e = CliError::Io(io_error());
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn exit_code_is_2_for_validation() {
        let e = CliError::Validation("bad priority".into());
        assert_eq!(e.exit_code(), 2);
    }

    #[test]
    fn exit_code_is_4_for_transport() {
        let e = CliError::Client(Box::new(client_error()));
        assert_eq!(e.exit_code(), 4);
    }

    #[test]
    fn exit_code_is_1_for_api_error() {
        let e = CliError::Client(Box::new(ClientError::Api(api_problem())));
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn exit_code_is_5_for_conflict() {
        use atlas_api::dtos::documents::ConflictProblemDto;
        let dto = ConflictProblemDto::new(uuid::Uuid::now_v7(), 3, "--- a\n+++ b\n".to_owned());
        let e = CliError::Conflict(Box::new(dto));
        assert_eq!(e.exit_code(), 5);
    }

    #[test]
    fn request_id_returned_for_api_error() {
        let mut problem = api_problem();
        problem.request_id = Some("req-123".to_owned());
        let e = CliError::Client(Box::new(ClientError::Api(problem)));
        assert_eq!(e.request_id(), Some("req-123"));
    }

    #[test]
    fn request_id_absent_for_non_api_errors() {
        let e = CliError::Config("bad toml".into());
        assert!(e.request_id().is_none());
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

    #[test]
    fn exit_code_is_1_for_resolver() {
        let e = CliError::Resolver(Box::new(ResolverError::BoardNotFound {
            board_ref: "x".into(),
            workspace: "ws".into(),
        }));
        assert_eq!(e.exit_code(), 1);
    }

    #[test]
    fn display_resolver_is_non_empty() {
        let e = CliError::Resolver(Box::new(ResolverError::BoardNotFound {
            board_ref: "x".into(),
            workspace: "ws".into(),
        }));
        let s = e.to_string();
        assert!(!s.is_empty());
        assert!(s.contains("x"), "display must echo the board_ref");
    }

    #[test]
    fn from_resolver_error_produces_resolver_variant() {
        let re = ResolverError::InvalidBoardUuid {
            board_id: "bad".into(),
        };
        let e: CliError = re.into();
        assert!(matches!(e, CliError::Resolver(_)));
    }
}
