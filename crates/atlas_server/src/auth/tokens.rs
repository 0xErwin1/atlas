use atlas_custos::entities::identity::ApiKeyKind;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngCore;
use sha2::{Digest, Sha256};

/// Generates a random 43-char base64url-nopad session token.
pub fn generate_session_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Generates a random `atlas_ak_`-prefixed API key (43-char base64url body).
/// Agent is the default credential kind; kept for existing agent-key callers.
pub fn generate_api_key() -> String {
    generate_api_key_of_kind(ApiKeyKind::Agent)
}

/// Generates a random API key whose self-describing prefix mirrors the
/// credential kind minted at creation: `atlas_pk_…` personal, `atlas_ak_…`
/// agent. The 43-char base64url body is unchanged, so the stored SHA-256 and
/// every existing lookup keep working. Only the token's hash is stored, so the
/// prefix is not recoverable at rest — the prefix/kind agreement is enforced
/// at authentication time.
pub fn generate_api_key_of_kind(kind: ApiKeyKind) -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let prefix = match kind {
        ApiKeyKind::Personal => "atlas_pk_",
        ApiKeyKind::Agent => "atlas_ak_",
    };
    format!("{prefix}{}", URL_SAFE_NO_PAD.encode(bytes))
}

/// SHA-256 hex digest of the given token (for storage and lookup).
pub fn hash_token(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn personal_api_key_uses_atlas_pk_prefix() {
        let token = generate_api_key_of_kind(ApiKeyKind::Personal);
        assert!(
            token.starts_with("atlas_pk_"),
            "personal key must use the atlas_pk_ prefix, got {token}"
        );
        assert_eq!(token.len(), "atlas_pk_".len() + 43);
    }

    #[test]
    fn agent_api_key_uses_atlas_ak_prefix() {
        let token = generate_api_key_of_kind(ApiKeyKind::Agent);
        assert!(
            token.starts_with("atlas_ak_"),
            "agent key must use the atlas_ak_ prefix, got {token}"
        );
        assert_eq!(token.len(), "atlas_ak_".len() + 43);
    }

    #[test]
    fn legacy_generate_api_key_now_emits_the_agent_prefix() {
        let token = generate_api_key();
        assert!(
            token.starts_with("atlas_ak_"),
            "the default generator must emit the agent prefix, got {token}"
        );
    }
}
