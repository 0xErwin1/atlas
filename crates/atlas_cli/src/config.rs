#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used, clippy::panic))]

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::CliError;

const KEYRING_SERVICE: &str = "atlas";
const KEYRING_USER: &str = "token";

#[derive(Debug, Default, Deserialize, Serialize)]
pub(crate) struct Config {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) base_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
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

fn load_from_path(path: &Path) -> Result<Config, CliError> {
    match std::fs::read_to_string(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(CliError::Io(e)),
        Ok(content) => toml::from_str(&content)
            .map_err(|e| CliError::Config(format!("{}: {e}", path.display()))),
    }
}

/// Persists `config` to the default config file location.
///
/// Creates the parent directory with mode 0o700 and the file with mode 0o600.
/// Only `Some` fields are written; `None` fields are omitted entirely.
pub(crate) fn save(config: &Config) -> Result<(), CliError> {
    let path = config_path();
    save_to_path(config, &path)
}

/// Pure inner: serialize and write `config` to `path`.
///
/// The parent directory is created with mode 0o700 and the file with mode 0o600.
/// This is the security invariant for a file that may contain a plaintext token.
fn save_to_path(config: &Config, path: &Path) -> Result<(), CliError> {
    let content = toml::to_string(config)
        .map_err(|e| CliError::Config(format!("failed to serialize config: {e}")))?;

    if let Some(parent) = path.parent() {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)
            .map_err(CliError::Io)?;
    }

    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .map_err(CliError::Io)?;

        file.write_all(content.as_bytes()).map_err(CliError::Io)?;
    }

    Ok(())
}

/// Stores `token` in the OS keyring under the Atlas service entry.
///
/// Returns `CliError::Config` when the keyring backend is unavailable or rejects
/// the write. Callers are responsible for ensuring the token is NOT subsequently
/// written to the config file to avoid duplicate storage.
pub(crate) fn keyring_set_token(token: &str) -> Result<(), CliError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| CliError::Config(format!("failed to access keyring: {e}")))?;

    entry
        .set_password(token)
        .map_err(|e| CliError::Config(format!("failed to store token in keyring: {e}")))
}

/// Removes the Atlas token from the OS keyring.
///
/// A missing entry is treated as success so that `clear-token` is idempotent.
/// Other keyring errors (e.g. access denied) are surfaced as `CliError::Config`.
pub(crate) fn keyring_delete_token() -> Result<(), CliError> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
        .map_err(|e| CliError::Config(format!("failed to access keyring: {e}")))?;

    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(CliError::Config(format!(
            "failed to delete keyring entry: {e}"
        ))),
    }
}

/// Returns a masked representation of `token` safe to display to the user.
///
/// Tokens of 4 characters or fewer are fully masked. Longer tokens show only
/// the last 4 characters prefixed with `...`. The full secret is never included.
pub(crate) fn mask_token(token: &str) -> String {
    let char_count = token.chars().count();
    if char_count <= 4 {
        "****".to_owned()
    } else {
        let last4: String = token.chars().skip(char_count - 4).collect();
        format!("...{last4}")
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
pub(crate) fn try_keyring_token() -> Option<String> {
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

    // -----------------------------------------------------------------------
    // save_to_path tests
    // -----------------------------------------------------------------------

    #[test]
    fn save_to_path_round_trips_base_url_and_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas").join("config.toml");

        let config = Config {
            base_url: Some("https://round-trip.example.com".to_owned()),
            token: Some("round-trip-token".to_owned()),
        };

        save_to_path(&config, &path).unwrap();
        let loaded = load_from_path(&path).unwrap();

        assert_eq!(
            loaded.base_url.as_deref(),
            Some("https://round-trip.example.com")
        );
        assert_eq!(loaded.token.as_deref(), Some("round-trip-token"));
    }

    #[test]
    fn save_to_path_omits_none_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas").join("config.toml");

        let config = Config {
            base_url: Some("https://example.com".to_owned()),
            token: None,
        };

        save_to_path(&config, &path).unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();

        assert!(!raw.contains("token"), "None token must not be emitted");
    }

    #[test]
    fn save_to_path_creates_file_with_mode_0o600() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas").join("config.toml");

        save_to_path(&Config::default(), &path).unwrap();

        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "config file must have mode 0o600");
    }

    #[test]
    fn save_to_path_creates_parent_dir_with_mode_0o700() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas").join("config.toml");

        save_to_path(&Config::default(), &path).unwrap();

        let parent = path.parent().unwrap();
        let meta = std::fs::metadata(parent).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(mode & 0o777, 0o700, "config dir must have mode 0o700");
    }

    #[test]
    fn save_to_path_overwrites_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("atlas").join("config.toml");

        let first = Config {
            base_url: Some("https://first.example.com".to_owned()),
            token: Some("first-token".to_owned()),
        };
        save_to_path(&first, &path).unwrap();

        let second = Config {
            base_url: Some("https://second.example.com".to_owned()),
            token: None,
        };
        save_to_path(&second, &path).unwrap();

        let loaded = load_from_path(&path).unwrap();
        assert_eq!(
            loaded.base_url.as_deref(),
            Some("https://second.example.com")
        );
        assert!(
            loaded.token.is_none(),
            "token from first write must not persist"
        );
    }

    // -----------------------------------------------------------------------
    // mask_token tests
    // -----------------------------------------------------------------------

    #[test]
    fn mask_token_empty_returns_fixed_mask() {
        let masked = mask_token("");
        assert_eq!(masked, "****");
    }

    #[test]
    fn mask_token_short_token_returns_fixed_mask() {
        assert_eq!(mask_token("ab"), "****");
        assert_eq!(mask_token("abc"), "****");
    }

    #[test]
    fn mask_token_exactly_four_chars_returns_fixed_mask() {
        let masked = mask_token("abcd");
        assert_eq!(masked, "****");
    }

    #[test]
    fn mask_token_long_shows_last_four_only() {
        let token = "supersecret-ab12";
        let masked = mask_token(token);
        assert!(masked.ends_with("b12"), "must show last 4 chars");
        assert!(!masked.contains("supersecret"), "must not expose prefix");
    }

    #[test]
    fn mask_token_never_equals_full_token() {
        let token = "mytoken123456789";
        let masked = mask_token(token);
        assert_ne!(masked, token);
        assert!(!masked.contains(token));
    }

    #[test]
    fn mask_token_five_chars_shows_last_four() {
        let token = "abcde";
        let masked = mask_token(token);
        assert_eq!(masked, "...bcde");
    }
}
