use std::fmt;
use std::str::FromStr;

use crate::ids::impl_string_conversions;

/// Where a satellite process runs relative to its owning component
/// (SHELL-INT-3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SatelliteMode {
    Local,
    Remote,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SatelliteModeParseError {
    #[error("unknown satellite mode `{value}`")]
    Unknown { value: String },
}

impl FromStr for SatelliteMode {
    type Err = SatelliteModeParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "local" => Ok(Self::Local),
            "remote" => Ok(Self::Remote),
            _ => Err(SatelliteModeParseError::Unknown {
                value: s.to_string(),
            }),
        }
    }
}

impl fmt::Display for SatelliteMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text = match self {
            Self::Local => "local",
            Self::Remote => "remote",
        };

        write!(f, "{text}")
    }
}

impl_string_conversions!(SatelliteMode, SatelliteModeParseError);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_every_variant_through_display_and_parse() {
        let cases = [
            (SatelliteMode::Local, "local"),
            (SatelliteMode::Remote, "remote"),
        ];

        for (mode, text) in cases {
            assert_eq!(mode.to_string(), text);
            assert_eq!(text.parse::<SatelliteMode>(), Ok(mode));
        }
    }

    #[test]
    fn rejects_unknown_values() {
        for value in ["Local", ""] {
            assert_eq!(
                value.parse::<SatelliteMode>(),
                Err(SatelliteModeParseError::Unknown {
                    value: value.to_string()
                })
            );
        }
    }
}
