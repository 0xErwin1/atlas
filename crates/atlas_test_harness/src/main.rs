//! Starts an ephemeral `pgvector/pgvector:pg17` container, exports
//! `ATLAS_TEST_DATABASE_URL` for it, and execs the given command as a child,
//! propagating the child's exit code.
//!
//! `cargo nextest` is process-per-test, so a container owned by test code
//! would be started once per test process instead of once per run. The
//! container therefore lives in this process, one level above whatever test
//! command it wraps.
//!
//! Uses the blocking `SyncRunner` rather than the async API: nothing here is
//! concurrent, and dropping an async container from inside an async runtime
//! can panic when `Drop`'s internal `block_on` collides with the caller's own
//! runtime. The `watchdog` feature is enabled so Ctrl-C (SIGINT/SIGTERM/
//! SIGQUIT) still removes the container; it cannot catch SIGKILL, so a
//! SIGKILLed harness leaks its container (there is no Ryuk in this crate).

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::env;
use std::fmt;
use std::process::{Command, ExitCode, ExitStatus};

use testcontainers::ImageExt;
use testcontainers::runners::SyncRunner;
use testcontainers_modules::postgres::Postgres;

const DB_NAME: &str = "atlas_dev";
const DB_USER: &str = "atlas";
const DB_PASSWORD: &str = "atlas";
const IMAGE_NAME: &str = "docker.io/pgvector/pgvector";
const IMAGE_TAG: &str = "pg17";
const SHM_SIZE_BYTES: u64 = 512 * 1024 * 1024;
const POSTGRES_PORT: u16 = 5432;

const DOCKER_HOST_VAR: &str = "DOCKER_HOST";
const DATABASE_URL_VAR: &str = "ATLAS_TEST_DATABASE_URL";

#[derive(Debug)]
enum HarnessError {
    MissingCommand,
    DockerHostUnset,
    ContainerStart(String),
    ContainerHost(String),
    ContainerPort(String),
    Exec { program: String, source: String },
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingCommand => {
                write!(f, "usage: atlas-test-harness <command> [args...]")
            }
            Self::DockerHostUnset => write!(
                f,
                "{DOCKER_HOST_VAR} is not set — testcontainers cannot reach rootless podman.\n\
                 One-time host setup: `systemctl --user enable --now podman.socket`.\n\
                 Then export {DOCKER_HOST_VAR}=unix:///run/user/$UID/podman/podman.sock."
            ),
            Self::ContainerStart(error) => {
                write!(f, "failed to start the Postgres test container: {error}")
            }
            Self::ContainerHost(error) => {
                write!(f, "failed to resolve the test container host: {error}")
            }
            Self::ContainerPort(error) => {
                write!(f, "failed to resolve the test container port: {error}")
            }
            Self::Exec { program, source } => {
                write!(f, "failed to execute `{program}`: {source}")
            }
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(exit_code) => exit_code,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<ExitCode, HarnessError> {
    let (program, command_args) = command_from_args(env::args().skip(1))?;

    validate_docker_host(env::var(DOCKER_HOST_VAR).ok().as_deref())?;

    let container = start_container()?;
    let host = container
        .get_host()
        .map_err(|error| HarnessError::ContainerHost(error.to_string()))?;
    let port = container
        .get_host_port_ipv4(POSTGRES_PORT)
        .map_err(|error| HarnessError::ContainerPort(error.to_string()))?;

    let database_url = fixture_database_url(&host.to_string(), port);

    let status = Command::new(&program)
        .args(&command_args)
        .env(DATABASE_URL_VAR, database_url)
        .status()
        .map_err(|error| HarnessError::Exec {
            program: program.clone(),
            source: error.to_string(),
        })?;

    Ok(exit_code_from(status))
}

/// Splits `atlas-test-harness <command> [args...]` into the child program and
/// its arguments.
fn command_from_args(
    mut args: impl Iterator<Item = String>,
) -> Result<(String, Vec<String>), HarnessError> {
    let program = args.next().ok_or(HarnessError::MissingCommand)?;

    Ok((program, args.collect()))
}

/// Rejects a missing `DOCKER_HOST`: testcontainers only auto-detects rootless
/// *Docker* socket paths, never podman's, so an unset variable here produces a
/// confusing connection error instead of an actionable one.
fn validate_docker_host(raw: Option<&str>) -> Result<(), HarnessError> {
    match raw {
        Some(_) => Ok(()),
        None => Err(HarnessError::DockerHostUnset),
    }
}

fn fixture_database_url(host: &str, port: u16) -> String {
    format!("postgres://{DB_USER}:{DB_PASSWORD}@{host}:{port}/{DB_NAME}")
}

fn exit_code_from(status: ExitStatus) -> ExitCode {
    match status.code() {
        Some(code) => ExitCode::from(code as u8),
        None => ExitCode::FAILURE,
    }
}

/// Builds the container request.
///
/// The Postgres-specific builders (`with_db_name`/`with_user`/`with_password`)
/// are inherent methods on `Postgres` and must run before any `ImageExt`
/// method (`with_name`/`with_tag`/`with_shm_size`): those convert `Postgres`
/// into `ContainerRequest<Postgres>`, which no longer exposes the inherent
/// methods.
fn start_container() -> Result<testcontainers::Container<Postgres>, HarnessError> {
    Postgres::default()
        .with_db_name(DB_NAME)
        .with_user(DB_USER)
        .with_password(DB_PASSWORD)
        .with_name(IMAGE_NAME)
        .with_tag(IMAGE_TAG)
        .with_shm_size(SHM_SIZE_BYTES)
        .start()
        .map_err(|error| HarnessError::ContainerStart(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_with_arguments_splits_into_program_and_args() {
        let args = ["cargo", "nextest", "run", "--workspace"]
            .into_iter()
            .map(str::to_owned);

        let (program, command_args) = command_from_args(args).expect("expected Ok");

        assert_eq!(program, "cargo");
        assert_eq!(command_args, vec!["nextest", "run", "--workspace"]);
    }

    #[test]
    fn a_command_with_no_arguments_splits_into_program_and_an_empty_list() {
        let args = ["bash"].into_iter().map(str::to_owned);

        let (program, command_args) = command_from_args(args).expect("expected Ok");

        assert_eq!(program, "bash");
        assert!(command_args.is_empty());
    }

    #[test]
    fn no_arguments_at_all_is_rejected() {
        let args = std::iter::empty::<String>();

        assert!(matches!(
            command_from_args(args),
            Err(HarnessError::MissingCommand)
        ));
    }

    #[test]
    fn a_set_docker_host_is_accepted() {
        assert!(validate_docker_host(Some("unix:///run/user/1000/podman/podman.sock")).is_ok());
    }

    #[test]
    fn an_unset_docker_host_is_rejected() {
        assert!(matches!(
            validate_docker_host(None),
            Err(HarnessError::DockerHostUnset)
        ));
    }

    #[test]
    fn the_docker_host_error_names_both_prerequisites() {
        let message = HarnessError::DockerHostUnset.to_string();

        assert!(message.contains("DOCKER_HOST"));
        assert!(message.contains("systemctl --user enable --now podman.socket"));
    }

    #[test]
    fn the_database_url_is_built_from_host_and_port() {
        assert_eq!(
            fixture_database_url("127.0.0.1", 55432),
            "postgres://atlas:atlas@127.0.0.1:55432/atlas_dev"
        );
    }

    #[test]
    fn the_database_url_uses_the_loopback_hostname_verbatim() {
        assert_eq!(
            fixture_database_url("localhost", 5432),
            "postgres://atlas:atlas@localhost:5432/atlas_dev"
        );
    }
}
