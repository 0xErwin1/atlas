use crate::DomainError;
use async_trait::async_trait;
use bytes::Bytes;

/// Content-addressed binary store for attachment data.
///
/// The store is keyed by the hex-encoded SHA-256 of the content, so identical
/// bytes are stored once regardless of how many documents reference them.
#[async_trait]
pub trait AttachmentStore: Send + Sync {
    /// Writes `data` to the store and returns the hex-encoded SHA-256 digest.
    ///
    /// Callers may call `exists` first to skip redundant writes, but `put` is
    /// idempotent: writing the same bytes again is a no-op on the storage side.
    async fn put(&self, data: &[u8]) -> Result<String, DomainError>;

    /// Reads the object identified by its hex-encoded SHA-256 `digest`.
    ///
    /// Returns `DomainError::NotFound` when the object does not exist.
    async fn get(&self, digest: &str) -> Result<Bytes, DomainError>;

    /// Returns `true` if an object with this digest already exists in the store.
    async fn exists(&self, digest: &str) -> Result<bool, DomainError>;
}

#[cfg(test)]
mod tests {
    /// Doc-test: the trait is object-safe and its method signatures compile.
    ///
    /// This is intentionally a compile-only test — it has no runtime I/O.
    #[test]
    fn attachment_store_is_object_safe() {
        use super::AttachmentStore;
        let _: Option<Box<dyn AttachmentStore>> = None;
    }
}
