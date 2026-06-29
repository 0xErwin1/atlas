#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::CliError;

const KEYRING_SERVICE: &str = "atlas";
const KEYRING_USER: &str = "token";

#[derive(Debug, Default, Deserialize)]
pub(crate) struct Config {
    pub(crate) base_url: Option<String>,
    pub(crate) token: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Resolved {
    pub(crate) base_url: String,
    pub(crate) token: Option<String>,
}

/// Returns the path to the Atlas CLI config file.
///
/// Respects `$XDG_CONFIG_HOME` when set; otherwise falls back to `$HOME/.config`.
/// Does not depend on any third-party `dirs` crate.
pub(crate) fn config_path() -> PathBuf {
    config_path_from_env(
        std::env::var("XDG_CONFIG_HOME").ok().as_deref(),
        std::env::var("HOME").ok().as_deref(),
    )
}

/// Pure inner function — exposed only for unit testing.
fn config_path_from_env(xdg_config_home: Option<&str>, home: Option<&str>) -> PathBuf {
    let base = xdg_config_home
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(home.unwrap_or("")).join(".config"));

    base.join("atlas").join("config.toml")
}

/// Reads the config file from the default path.
///
/// A missing config file is not an error; it returns `Config::default()`.
/// A file that exists but contains invalid TOML returns `CliError::Config`.
pub(crate) fn load() -> Result<Config, CliError> {
    let path = config_path();
    load_from_path(&path)
}

fn load_from_path(path: &std::path::Path) -> Result<Config, CliError> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(CliError::Io(e)),
        Ok(content) => toml::from_str(&content)
            .map_err(|e| CliError::Config(format!("{}: {e}", path.display()))),
    }
}

/// Resolves the effective base URL and token from all precedence sources.
///
/// Precedence (highest to lowest) for `base_url`:
///   `--base-url` flag → `ATLAS_BASE_URL` env → config file → `http://localhost:8080`.
///
/// Precedence for `token`:
///   `--token` flag → `ATLAS_TOKEN` env → config file → keyring → `None`.
///
/// Keyring errors (no entry, unavailable backend, …) are silently treated as
/// `None` so keychain-less environments compile and run without issue.
pub(crate) fn resolve(cli_base: Option<&str>, cli_token: Option<&str>, file: &Config) -> Resolved {
    let r = resolve_with_env(
        cli_base,
        cli_token,
        std::env::var("ATLAS_BASE_URL").ok().as_deref(),
        std::env::var("ATLAS_TOKEN").ok().as_deref(),
        file,
    );

    if r.token.is_some() {
        return r;
    }

    Resolved {
        base_url: r.base_url,
        token: apply_keyring_fallback(None, try_keyring_token()),
    }
}

/// Attempts to retrieve the token from the system keyring.
///
/// Any keyring error (including `NoEntry` and absent backends) silently yields
/// `None` — keychain-less environments must not fail.
fn try_keyring_token() -> Option<String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .ok()
        .and_then(|entry| entry.get_password().ok())
}

/// Pure combining function: returns `current` if set, otherwise `keyring_token`.
///
/// Exists as a testable unit so callers can verify the composition logic
/// without depending on the presence or absence of a real keyring entry.
fn apply_keyring_fallback(
    current: Option<String>,
    keyring_token: Option<String>,
) -> Option<String> {
    current.or(keyring_token)
}

/// Pure inner: resolves base_url and token from explicit sources (no keyring).
fn resolve_with_env(
    cli_base: Option<&str>,
    cli_token: Option<&str>,
    env_base: Option<&str>,
    env_token: Option<&str>,
    file: &Config,
) -> Resolved {
    let base_url = cli_base
        .or(env_base)
        .map(str::to_owned)
        .or_else(|| file.base_url.clone())
        .unwrap_or_else(|| "http://localhost:8080".to_owned());

    let token = cli_token
        .or(env_token)
        .map(str::to_owned)
        .or_else(|| file.token.clone());

    Resolved { base_url, token }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp_config(content: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        (dir, path)
    }

    // ---------------------------------------------------------------------------
    // config_path_from_env tests (pure — no env mutation)
    // ---------------------------------------------------------------------------

    #[test]
    fn config_path_uses_xdg_config_home_when_provided() {
        let p = config_path_from_env(Some("/tmp/test-xdg"), None);
        assert_eq!(p, PathBuf::from("/tmp/test-xdg/atlas/config.toml"));
    }

    #[test]
    fn config_path_falls_back_to_home_dot_config() {
        let p = config_path_from_env(None, Some("/home/testuser"));
        assert_eq!(p, PathBuf::from("/home/testuser/.config/atlas/config.toml"));
    }

    #[test]
    fn config_path_xdg_wins_over_home() {
        let p = config_path_from_env(Some("/xdg"), Some("/home/user"));
        assert_eq!(p, PathBuf::from("/xdg/atlas/config.toml"));
    }

    // ---------------------------------------------------------------------------
    // load_from_path tests
    // ---------------------------------------------------------------------------

    #[test]
    fn load_missing_file_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent.toml");
        let result = load_from_path(&path);
        assert!(result.is_ok(), "missing file must not be an error");
        let cfg = result.unwrap();
        assert!(cfg.base_url.is_none());
        assert!(cfg.token.is_none());
    }

    #[test]
    fn load_valid_toml_parses_fields() {
        let (_dir, path) = write_temp_config(
            r#"
base_url = "https://example.com"
token    = "secret-tok"
"#,
        );
        let cfg = load_from_path(&path).unwrap();
        assert_eq!(cfg.base_url.as_deref(), Some("https://example.com"));
        assert_eq!(cfg.token.as_deref(), Some("secret-tok"));
    }

    #[test]
    fn load_malformed_toml_returns_config_error() {
        let (_dir, path) = write_temp_config("this is not valid toml ===");
        let result = load_from_path(&path);
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), CliError::Config(_)));
    }

    // ---------------------------------------------------------------------------
    // resolve_with_env tests (pure — no env mutation)
    // ---------------------------------------------------------------------------

    fn empty_config() -> Config {
        Config::default()
    }

    fn config_with_base(base: &str) -> Config {
        Config {
            base_url: Some(base.to_owned()),
            token: None,
        }
    }

    fn config_with_token(tok: &str) -> Config {
        Config {
            base_url: None,
            token: Some(tok.to_owned()),
        }
    }

    #[test]
    fn resolve_flag_wins_over_env_for_base_url() {
        let cfg = config_with_base("https://from-file.com");
        let r = resolve_with_env(
            Some("https://from-flag.com"),
            None,
            Some("https://from-env.com"),
            None,
            &cfg,
        );
        assert_eq!(r.base_url, "https://from-flag.com");
    }

    #[test]
    fn resolve_env_wins_over_file_for_base_url() {
        let cfg = config_with_base("https://from-file.com");
        let r = resolve_with_env(None, None, Some("https://from-env.com"), None, &cfg);
        assert_eq!(r.base_url, "https://from-env.com");
    }

    #[test]
    fn resolve_file_wins_over_default_for_base_url() {
        let cfg = config_with_base("https://from-file.com");
        let r = resolve_with_env(None, None, None, None, &cfg);
        assert_eq!(r.base_url, "https://from-file.com");
    }

    #[test]
    fn resolve_default_base_url_when_nothing_provided() {
        let r = resolve_with_env(None, None, None, None, &empty_config());
        assert_eq!(r.base_url, "http://localhost:8080");
    }

    #[test]
    fn resolve_flag_wins_over_env_for_token() {
        let cfg = config_with_token("file-tok");
        let r = resolve_with_env(None, Some("flag-tok"), None, Some("env-tok"), &cfg);
        assert_eq!(r.token.as_deref(), Some("flag-tok"));
    }

    #[test]
    fn resolve_env_wins_over_file_for_token() {
        let cfg = config_with_token("file-tok");
        let r = resolve_with_env(None, None, None, Some("env-tok"), &cfg);
        assert_eq!(r.token.as_deref(), Some("env-tok"));
    }

    #[test]
    fn resolve_file_token_used_when_no_flag_or_env() {
        let cfg = config_with_token("file-tok");
        let r = resolve_with_env(None, None, None, None, &cfg);
        assert_eq!(r.token.as_deref(), Some("file-tok"));
    }

    #[test]
    fn resolve_token_is_none_when_nothing_provided() {
        let r = resolve_with_env(None, None, None, None, &empty_config());
        assert!(r.token.is_none());
    }

    #[test]
    fn resolve_atlas_base_url_env_param_is_honored() {
        let r = resolve_with_env(
            None,
            None,
            Some("https://env-server.com"),
            None,
            &empty_config(),
        );
        assert_eq!(r.base_url, "https://env-server.com");
    }

    // -----------------------------------------------------------------------
    // WU-35: keyring fallback tests (pure functions only — no real keyring)
    // -----------------------------------------------------------------------

    #[test]
    fn apply_keyring_fallback_all_none_returns_none() {
        let result = apply_keyring_fallback(None, None);
        assert!(result.is_none(), "both absent must yield None — no panic");
    }

    #[test]
    fn apply_keyring_fallback_keyring_none_entry_returns_none() {
        // Simulates keyring returning None (no entry or unavailable backend).
        let result = apply_keyring_fallback(None, None);
        assert!(result.is_none());
    }

    #[test]
    fn apply_keyring_fallback_uses_current_when_set() {
        let result =
            apply_keyring_fallback(Some("from-file".to_owned()), Some("from-kr".to_owned()));
        assert_eq!(result.as_deref(), Some("from-file"));
    }

    #[test]
    fn apply_keyring_fallback_uses_keyring_when_current_absent() {
        let result = apply_keyring_fallback(None, Some("kr-tok".to_owned()));
        assert_eq!(result.as_deref(), Some("kr-tok"));
    }

    #[test]
    fn try_keyring_token_does_not_panic_in_test_env() {
        // Verifies the keyring call does not panic regardless of the
        // environment (no entry → None, no backend → None, has entry → Some).
        drop(try_keyring_token());
    }
}
