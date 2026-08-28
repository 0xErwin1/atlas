use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

/// The role a component plays in the platform (SHELL-REG-1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ComponentKind {
    Product,
    PlatformService,
    Module,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ComponentKindParseError {
    #[error("unknown component kind `{value}`")]
    Unknown { value: String },
}

impl FromStr for ComponentKind {
    type Err = ComponentKindParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "product" => Ok(Self::Product),
            "platform-service" => Ok(Self::PlatformService),
            "module" => Ok(Self::Module),
            _ => Err(ComponentKindParseError::Unknown {
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for ComponentKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Product => "product",
            Self::PlatformService => "platform-service",
            Self::Module => "module",
        };

        write!(f, "{text}")
    }
}

impl_string_conversions!(ComponentKind, ComponentKindParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_variant_through_display_and_parse() {
        let cases = [
            (ComponentKind::Product, "product"),
            (ComponentKind::PlatformService, "platform-service"),
            (ComponentKind::Module, "module"),
        ];

        for (kind, text) in cases {
            assert_eq!(kind.to_string(), text);
            assert_eq!(text.parse::<ComponentKind>(), Ok(kind));
        }
    }

    #[test]
    fn rejects_unknown_values() {
        for value in ["Product", ""] {
            assert_eq!(
                value.parse::<ComponentKind>(),
                Err(ComponentKindParseError::Unknown {
                    value: value.to_string()
                })
            );
        }
    }
}
