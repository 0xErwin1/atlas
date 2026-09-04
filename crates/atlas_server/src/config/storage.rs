//! `storage.filesystem` / `storage.s3` Module configuration (registry
//! declaration: `ConfigDeclaration::new("StorageConfig", "ATLAS_STORAGE_",
//! true)` on both entries, `reg5.rs`).
//!
//! An enum, not a struct with parallel `Option` fields (design D1.2):
//! `ATLAS_ATTACHMENT_BACKEND` already selects between two mutually exclusive
//! registry entries, so the mutual exclusion becomes the type's shape. The
//! `ATLAS_STORAGE_` prefix governs *new* variable names only (design D2.4);
//! V1's `ATLAS_ATTACHMENT_*`/`ATLAS_S3_*` names are grandfathered unchanged.

use atlas_core::config::{ComponentConfig, ConfigError, EnvSource, Secret};

use super::env_var_nonempty;

/// Typed configuration owned by whichever storage Module is active.
#[derive(Debug, Clone, PartialEq, Eq)] // safe: `secret_access_key` is `Secret<String>`.
pub enum StorageConfig {
    Disk {
        root: String,
    },
    S3 {
        bucket: String,
        endpoint: String,
        access_key_id: String,
        secret_access_key: Secret<String>,
        region: String,
    },
}

impl ComponentConfig for StorageConfig {
    fn from_env(source: &dyn EnvSource) -> Result<Self, ConfigError> {
        let backend = env_var_nonempty(source, "ATLAS_ATTACHMENT_BACKEND")
            .unwrap_or_else(|| "disk".to_string());

        match backend.as_str() {
            "disk" => Ok(Self::Disk {
                root: env_var_nonempty(source, "ATLAS_ATTACHMENT_ROOT")
                    .unwrap_or_else(|| "./data/attachments".to_string()),
            }),
            "s3" => Ok(Self::S3 {
                bucket: require(source, "ATLAS_S3_BUCKET")?,
                endpoint: require(source, "ATLAS_S3_ENDPOINT")?,
                access_key_id: require(source, "ATLAS_S3_ACCESS_KEY_ID")?,
                secret_access_key: Secret::new(require(source, "ATLAS_S3_SECRET_ACCESS_KEY")?),
                region: env_var_nonempty(source, "ATLAS_S3_REGION")
                    .unwrap_or_else(|| "auto".to_string()),
            }),
            _ => Err(ConfigError::invalid(
                "ATLAS_ATTACHMENT_BACKEND",
                "must be 'disk' or 's3'",
            )),
        }
    }
}

fn require(source: &dyn EnvSource, var: &str) -> Result<String, ConfigError> {
    source.get(var).ok_or_else(|| ConfigError::missing(var))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::reg5::{StorageBackend, storage_backend_from_env};

    fn env(pairs: &'static [(&'static str, &'static str)]) -> impl EnvSource {
        move |key: &str| {
            pairs
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| (*value).to_owned())
        }
    }

    #[test]
    fn from_env_defaults_to_disk_backend() {
        let cfg = StorageConfig::from_env(&env(&[])).expect("expected Ok");

        assert_eq!(
            cfg,
            StorageConfig::Disk {
                root: "./data/attachments".to_string()
            }
        );
    }

    #[test]
    fn from_env_binds_the_disk_root() {
        let cfg = StorageConfig::from_env(&env(&[("ATLAS_ATTACHMENT_ROOT", "/custom/root")]))
            .expect("expected Ok");

        assert_eq!(
            cfg,
            StorageConfig::Disk {
                root: "/custom/root".to_string()
            }
        );
    }

    #[test]
    fn from_env_binds_the_s3_variant_with_a_secret_key() {
        let cfg = StorageConfig::from_env(&env(&[
            ("ATLAS_ATTACHMENT_BACKEND", "s3"),
            ("ATLAS_S3_BUCKET", "b"),
            ("ATLAS_S3_ENDPOINT", "e"),
            ("ATLAS_S3_ACCESS_KEY_ID", "a"),
            ("ATLAS_S3_SECRET_ACCESS_KEY", "s"),
        ]))
        .expect("expected Ok");

        match cfg {
            StorageConfig::S3 {
                bucket,
                endpoint,
                access_key_id,
                secret_access_key,
                region,
            } => {
                assert_eq!(bucket, "b");
                assert_eq!(endpoint, "e");
                assert_eq!(access_key_id, "a");
                assert_eq!(secret_access_key.expose(), "s");
                assert_eq!(region, "auto");
            }
            other => panic!("expected the S3 variant, got {other:?}"),
        }
    }

    #[test]
    fn from_env_s3_missing_bucket_is_missing_error() {
        let error = StorageConfig::from_env(&env(&[
            ("ATLAS_ATTACHMENT_BACKEND", "s3"),
            ("ATLAS_S3_ENDPOINT", "e"),
            ("ATLAS_S3_ACCESS_KEY_ID", "a"),
            ("ATLAS_S3_SECRET_ACCESS_KEY", "s"),
        ]))
        .expect_err("expected Err");

        assert_eq!(error, ConfigError::missing("ATLAS_S3_BUCKET"));
    }

    #[test]
    fn from_env_rejects_an_unknown_backend_without_echoing_it() {
        let error = StorageConfig::from_env(&env(&[("ATLAS_ATTACHMENT_BACKEND", "azure")]))
            .expect_err("expected Err");

        assert_eq!(
            error,
            ConfigError::invalid("ATLAS_ATTACHMENT_BACKEND", "must be 'disk' or 's3'")
        );
        assert!(!error.to_string().contains("azure"));
    }

    /// R6: `reg5::storage_backend_from_env` (which selects the entry set
    /// `AtlasConfig::from_registry` iterates, and therefore runs *before*
    /// config loading) and `StorageConfig::from_env`'s own discriminator
    /// must resolve identically for every input, or the registry and the
    /// composed config could disagree about which backend is active.
    #[test]
    fn agrees_with_reg5_storage_backend_selection_for_all_four_inputs() {
        let unset = env(&[]);
        assert!(matches!(
            storage_backend_from_env(&unset),
            Ok(StorageBackend::Filesystem)
        ));
        assert!(matches!(
            StorageConfig::from_env(&unset),
            Ok(StorageConfig::Disk { .. })
        ));

        let disk = env(&[("ATLAS_ATTACHMENT_BACKEND", "disk")]);
        assert!(matches!(
            storage_backend_from_env(&disk),
            Ok(StorageBackend::Filesystem)
        ));
        assert!(matches!(
            StorageConfig::from_env(&disk),
            Ok(StorageConfig::Disk { .. })
        ));

        let s3 = env(&[
            ("ATLAS_ATTACHMENT_BACKEND", "s3"),
            ("ATLAS_S3_BUCKET", "b"),
            ("ATLAS_S3_ENDPOINT", "e"),
            ("ATLAS_S3_ACCESS_KEY_ID", "a"),
            ("ATLAS_S3_SECRET_ACCESS_KEY", "s"),
        ]);
        assert!(matches!(
            storage_backend_from_env(&s3),
            Ok(StorageBackend::S3)
        ));
        assert!(matches!(
            StorageConfig::from_env(&s3),
            Ok(StorageConfig::S3 { .. })
        ));

        let garbage = env(&[("ATLAS_ATTACHMENT_BACKEND", "azure")]);
        assert!(storage_backend_from_env(&garbage).is_err());
        assert!(StorageConfig::from_env(&garbage).is_err());
    }
}
