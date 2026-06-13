use async_trait::async_trait;
use atlas_domain::{AttachmentStore, DomainError};
use bytes::Bytes;
use sha2::{Digest, Sha256};
use std::path::PathBuf;
use tokio::io::AsyncWriteExt;

/// Content-addressed binary store backed by the local filesystem.
///
/// Layout: `{root}/{sha[0..2]}/{sha[2..]}` — the two-character prefix keeps
/// directory width bounded without deep nesting.
///
/// Writes are atomic: bytes land in a `.tmp` file first, then renamed into
/// place. Concurrent puts of the same content are idempotent.
pub struct DiskAttachmentStore {
    root: PathBuf,
}

impl DiskAttachmentStore {
    /// Creates a new store rooted at `root`.
    ///
    /// `root` is created if it does not exist. The function validates that the
    /// directory is writable by creating it (or confirming it already exists).
    pub async fn new(root: impl Into<PathBuf>) -> Result<Self, DomainError> {
        let root = root.into();

        tokio::fs::create_dir_all(&root)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("cannot create attachment root {}: {e}", root.display()),
            })?;

        Ok(Self { root })
    }

    fn object_path(&self, digest: &str) -> PathBuf {
        let prefix = &digest[..2];
        let rest = &digest[2..];
        self.root.join(prefix).join(rest)
    }
}

#[async_trait]
impl AttachmentStore for DiskAttachmentStore {
    async fn put(&self, data: &[u8]) -> Result<String, DomainError> {
        let digest = hex_sha256(data);
        let dest = self.object_path(&digest);

        if dest.exists() {
            return Ok(digest);
        }

        let prefix_dir = dest.parent().ok_or_else(|| DomainError::Internal {
            message: "object path has no parent directory".into(),
        })?;

        tokio::fs::create_dir_all(prefix_dir)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("create prefix dir: {e}"),
            })?;

        let tmp_path = dest.with_extension("tmp");
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("open tmp file: {e}"),
            })?;

        file.write_all(data)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("write tmp file: {e}"),
            })?;

        file.flush().await.map_err(|e| DomainError::Internal {
            message: format!("flush tmp file: {e}"),
        })?;

        drop(file);

        tokio::fs::rename(&tmp_path, &dest)
            .await
            .map_err(|e| DomainError::Internal {
                message: format!("rename tmp to dest: {e}"),
            })?;

        Ok(digest)
    }

    async fn get(&self, digest: &str) -> Result<Bytes, DomainError> {
        let path = self.object_path(digest);
        let bytes = tokio::fs::read(&path).await.map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => DomainError::NotFound {
                entity: "attachment",
                id: uuid::Uuid::nil(),
            },
            _ => DomainError::Internal {
                message: format!("read attachment {digest}: {e}"),
            },
        })?;

        Ok(Bytes::from(bytes))
    }

    async fn exists(&self, digest: &str) -> Result<bool, DomainError> {
        let path = self.object_path(digest);
        Ok(path.exists())
    }
}

fn hex_sha256(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    let result = hasher.finalize();
    format!("{result:x}")
}
