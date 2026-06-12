use argon2::{
    Argon2, PasswordHasher, PasswordVerifier,
    password_hash::{PasswordHash, SaltString, rand_core::OsRng},
};

#[derive(Debug)]
pub enum PasswordError {
    Hashing(String),
    Verification(String),
}

impl std::fmt::Display for PasswordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PasswordError::Hashing(msg) => write!(f, "password hashing failed: {msg}"),
            PasswordError::Verification(msg) => write!(f, "password verification failed: {msg}"),
        }
    }
}

/// Hashes `password` using Argon2id inside `spawn_blocking` so the async runtime is not blocked.
pub async fn hash(password: String) -> Result<String, PasswordError> {
    tokio::task::spawn_blocking(move || hash_sync(&password))
        .await
        .map_err(|e| PasswordError::Hashing(e.to_string()))?
}

/// Verifies `password` against a stored PHC-format `hash` inside `spawn_blocking`.
pub async fn verify(password: String, hash: String) -> Result<bool, PasswordError> {
    tokio::task::spawn_blocking(move || verify_sync(&password, &hash))
        .await
        .map_err(|e| PasswordError::Verification(e.to_string()))?
}

fn hash_sync(password: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| PasswordError::Hashing(e.to_string()))
}

fn verify_sync(password: &str, hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(hash).map_err(|e| PasswordError::Verification(e.to_string()))?;
    match Argon2::default().verify_password(password.as_bytes(), &parsed) {
        Ok(()) => Ok(true),
        Err(argon2::password_hash::Error::Password) => Ok(false),
        Err(e) => Err(PasswordError::Verification(e.to_string())),
    }
}
