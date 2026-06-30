#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

use aes_gcm::{
    Aes256Gcm, Key,
    aead::{Aead, AeadCore, KeyInit, generic_array::GenericArray},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use rand::rngs::OsRng;
use std::fmt;

/// AES-256-GCM cipher used to encrypt and decrypt webhook HMAC secrets at rest.
///
/// The cipher key is loaded once at startup from `ATLAS_WEBHOOK_ENC_KEY`. It is
/// never written to logs or debug output. Each encryption call uses a fresh
/// 12-byte nonce from `OsRng`; both ciphertext and nonce must be persisted so
/// that decryption is possible at delivery time.
pub struct WebhookCrypto {
    cipher: Aes256Gcm,
}

impl WebhookCrypto {
    /// Constructs the cipher from a 32-byte key.
    pub fn new(key_bytes: &[u8; 32]) -> Self {
        let key = Key::<Aes256Gcm>::from_slice(key_bytes);
        Self {
            cipher: Aes256Gcm::new(key),
        }
    }

    /// Decodes a standard-base64 string and constructs the cipher.
    ///
    /// The input must decode to exactly 32 bytes. Returns `Err` with a message
    /// that describes the problem but never echoes the encoded value.
    pub fn from_base64_key(encoded: &str) -> Result<Self, String> {
        let bytes = STANDARD
            .decode(encoded.trim())
            .map_err(|e| format!("ATLAS_WEBHOOK_ENC_KEY is not valid base64: {e}"))?;

        let key_bytes: [u8; 32] = bytes.as_slice().try_into().map_err(|_| {
            format!(
                "ATLAS_WEBHOOK_ENC_KEY must decode to exactly 32 bytes, got {}",
                bytes.len()
            )
        })?;

        Ok(Self::new(&key_bytes))
    }

    /// Reads `ATLAS_WEBHOOK_ENC_KEY` from the environment and constructs the cipher.
    ///
    /// Returns `Err` if the variable is missing or does not pass `from_base64_key`
    /// validation. The error message names the variable and the problem but never
    /// echoes the key value.
    pub fn load_from_env() -> Result<Self, String> {
        let raw = std::env::var("ATLAS_WEBHOOK_ENC_KEY")
            .map_err(|_| "ATLAS_WEBHOOK_ENC_KEY is required but not set".to_string())?;
        Self::from_base64_key(&raw)
    }

    /// Constructs the cipher from a fresh random 32-byte key.
    ///
    /// Only for test environments where `ATLAS_WEBHOOK_ENC_KEY` is not configured.
    pub fn generate_for_test() -> Self {
        use rand::RngCore;
        let mut key_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut key_bytes);
        Self::new(&key_bytes)
    }

    /// Encrypts `plaintext` with a fresh 12-byte random nonce.
    ///
    /// Returns `(ciphertext, nonce)`. Both must be stored together; the nonce is
    /// needed to decrypt.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>), String> {
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = self
            .cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| format!("AES-GCM encryption failed: {e}"))?;
        Ok((ciphertext, nonce.to_vec()))
    }

    /// Decrypts `ciphertext` using `nonce`.
    ///
    /// Returns `Err` when the nonce length is wrong, the ciphertext is tampered,
    /// or a different key was used to encrypt.
    pub fn decrypt(&self, ciphertext: &[u8], nonce: &[u8]) -> Result<Vec<u8>, String> {
        if nonce.len() != 12 {
            return Err(format!(
                "nonce must be exactly 12 bytes, got {}",
                nonce.len()
            ));
        }
        let nonce = GenericArray::from_slice(nonce);
        self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| format!("AES-GCM decryption failed: {e}"))
    }
}

impl fmt::Debug for WebhookCrypto {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WebhookCrypto")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key() -> [u8; 32] {
        [0x42u8; 32]
    }

    fn test_crypto() -> WebhookCrypto {
        WebhookCrypto::new(&test_key())
    }

    // B3.2-1 — roundtrip: encrypt then decrypt recovers the original plaintext
    #[test]
    fn roundtrip_recovers_plaintext() {
        let crypto = test_crypto();
        let plaintext = b"my-webhook-secret-value";

        let (ciphertext, nonce) = crypto.encrypt(plaintext).unwrap();
        let recovered = crypto.decrypt(&ciphertext, &nonce).unwrap();

        assert_eq!(recovered, plaintext);
    }

    // B3.2-2 — two encryptions of the same plaintext produce different nonces and ciphertexts
    #[test]
    fn nonces_are_unique_per_encrypt() {
        let crypto = test_crypto();
        let plaintext = b"same-plaintext";

        let (ct1, nonce1) = crypto.encrypt(plaintext).unwrap();
        let (ct2, nonce2) = crypto.encrypt(plaintext).unwrap();

        assert_ne!(nonce1, nonce2, "nonces must differ between calls");
        assert_ne!(ct1, ct2, "ciphertexts must differ when nonces differ");
    }

    // B3.2-3 — wrong key causes decryption to fail
    #[test]
    fn wrong_key_fails_decryption() {
        let encryptor = WebhookCrypto::new(&[0xAAu8; 32]);
        let decryptor = WebhookCrypto::new(&[0xBBu8; 32]);

        let (ciphertext, nonce) = encryptor.encrypt(b"secret").unwrap();
        let result = decryptor.decrypt(&ciphertext, &nonce);

        assert!(result.is_err(), "decryption with wrong key must fail");
    }

    // B3.2-4 — tampered nonce causes decryption to fail
    #[test]
    fn wrong_nonce_fails_decryption() {
        let crypto = test_crypto();
        let (ciphertext, mut nonce) = crypto.encrypt(b"secret").unwrap();

        nonce[0] ^= 0xFF;
        let result = crypto.decrypt(&ciphertext, &nonce);

        assert!(result.is_err(), "decryption with wrong nonce must fail");
    }

    // B3.2-5 — nonce with wrong length returns an error immediately
    #[test]
    fn wrong_nonce_length_returns_error() {
        let crypto = test_crypto();
        let (ciphertext, _) = crypto.encrypt(b"secret").unwrap();

        let short_nonce = [0u8; 8];
        let result = crypto.decrypt(&ciphertext, &short_nonce);

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("12 bytes"), "error must mention expected nonce size");
    }

    // B3.2-6 — Debug output must be redacted (never expose the key)
    #[test]
    fn debug_output_is_redacted() {
        let crypto = WebhookCrypto::new(&[0x99u8; 32]);
        let output = format!("{crypto:?}");

        assert!(
            output.contains("[REDACTED]"),
            "Debug output must contain [REDACTED]: {output}"
        );
        assert!(
            !output.contains("0x99"),
            "key bytes must not appear in Debug output: {output}"
        );
    }

    // B3.2-7 — from_base64_key fails when the encoded value is not 32 bytes
    #[test]
    fn from_base64_key_fails_on_wrong_key_size() {
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 16]);
        let result = WebhookCrypto::from_base64_key(&short);

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("32 bytes"), "error must mention required key length: {msg}");
    }

    // B3.2-8 — from_base64_key fails when the input is not valid base64
    #[test]
    fn from_base64_key_fails_on_invalid_base64() {
        let result = WebhookCrypto::from_base64_key("not!!valid!!base64");

        assert!(result.is_err());
        let msg = result.unwrap_err();
        assert!(msg.contains("base64"), "error must mention base64: {msg}");
    }

    // B3.2-9 — from_base64_key succeeds with a valid 32-byte base64 key
    #[test]
    fn from_base64_key_succeeds_with_valid_key() {
        let valid = base64::engine::general_purpose::STANDARD.encode([0x42u8; 32]);
        let result = WebhookCrypto::from_base64_key(&valid);

        assert!(result.is_ok(), "must succeed with a valid 32-byte base64 key");

        // Verify it's actually functional (roundtrip)
        let crypto = result.unwrap();
        let (ct, nonce) = crypto.encrypt(b"test").unwrap();
        let pt = crypto.decrypt(&ct, &nonce).unwrap();
        assert_eq!(pt, b"test");
    }
}
