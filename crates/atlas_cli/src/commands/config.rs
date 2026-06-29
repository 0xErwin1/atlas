#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use std::io::BufRead;

use clap::{Args, Subcommand};
use serde::Serialize;

use crate::config;
use crate::ctx::Ctx;
use crate::error::CliError;
use crate::output;

// ---------------------------------------------------------------------------
// ConfigArgs / ConfigCmd
// ---------------------------------------------------------------------------

/// Arguments holder for the `config` subcommand group.
#[derive(Args)]
pub(crate) struct ConfigArgs {
    #[command(subcommand)]
    pub(crate) command: ConfigCmd,
}

#[derive(Subcommand)]
pub(crate) enum ConfigCmd {
    /// Print the absolute path to the config file.
    Path,
    /// Show the resolved configuration and the source of each value.
    Show,
    /// Set the base URL saved in the config file.
    SetUrl(ConfigSetUrlArgs),
    /// Read a token from stdin and persist it to the config file or keyring.
    SetToken(ConfigSetTokenArgs),
    /// Remove the token from the config file and keyring.
    ClearToken,
}

/// Arguments for `atlas config set-url`.
#[derive(Args)]
pub(crate) struct ConfigSetUrlArgs {
    /// Base URL of the Atlas server (e.g. https://atlas.example.com).
    #[arg(index = 1)]
    pub(crate) url: String,
}

/// Arguments for `atlas config set-token`.
#[derive(Args)]
pub(crate) struct ConfigSetTokenArgs {
    /// Store the token in the OS keyring instead of the config file.
    #[arg(long)]
    pub(crate) keyring: bool,
}

/// Dispatches a parsed `ConfigCmd` to its handler.
pub(crate) async fn run(ctx: &Ctx, cmd: ConfigCmd) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Path => run_path(),
        ConfigCmd::Show => run_show(ctx),
        ConfigCmd::SetUrl(args) => run_set_url(args),
        ConfigCmd::SetToken(args) => run_set_token(args),
        ConfigCmd::ClearToken => run_clear_token(),
    }
}

// ---------------------------------------------------------------------------
// path
// ---------------------------------------------------------------------------

fn run_path() -> Result<(), CliError> {
    println!("{}", config::config_path().display());
    Ok(())
}

// ---------------------------------------------------------------------------
// show
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ConfigShowOutput {
    base_url: String,
    base_url_source: String,
    token_status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token_hint: Option<String>,
}

/// Pure computation: resolves show output from explicit sources.
///
/// Takes all environment/config values as parameters so this can be unit-tested
/// without real env-var mutation or keyring access.
fn compute_show_output(
    env_base_url: Option<&str>,
    env_token: Option<&str>,
    file_config: &config::Config,
    keyring_token: Option<&str>,
) -> ConfigShowOutput {
    let (base_url, base_url_source) = if let Some(v) = env_base_url {
        (v.to_owned(), "env ATLAS_BASE_URL".to_owned())
    } else if let Some(v) = file_config.base_url.as_deref() {
        (v.to_owned(), "config file".to_owned())
    } else {
        ("http://localhost:8080".to_owned(), "default".to_owned())
    };

    let (token_status, token_hint) = if env_token.is_some() {
        ("set (env ATLAS_TOKEN)".to_owned(), None)
    } else if let Some(t) = file_config.token.as_deref() {
        ("set (config file)".to_owned(), Some(config::mask_token(t)))
    } else if let Some(t) = keyring_token {
        ("set (keyring)".to_owned(), Some(config::mask_token(t)))
    } else {
        ("not set".to_owned(), None)
    };

    ConfigShowOutput {
        base_url,
        base_url_source,
        token_status,
        token_hint,
    }
}

fn resolve_show_output() -> Result<ConfigShowOutput, CliError> {
    let file_config = config::load()?;
    let env_base_url = std::env::var("ATLAS_BASE_URL").ok();
    let env_token = std::env::var("ATLAS_TOKEN").ok();
    let keyring_token = config::try_keyring_token();

    Ok(compute_show_output(
        env_base_url.as_deref(),
        env_token.as_deref(),
        &file_config,
        keyring_token.as_deref(),
    ))
}

fn run_show(ctx: &Ctx) -> Result<(), CliError> {
    let out = resolve_show_output()?;

    match ctx.output {
        output::OutputFormat::Json => output::print_json(&out),
        output::OutputFormat::Human => {
            println!(
                "base_url:  {}  (source: {})",
                out.base_url, out.base_url_source
            );

            let token_line = match out.token_hint.as_deref() {
                Some(hint) => format!("{}  [hint: {}]", out.token_status, hint),
                None => out.token_status.clone(),
            };
            println!("token:     {token_line}");

            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// set-url
// ---------------------------------------------------------------------------

fn run_set_url(args: ConfigSetUrlArgs) -> Result<(), CliError> {
    let mut cfg = config::load()?;
    cfg.base_url = Some(args.url.clone());
    config::save(&cfg)?;
    println!("base_url set to: {}", args.url);
    Ok(())
}

// ---------------------------------------------------------------------------
// set-token
// ---------------------------------------------------------------------------

/// Reads exactly one line from `reader`, trims trailing whitespace, and returns
/// it as the token string. Empty input (after trimming) is a Validation error.
///
/// The helper accepts any `BufRead` so callers can inject a controlled reader in
/// tests without touching real stdin. In production, pass `stdin.lock()`.
/// Reading from stdin rather than argv keeps the token out of shell history.
pub(crate) fn read_token_from_reader<R: BufRead>(reader: &mut R) -> Result<String, CliError> {
    let mut line = String::new();
    reader.read_line(&mut line).map_err(CliError::Io)?;

    let token = line.trim_end().to_owned();

    if token.is_empty() {
        return Err(CliError::Validation(
            "token must not be empty; pipe or type it into stdin".into(),
        ));
    }

    Ok(token)
}

fn run_set_token(args: ConfigSetTokenArgs) -> Result<(), CliError> {
    // Token read from stdin keeps it out of argv and shell history.
    let stdin = std::io::stdin();
    let mut reader = stdin.lock();
    let token = read_token_from_reader(&mut reader)?;

    if args.keyring {
        config::keyring_set_token(&token)?;

        // Ensure the secret lives in exactly one place.
        let mut cfg = config::load()?;
        if cfg.token.is_some() {
            cfg.token = None;
            config::save(&cfg)?;
        }

        println!("Token stored in OS keyring.");
    } else {
        let mut cfg = config::load()?;
        cfg.token = Some(token);
        config::save(&cfg)?;
        println!("Token saved to config file.");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// clear-token
// ---------------------------------------------------------------------------

fn run_clear_token() -> Result<(), CliError> {
    let mut cfg = config::load()?;
    cfg.token = None;
    config::save(&cfg)?;

    config::keyring_delete_token()?;

    println!("Token cleared from config file and keyring.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands};
    use clap::Parser;

    // -----------------------------------------------------------------------
    // Clap parse tests
    // -----------------------------------------------------------------------

    #[test]
    fn config_path_parses() {
        let cli = Cli::try_parse_from(["atlas", "config", "path"]).unwrap();
        if let Commands::Config(args) = cli.command {
            assert!(matches!(args.command, ConfigCmd::Path));
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_show_parses() {
        let cli = Cli::try_parse_from(["atlas", "config", "show"]).unwrap();
        if let Commands::Config(args) = cli.command {
            assert!(matches!(args.command, ConfigCmd::Show));
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_set_url_parses_url_arg() {
        let cli =
            Cli::try_parse_from(["atlas", "config", "set-url", "https://example.com"]).unwrap();
        if let Commands::Config(args) = cli.command {
            if let ConfigCmd::SetUrl(set_url) = args.command {
                assert_eq!(set_url.url, "https://example.com");
            } else {
                panic!("expected SetUrl");
            }
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_set_url_requires_url_argument() {
        let result = Cli::try_parse_from(["atlas", "config", "set-url"]);
        assert!(
            result.is_err(),
            "set-url without URL must fail at parse time"
        );
    }

    #[test]
    fn config_set_token_parses_without_keyring_flag() {
        let cli = Cli::try_parse_from(["atlas", "config", "set-token"]).unwrap();
        if let Commands::Config(args) = cli.command {
            if let ConfigCmd::SetToken(set_token) = args.command {
                assert!(!set_token.keyring, "--keyring must default to false");
            } else {
                panic!("expected SetToken");
            }
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_set_token_parses_with_keyring_flag() {
        let cli = Cli::try_parse_from(["atlas", "config", "set-token", "--keyring"]).unwrap();
        if let Commands::Config(args) = cli.command {
            if let ConfigCmd::SetToken(set_token) = args.command {
                assert!(set_token.keyring, "--keyring must be true when supplied");
            } else {
                panic!("expected SetToken");
            }
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_clear_token_parses() {
        let cli = Cli::try_parse_from(["atlas", "config", "clear-token"]).unwrap();
        if let Commands::Config(args) = cli.command {
            assert!(matches!(args.command, ConfigCmd::ClearToken));
        } else {
            panic!("expected Config");
        }
    }

    #[test]
    fn config_requires_a_subcommand() {
        let result = Cli::try_parse_from(["atlas", "config"]);
        assert!(result.is_err(), "config without subcommand must fail");
    }

    // -----------------------------------------------------------------------
    // read_token_from_reader tests
    // -----------------------------------------------------------------------

    #[test]
    fn read_token_empty_input_is_validation_error() {
        let mut reader = std::io::Cursor::new(b"");
        let result = read_token_from_reader(&mut reader);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Validation(_)));
    }

    #[test]
    fn read_token_newline_only_is_validation_error() {
        let mut reader = std::io::Cursor::new(b"\n");
        let result = read_token_from_reader(&mut reader);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Validation(_)));
    }

    #[test]
    fn read_token_whitespace_only_is_validation_error() {
        let mut reader = std::io::Cursor::new(b"   \n");
        let result = read_token_from_reader(&mut reader);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Validation(_)));
    }

    #[test]
    fn read_token_trims_trailing_newline() {
        let mut reader = std::io::Cursor::new(b"mytoken\n");
        let token = read_token_from_reader(&mut reader).unwrap();
        assert_eq!(token, "mytoken");
    }

    #[test]
    fn read_token_trims_trailing_whitespace_and_newline() {
        let mut reader = std::io::Cursor::new(b"mytoken   \n");
        let token = read_token_from_reader(&mut reader).unwrap();
        assert_eq!(token, "mytoken");
    }

    #[test]
    fn read_token_no_newline_returns_full_line() {
        let mut reader = std::io::Cursor::new(b"secret-api-key-123");
        let token = read_token_from_reader(&mut reader).unwrap();
        assert_eq!(token, "secret-api-key-123");
    }

    #[test]
    fn read_token_reads_only_first_line() {
        let mut reader = std::io::Cursor::new(b"first-line\nsecond-line\n");
        let token = read_token_from_reader(&mut reader).unwrap();
        assert_eq!(token, "first-line");
    }

    // -----------------------------------------------------------------------
    // compute_show_output tests — pure, no env/keyring side effects
    // -----------------------------------------------------------------------

    fn empty_config() -> config::Config {
        config::Config::default()
    }

    #[test]
    fn show_base_url_defaults_when_no_sources() {
        let out = compute_show_output(None, None, &empty_config(), None);
        assert_eq!(out.base_url, "http://localhost:8080");
        assert_eq!(out.base_url_source, "default");
    }

    #[test]
    fn show_base_url_from_env_wins_over_file() {
        let cfg = config::Config {
            base_url: Some("https://from-file.com".to_owned()),
            token: None,
        };
        let out = compute_show_output(Some("https://from-env.com"), None, &cfg, None);
        assert_eq!(out.base_url, "https://from-env.com");
        assert_eq!(out.base_url_source, "env ATLAS_BASE_URL");
    }

    #[test]
    fn show_base_url_from_file_when_no_env() {
        let cfg = config::Config {
            base_url: Some("https://from-file.com".to_owned()),
            token: None,
        };
        let out = compute_show_output(None, None, &cfg, None);
        assert_eq!(out.base_url, "https://from-file.com");
        assert_eq!(out.base_url_source, "config file");
    }

    #[test]
    fn show_token_not_set_when_no_sources() {
        let out = compute_show_output(None, None, &empty_config(), None);
        assert_eq!(out.token_status, "not set");
        assert!(out.token_hint.is_none());
    }

    #[test]
    fn show_token_from_env() {
        let out = compute_show_output(None, Some("env-tok"), &empty_config(), None);
        assert_eq!(out.token_status, "set (env ATLAS_TOKEN)");
        // Env token is NOT hinted — we don't have the value at this point
        assert!(out.token_hint.is_none());
    }

    #[test]
    fn show_token_from_config_file_uses_masked_hint() {
        let cfg = config::Config {
            base_url: None,
            token: Some("supersecret-ab12".to_owned()),
        };
        let out = compute_show_output(None, None, &cfg, None);
        assert_eq!(out.token_status, "set (config file)");
        let hint = out.token_hint.unwrap();
        // hint must not contain the full token
        assert!(!hint.contains("supersecret"), "hint must not expose prefix");
        assert!(hint.ends_with("b12"), "hint must end with last 4 chars");
    }

    #[test]
    fn show_token_from_keyring_uses_masked_hint() {
        let out = compute_show_output(None, None, &empty_config(), Some("keyring-tok-xyz9"));
        assert_eq!(out.token_status, "set (keyring)");
        let hint = out.token_hint.unwrap();
        assert!(!hint.contains("keyring-tok"), "hint must not expose prefix");
        assert!(hint.ends_with("xyz9"), "hint must end with last 4 chars");
    }

    #[test]
    fn show_token_env_wins_over_file() {
        let cfg = config::Config {
            base_url: None,
            token: Some("file-tok".to_owned()),
        };
        let out = compute_show_output(None, Some("env-tok"), &cfg, None);
        assert_eq!(out.token_status, "set (env ATLAS_TOKEN)");
    }

    #[test]
    fn show_token_file_wins_over_keyring() {
        let cfg = config::Config {
            base_url: None,
            token: Some("file-tok".to_owned()),
        };
        let out = compute_show_output(None, None, &cfg, Some("keyring-tok"));
        assert_eq!(out.token_status, "set (config file)");
    }

    #[test]
    fn show_token_secret_never_appears_verbatim_in_hint() {
        let raw_token = "super-secret-token-abc123";
        let cfg = config::Config {
            base_url: None,
            token: Some(raw_token.to_owned()),
        };
        let out = compute_show_output(None, None, &cfg, None);
        let hint = out.token_hint.unwrap();
        assert!(
            !hint.contains(raw_token),
            "raw token must not appear verbatim in hint"
        );
    }
}
