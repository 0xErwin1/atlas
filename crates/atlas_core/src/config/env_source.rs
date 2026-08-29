//! Environment-backed configuration sources (SHELL-CFG-1). `EnvSource` is
//! the dyn-safe seam components load configuration through; `ProcessEnv` is
//! the production implementation, and any `Fn(&str) -> Option<String>`
//! closure works too, via the blanket impl below.

/// A source of configuration values keyed by name.
///
/// Dyn-safe by design: components load configuration through `&dyn
/// EnvSource`, so tests can substitute a closure or a map without a wrapper
/// type.
pub trait EnvSource {
    /// Returns the value for `key`, or `None` if it is absent.
    fn get(&self, key: &str) -> Option<String>;
}

impl<F> EnvSource for F
where
    F: Fn(&str) -> Option<String>,
{
    fn get(&self, key: &str) -> Option<String> {
        self(key)
    }
}

/// Reads configuration from the process environment.
///
/// An empty-string value is treated as absent, matching V1's `nonempty`
/// semantics: an environment variable set to `""` is indistinguishable from
/// one that was never set.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        nonempty(std::env::var(key).ok())
    }
}

/// Collapses an empty-string value to `None`.
fn nonempty(value: Option<String>) -> Option<String> {
    value.filter(|v| !v.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct MapEnv(HashMap<String, String>);

    impl EnvSource for MapEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
    }

    #[test]
    fn closure_satisfies_env_source_via_blanket_impl() {
        let source = |k: &str| if k == "FOO" { Some("bar".into()) } else { None };

        assert_eq!((&source as &dyn EnvSource).get("FOO"), Some("bar".into()));
        assert_eq!((&source as &dyn EnvSource).get("MISSING"), None);
    }

    #[test]
    fn map_env_manual_impl_matches_closure_answers() {
        let closure = |k: &str| if k == "FOO" { Some("bar".into()) } else { None };
        let map_env = MapEnv(HashMap::from([("FOO".to_string(), "bar".to_string())]));

        assert_eq!(closure.get("FOO"), map_env.get("FOO"));
        assert_eq!(closure.get("MISSING"), map_env.get("MISSING"));
    }

    #[test]
    fn dyn_env_source_coercion_from_both() {
        let closure = |k: &str| if k == "FOO" { Some("bar".into()) } else { None };
        let map_env = MapEnv(HashMap::from([("FOO".to_string(), "bar".to_string())]));

        let sources: Vec<Box<dyn EnvSource>> = vec![Box::new(closure), Box::new(map_env)];

        for source in &sources {
            assert_eq!(source.get("FOO"), Some("bar".to_string()));
        }
    }

    #[test]
    fn nonempty_collapses_empty_string_to_none() {
        let cases = [
            (Some(String::new()), None),
            (Some("x".to_string()), Some("x".to_string())),
            (None, None),
        ];

        for (input, expected) in cases {
            assert_eq!(nonempty(input), expected);
        }
    }

    #[test]
    fn process_env_returns_none_for_certainly_absent_key() {
        let value = ProcessEnv.get("ATLAS_CORE_CONFIG_TEST_KEY_DOES_NOT_EXIST");

        assert_eq!(value, None);
    }
}
