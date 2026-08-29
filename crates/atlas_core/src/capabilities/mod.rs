//! Neutral in-process trait contracts backing the registry's capability
//! identities (SHELL-CAP-1). This is the sole allowed in-process
//! collaboration surface between components (SHELL-INT-1).

pub mod diagnostics;
pub mod error;
pub mod resource_provider;
pub mod search;
pub mod search_lexical;
pub mod search_semantic;
pub mod share_link;
pub mod storage_blob;

#[cfg(test)]
mod test_support;

pub use diagnostics::{
    Doctor, DoctorFinding, Health, HealthStatus, Readiness, ReadinessStatus, Severity,
};
pub use error::CapabilityError;
pub use resource_provider::{ProviderCatalog, ResourceProvider};
pub use search::{IndexedDocument, LexicalQuery, SearchHit, SemanticQuery};
pub use search_lexical::SearchLexical;
pub use search_semantic::SearchSemantic;
pub use share_link::{
    MAX_SHARE_TOKEN_BYTES, PasswordHash, PasswordHashError, ShareLinkCredentialPort,
    ShareLinkGrant, ShareToken, ShareTokenError,
};
pub use storage_blob::{BlobKey, BlobKeyError, MAX_BLOB_KEY_BYTES, StorageBlob};
