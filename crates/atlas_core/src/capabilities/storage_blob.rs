use std::fmt;
use std::str::FromStr;

use async_trait::async_trait;
use bytes::Bytes;

use crate::ids::segment::{SegmentError, validate_segment};

use super::error::CapabilityError;

/// The maximum length, in bytes, of a `BlobKey`.
pub const MAX_BLOB_KEY_BYTES: usize = 512;

/// A flat, traversal-safe key identifying a blob within a `StorageBlob`.
///
/// Keys have no path separators: each `StorageBlob` implementation owns its
/// own fan-out layout on top of a flat key.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobKey(String);

/// The error returned when a string is not a valid `BlobKey`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BlobKeyError {
    /// The key failed the shared segment rules (empty, or contains `:`,
    /// `/`, or `*`).
    #[error("invalid blob key: {source}")]
    Segment {
        #[source]
        source: SegmentError,
    },
    /// The key is exactly `.` or `..`, a filesystem relative-path marker.
    #[error("blob key `{value}` is a relative path marker")]
    RelativePath {
        /// The rejected value.
        value: String,
    },
    /// The key contains a character outside the allowed charset (a
    /// backslash or an ASCII control character).
    #[error("blob key contains disallowed character `{ch}`")]
    Charset {
        /// The disallowed character found.
        ch: char,
    },
    /// The key exceeds `MAX_BLOB_KEY_BYTES`.
    #[error("blob key exceeds the maximum length of {max} bytes")]
    TooLong {
        /// The maximum allowed length, in bytes.
        max: usize,
    },
}

impl BlobKey {
    /// Validates and wraps `value` as a `BlobKey`.
    pub fn new(value: &str) -> Result<Self, BlobKeyError> {
        validate_segment(value).map_err(|source| BlobKeyError::Segment { source })?;

        if value == "." || value == ".." {
            return Err(BlobKeyError::RelativePath {
                value: value.to_string(),
            });
        }

        if let Some(ch) = value.chars().find(|ch| *ch == '\\' || ch.is_control()) {
            return Err(BlobKeyError::Charset { ch });
        }

        if value.len() > MAX_BLOB_KEY_BYTES {
            return Err(BlobKeyError::TooLong {
                max: MAX_BLOB_KEY_BYTES,
            });
        }

        Ok(Self(value.to_string()))
    }

    /// Returns the underlying string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for BlobKey {
    type Err = BlobKeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl fmt::Display for BlobKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Content-addressed or key-addressed binary storage, keyed by `BlobKey`.
#[async_trait]
pub trait StorageBlob: Send + Sync {
    /// Writes `data` under `key`, overwriting any existing value.
    async fn put(&self, key: &BlobKey, data: Bytes) -> Result<(), CapabilityError>;

    /// Reads the bytes stored under `key`.
    ///
    /// Returns `CapabilityError::NotFound` when `key` was never written.
    async fn get(&self, key: &BlobKey) -> Result<Bytes, CapabilityError>;

    /// Returns `true` if `key` currently has a stored value.
    async fn exists(&self, key: &BlobKey) -> Result<bool, CapabilityError>;

    /// Removes the value stored under `key`, if any.
    async fn delete(&self, key: &BlobKey) -> Result<(), CapabilityError>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::test_support::block_on;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[test]
    fn accepts_a_flat_key() {
        assert!(BlobKey::new("digest-abc123").is_ok());
    }

    #[test]
    fn rejects_invalid_inputs() {
        let long_key = "a".repeat(MAX_BLOB_KEY_BYTES + 1);
        let cases: Vec<(&str, BlobKeyError)> = vec![
            (
                "",
                BlobKeyError::Segment {
                    source: SegmentError::Empty,
                },
            ),
            (
                ".",
                BlobKeyError::RelativePath {
                    value: ".".to_string(),
                },
            ),
            (
                "..",
                BlobKeyError::RelativePath {
                    value: "..".to_string(),
                },
            ),
            (
                "a/b",
                BlobKeyError::Segment {
                    source: SegmentError::Reserved { ch: '/' },
                },
            ),
            ("a\\b", BlobKeyError::Charset { ch: '\\' }),
            (
                "a:b",
                BlobKeyError::Segment {
                    source: SegmentError::Reserved { ch: ':' },
                },
            ),
            ("a\u{0}b", BlobKeyError::Charset { ch: '\u{0}' }),
            (
                long_key.as_str(),
                BlobKeyError::TooLong {
                    max: MAX_BLOB_KEY_BYTES,
                },
            ),
        ];

        for (input, expected) in cases {
            assert_eq!(BlobKey::new(input).unwrap_err(), expected);
        }
    }

    struct StubStore {
        data: Mutex<HashMap<String, Bytes>>,
    }

    impl StubStore {
        fn new() -> Self {
            Self {
                data: Mutex::new(HashMap::new()),
            }
        }
    }

    #[async_trait]
    impl StorageBlob for StubStore {
        async fn put(&self, key: &BlobKey, data: Bytes) -> Result<(), CapabilityError> {
            self.data
                .lock()
                .expect("lock")
                .insert(key.as_str().to_string(), data);
            Ok(())
        }

        async fn get(&self, key: &BlobKey) -> Result<Bytes, CapabilityError> {
            self.data
                .lock()
                .expect("lock")
                .get(key.as_str())
                .cloned()
                .ok_or_else(|| CapabilityError::not_found(key.as_str().to_string()))
        }

        async fn exists(&self, key: &BlobKey) -> Result<bool, CapabilityError> {
            Ok(self.data.lock().expect("lock").contains_key(key.as_str()))
        }

        async fn delete(&self, key: &BlobKey) -> Result<(), CapabilityError> {
            self.data.lock().expect("lock").remove(key.as_str());
            Ok(())
        }
    }

    #[test]
    fn storage_blob_is_object_safe() {
        let _: Option<Box<dyn StorageBlob>> = None;
    }

    #[test]
    fn put_then_get_round_trips_the_same_bytes() {
        let store: Box<dyn StorageBlob> = Box::new(StubStore::new());
        let key = BlobKey::new("digest-abc").expect("valid key");

        block_on(store.put(&key, Bytes::from_static(b"payload"))).expect("put succeeds");
        let bytes = block_on(store.get(&key)).expect("get succeeds");

        assert_eq!(bytes, Bytes::from_static(b"payload"));
        assert!(block_on(store.exists(&key)).expect("exists succeeds"));
    }

    #[test]
    fn get_on_missing_key_returns_not_found() {
        let store: Box<dyn StorageBlob> = Box::new(StubStore::new());
        let key = BlobKey::new("never-written").expect("valid key");

        let error = block_on(store.get(&key)).unwrap_err();

        assert_eq!(error, CapabilityError::not_found("never-written"));
    }

    #[test]
    fn delete_removes_the_stored_value() {
        let store: Box<dyn StorageBlob> = Box::new(StubStore::new());
        let key = BlobKey::new("digest-xyz").expect("valid key");

        block_on(store.put(&key, Bytes::from_static(b"payload"))).expect("put succeeds");
        block_on(store.delete(&key)).expect("delete succeeds");

        assert!(!block_on(store.exists(&key)).expect("exists succeeds"));
    }
}
