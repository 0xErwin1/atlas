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

        for segment in value.split('/').skip(1) {
            if segment.is_empty() {
                return Err(RoutePathError::EmptySegment);
            }

            validate_segment_braces(segment)?;
        }

        Ok(Self(value.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn validate_segment_braces(segment: &str) -> Result<(), RoutePathError> {
    let mut depth = 0u8;
    let mut param_len = 0usize;

    for ch in segment.chars() {
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
    pub idempotent: bool,
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
}
