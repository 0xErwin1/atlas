use async_trait::async_trait;
use atlas_domain::{AttachmentStore, DomainError};
use bytes::Bytes;
use object_store::aws::AmazonS3Builder;
use object_store::path::Path as ObjectPath;
use object_store::{Error as ObjectStoreError, ObjectStore, PutPayload};
use sha2::{Digest, Sha256};

/// Connection parameters for an S3-compatible object store (e.g. Cloudflare R2).
///
/// `region` is "auto" for R2; real AWS S3 expects a concrete region. The endpoint is
/// the account-scoped S3 API URL. Secrets are held only long enough to build the
/// client and are never logged.
pub struct S3Config {
    pub bucket: String,
    pub endpoint: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
}

/// Content-addressed binary store backed by an S3-compatible object store.
///
/// Object keys mirror `DiskAttachmentStore`'s on-disk layout exactly:
/// `{sha[0..2]}/{sha[2..]}`. Content addressing makes `put` idempotent — identical
/// bytes always resolve to the same key, so re-uploading shared content is a no-op.
pub struct S3AttachmentStore {
    store: Box<dyn ObjectStore>,
}

impl S3AttachmentStore {
    /// Builds the S3 client from `config`, failing if the configuration is invalid.
    ///
    /// `allow_http` is enabled only for plaintext `http://` endpoints (e.g. a local
    /// MinIO during development); R2 and AWS use HTTPS, so the flag stays off there.
    pub fn new(config: S3Config) -> Result<Self, DomainError> {
        let allow_http = config.endpoint.starts_with("http://");

        let store = AmazonS3Builder::new()
            .with_bucket_name(config.bucket)
            .with_endpoint(config.endpoint)
            .with_access_key_id(config.access_key_id)
            .with_secret_access_key(config.secret_access_key)
            .with_region(config.region)
            .with_allow_http(allow_http)
            .build()
            .map_err(|e| DomainError::Internal {
                message: format!("cannot build S3 attachment store client: {e}"),
            })?;

        Ok(Self {
            store: Box::new(store),
        })
    }

    /// Maps a content digest to its object key.
    ///
    /// The digest must be a lowercase hex SHA-256 (exactly 64 chars in `[0-9a-f]`).
    /// Rejecting anything else mirrors `DiskAttachmentStore`: it prevents a crafted
    /// key from escaping the content-addressed namespace and guards the slicing below.
    fn object_path(&self, digest: &str) -> Result<ObjectPath, DomainError> {
        let is_valid_digest = digest.len() == 64
            && digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));

        if !is_valid_digest {
            return Err(DomainError::NotFound {
                entity: "attachment",
                id: uuid::Uuid::nil(),
            });
        }

        let prefix = &digest[..2];
        let rest = &digest[2..];

        Ok(ObjectPath::from(format!("{prefix}/{rest}")))
    }

    async fn object_exists(&self, location: &ObjectPath) -> Result<bool, DomainError> {
        match self.store.head(location).await {
            Ok(_) => Ok(true),
            Err(ObjectStoreError::NotFound { .. }) => Ok(false),
            Err(e) => Err(DomainError::Internal {
                message: format!("head object: {e}"),
            }),
        }
    }
}

#[async_trait]
impl AttachmentStore for S3AttachmentStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        let digest = hex_sha256(data);
        let location = self.object_path(&digest)?;

        if self.object_exists(&location).await? {
            return Ok(digest);
        }

        let payload = PutPayload::from(Bytes::copy_from_slice(data));

        self.store
            .put(&location, payload)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("put attachment {digest}: {e}"),
            })?;

        Ok(digest)
    }

    async fn get(&self, digest: &str) -> Result<Bytes, DomainError> {
        let location = self.object_path(digest)?;

        let result = self.store.get(&location).await.map_err(|e| match e {
            ObjectStoreError::NotFound { .. } => DomainError::NotFound {
                entity: "attachment",
                id: uuid::Uuid::nil(),
            },
            other => DomainError::Internal {
                message: format!("get attachment {digest}: {other}"),
            },
        })?;

        result.bytes().await.map_err(|e| match e {
            ObjectStoreError::NotFound { .. } => DomainError::NotFound {
                entity: "attachment",
                id: uuid::Uuid::nil(),
            },
            other => DomainError::Internal {
                message: format!("read attachment {digest}: {other}"),
            },
        })
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        let location = self.object_path(digest)?;
        self.object_exists(&location).await
    }
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("{result:x}")
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    fn store() -> S3AttachmentStore {
        S3AttachmentStore::new(S3Config {
            bucket: "test-bucket".into(),
            endpoint: "http://localhost:9000".into(),
            access_key_id: "test-key".into(),
            secret_access_key: "test-secret".into(),
            region: "auto".into(),
        })
        .expect("test S3 store must build")
    }

    #[test]
    fn object_path_rejects_short_digest() {
        let result = store().object_path("abc");
        assert!(
            matches!(result, Err(DomainError::NotFound { .. })),
            "short digest must be rejected, got: {result:?}"
        );
    }

    #[test]
    fn object_path_rejects_traversal_digest() {
        let traversal = format!("..{}", "/".repeat(62));
        let result = store().object_path(&traversal);
        assert!(
            matches!(result, Err(DomainError::NotFound { .. })),
            "traversal digest must be rejected, got: {result:?}"
        );
    }

    #[test]
    fn object_path_accepts_valid_digest() {
        let digest = "a".repeat(64);
        let result = store().object_path(&digest);
        assert!(result.is_ok(), "valid 64-char hex digest must be accepted");
    }

    #[test]
    fn object_path_layout_matches_disk_store() {
        let digest = "abcdef0123456789".repeat(4);
        let path = store().object_path(&digest).expect("valid digest");
        assert_eq!(path.as_ref(), format!("{}/{}", &digest[..2], &digest[2..]));
    }
}
