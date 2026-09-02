use std::fmt;
use std::str::FromStr;

use crate::ids::ActionId;
use crate::ids::impl_string_conversions;

/// HTTP method of a declared route (SHELL-REG-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
    Head,
    Options,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HttpMethodParseError {
    #[error("unknown http method `{value}`")]
    Unknown { value: String },
}

impl FromStr for HttpMethod {
    type Err = HttpMethodParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "GET" => Ok(Self::Get),
            "POST" => Ok(Self::Post),
            "PUT" => Ok(Self::Put),
            "PATCH" => Ok(Self::Patch),
            "DELETE" => Ok(Self::Delete),
            "HEAD" => Ok(Self::Head),
            "OPTIONS" => Ok(Self::Options),
            _ => Err(HttpMethodParseError::Unknown {
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Get => "GET",
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Patch => "PATCH",
            Self::Delete => "DELETE",
            Self::Head => "HEAD",
            Self::Options => "OPTIONS",
        };

        write!(f, "{text}")
    }
}

impl_string_conversions!(HttpMethod, HttpMethodParseError);

/// Path relative to the component namespace: `/tasks/{task_id}`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RoutePath(String);

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RoutePathError {
    #[error("route path is empty")]
    Empty,
    #[error("route path must start with `/`")]
    MissingLeadingSlash,
    #[error("route path must not end with `/`")]
    TrailingSlash,
    #[error("route path has an empty segment")]
    EmptySegment,
    #[error("route path contains illegal character `{ch}`")]
    IllegalCharacter { ch: char },
    #[error("route path has an unbalanced `{{`/`}}`")]
    UnbalancedBrace,
    #[error("route path has an empty `{{}}` parameter")]
    EmptyParameter,
}

impl RoutePath {
    pub fn new(value: &str) -> Result<Self, RoutePathError> {
        if value.is_empty() {
            return Err(RoutePathError::Empty);
        }

        if !value.starts_with('/') {
            return Err(RoutePathError::MissingLeadingSlash);
        }

        if value.len() > 1 && value.ends_with('/') {
            return Err(RoutePathError::TrailingSlash);
        }

        let mut segments = value.split('/').skip(1).peekable();

        while let Some(segment) = segments.next() {
            if segment.is_empty() {
                return Err(RoutePathError::EmptySegment);
            }

            let is_last_segment = segments.peek().is_none();
            validate_segment_braces(segment, is_last_segment)?;
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Validates one path segment's character set and brace balance.
///
/// `is_last_segment` gates the one dot-acceptance rule this function knows
/// (`v2-e3-s4`, D3): a `.` is legal only in the last segment of the path,
/// at most once, and never as that segment's first or last character. Every
/// earlier segment, and every character inside a `{param}` marker
/// regardless of segment position, keeps the original character set
/// (alphanumeric, `-`, `_`).
fn validate_segment_braces(segment: &str, is_last_segment: bool) -> Result<(), RoutePathError> {
    let mut depth = 0u8;
    let mut param_len = 0usize;
    let mut dot_seen = false;
    let last_char_index = segment.chars().count().saturating_sub(1);

    for (index, ch) in segment.chars().enumerate() {
        match ch {
            '{' => {
                if depth == 1 {
                    return Err(RoutePathError::UnbalancedBrace);
                }
                depth = 1;
                param_len = 0;
            }
            '}' => {
                if depth == 0 {
                    return Err(RoutePathError::UnbalancedBrace);
                }
                if param_len == 0 {
                    return Err(RoutePathError::EmptyParameter);
                }
                depth = 0;
            }
            ch if depth == 1 => {
                param_len += 1;
                if !ch.is_ascii_alphanumeric() && ch != '_' {
                    return Err(RoutePathError::IllegalCharacter { ch });
                }
            }
            '.' if is_last_segment => {
                if dot_seen || index == 0 || index == last_char_index {
                    return Err(RoutePathError::IllegalCharacter { ch: '.' });
                }
                dot_seen = true;
            }
            ch if !ch.is_ascii_alphanumeric() && ch != '-' && ch != '_' => {
                return Err(RoutePathError::IllegalCharacter { ch });
            }
            _ => {}
        }
    }

    if depth != 0 {
        return Err(RoutePathError::UnbalancedBrace);
    }

    Ok(())
}

impl FromStr for RoutePath {
    type Err = RoutePathError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for RoutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl_string_conversions!(RoutePath, RoutePathError);

/// A declared HTTP route on a component's API surface (SHELL-REG-1, SHELL-CLI-2, SHELL-API-3).
#[derive(Debug)]
pub struct RouteDeclaration {
    pub method: HttpMethod,
    pub path: RoutePath,
    pub operation_id: String,
    pub action: Option<ActionId>,
    /// Whether this route honors a client-supplied `Idempotency-Key` request
    /// header and dedupes on it (`v2-e3-s3`, D8) — NOT HTTP-method
    /// idempotence. Pre-S3, this field meant "repeating this request has no
    /// additional effect" (true for most `GET`/`PUT`/`DELETE`); S3
    /// re-derives every one of the 210 entries under this one rule instead.
    ///
    /// **Rule**: `true` iff `method == HttpMethod::Post` AND the operation
    /// creates a new resource or triggers a one-shot side effect (sending an
    /// email, enqueueing a background job, minting a credential) that a
    /// client-visible retry would wrongly duplicate. `false` for every
    /// `GET`, `PUT`, and `DELETE` route regardless of side effects — the
    /// header has no effect on these methods. `false` for every `PATCH`:
    /// Atlas's `PATCH` routes are field-level updates on an existing
    /// resource identified by path, which already produce the same end
    /// state on retry at the application level. A `POST` route that only
    /// performs a read/search with a request body (not a mutation) is also
    /// `false`. A route with no authenticated principal at request time
    /// (login, activation, an HMAC-verified webhook ingest) is `false`
    /// regardless of its mutation shape — the mechanism scopes dedup to
    /// `principal_id`, which does not exist before `require_authn` runs, and
    /// these routes sit outside `require_authn` entirely.
    ///
    /// A route whose response carries a one-shot secret (an API key token,
    /// an activation link, password-reset material) is never `true`,
    /// regardless of its mutation shape — dedup requires storing the
    /// response, and a one-shot plaintext secret sitting in the store for
    /// the retention window defeats "hashed/shown once" handling.
    ///
    /// A route declared `false` under this rule ignores the
    /// `Idempotency-Key` header if presented — it is never rejected.
    ///
    /// A route whose request body is a streamed upload is also `false`
    /// regardless of its mutation shape (ORCHESTRATOR DECISION, 2026-09-02,
    /// `R4-upload-bodies-buffered-in-memory`): dedup requires buffering the
    /// whole body in memory to compute the fingerprint, and buffering a
    /// streamed upload undoes the reason it streams in the first place.
    /// Streaming-safe fingerprinting is future work, not part of this rule.
    ///
    /// Per-route judgment for every POST whose classification is not a
    /// mechanical `create_*`/`upload_*` name match:
    /// `docs/reg5-idempotent-judgment.md`.
    ///
    /// A route's 5xx handling once `idempotent` is `true` is itself split in
    /// two (`v2-e3-s3` PR4 scoped correction, D6): a route named in
    /// `router_audit::ONE_SHOT_IDEMPOTENT_ROUTES` (a one-shot side effect — enqueuing
    /// a background job — with no domain uniqueness check to catch a
    /// duplicate) stores a 5xx briefly and replays it within the window; every
    /// other `idempotent: true` route (an ordinary create, whose duplicate a
    /// domain check already catches) releases the row on a 5xx so the retry
    /// re-executes. See `docs/reg5-idempotent-judgment.md`'s "5xx policy"
    /// section.
    pub idempotent: bool,
    /// Whether this route is mounted with no auth layer at all (`v2-e3-s4`,
    /// D7) — a promotion of `tests/support/route_matrix.rs`'s existing,
    /// already-tested computation (membership in
    /// `platform_route_paths().chain(custos_route_paths()).chain(
    /// acta_route_paths())`, the union of every component's public
    /// sub-router accessor) from a test-only helper into the registry
    /// itself. A pure structural lookup against already-correct code, set
    /// once per logical route in `reg5.rs` — not a per-route judgment call.
    pub is_public: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn http_method_round_trips_every_variant() {
        let cases = [
            (HttpMethod::Get, "GET"),
            (HttpMethod::Post, "POST"),
            (HttpMethod::Put, "PUT"),
            (HttpMethod::Patch, "PATCH"),
            (HttpMethod::Delete, "DELETE"),
            (HttpMethod::Head, "HEAD"),
            (HttpMethod::Options, "OPTIONS"),
        ];

        for (method, text) in cases {
            assert_eq!(method.to_string(), text);
            assert_eq!(text.parse::<HttpMethod>(), Ok(method));
        }
    }

    #[test]
    fn http_method_rejects_lowercase() {
        assert_eq!(
            "get".parse::<HttpMethod>(),
            Err(HttpMethodParseError::Unknown {
                value: "get".to_string()
            })
        );
    }

    #[test]
    fn route_path_accepts_parameterized_path() {
        let path = RoutePath::new("/tasks/{task_id}").expect("valid route path");
        assert_eq!(path.as_str(), "/tasks/{task_id}");
    }

    #[test]
    fn route_path_rejects_every_error_variant() {
        let cases = [
            ("", RoutePathError::Empty),
            ("tasks", RoutePathError::MissingLeadingSlash),
            ("/tasks/", RoutePathError::TrailingSlash),
            ("/tasks//1", RoutePathError::EmptySegment),
            ("/tasks/ 1", RoutePathError::IllegalCharacter { ch: ' ' }),
            ("/tasks/{id", RoutePathError::UnbalancedBrace),
            ("/tasks/id}", RoutePathError::UnbalancedBrace),
            ("/tasks/{}", RoutePathError::EmptyParameter),
        ];

        for (value, expected) in cases {
            assert_eq!(RoutePath::new(value), Err(expected));
        }
    }

    /// `v2-e3-s4` D3: accepted shapes under the widened validator.
    #[test]
    fn route_path_accepts_one_interior_dot_in_the_final_segment() {
        let cases = [
            "/openapi.json",
            "/scalar",
            "/api/v2/openapi.json",
            "/documents/export.json",
        ];

        for value in cases {
            assert_eq!(
                RoutePath::new(value).map(|path| path.as_str().to_string()),
                Ok(value.to_string()),
                "{value} must be accepted by the widened validator"
            );
        }
    }

    /// `v2-e3-s4` D3: every shape the widening must keep rejecting, still via
    /// `IllegalCharacter { ch: '.' }` (no new error variant, T1.4).
    #[test]
    fn route_path_rejects_every_dot_shape_outside_the_widened_rule() {
        let cases = [
            "/.json",
            "/openapi.",
            "/open.api.json",
            "/open.api/json",
            "/{id.ext}",
        ];

        for value in cases {
            assert_eq!(
                RoutePath::new(value),
                Err(RoutePathError::IllegalCharacter { ch: '.' }),
                "{value} must still be rejected after the widening"
            );
        }
    }
}
