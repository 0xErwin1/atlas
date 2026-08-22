pub mod action_id;
pub mod resource_path;
pub mod resource_ref;
pub mod segment;
pub mod selector;
pub mod specificity;

pub use action_id::{ActionId, ActionIdParseError};
pub use resource_path::{PathSegment, ResourcePath, ResourcePathParseError};
pub use resource_ref::{ResourceRef, ResourceRefParseError};
pub use segment::SegmentError;
pub use selector::{ResourceSelector, ResourceSelectorParseError, SelectorSegment};
pub use specificity::Specificity;

/// Generates string-conversion glue (`TryFrom<&str>`, `TryFrom<String>`,
/// `Serialize`, `Deserialize`) for a colon-delimited id type backed by a
/// `FromStr` implementation.
macro_rules! impl_string_conversions {
    ($ty:ty, $err:ty) => {
        impl TryFrom<&str> for $ty {
            type Error = $err;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl TryFrom<String> for $ty {
            type Error = $err;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                value.parse()
            }
        }

        impl serde::Serialize for $ty {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.collect_str(self)
            }
        }

        impl<'de> serde::Deserialize<'de> for $ty {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                value.parse().map_err(serde::de::Error::custom)
            }
        }
    };
}

pub(crate) use impl_string_conversions;
