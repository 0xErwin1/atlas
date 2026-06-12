use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

/// Paginated response envelope.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(ToSchema))]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
}

impl<T> Page<T> {
    pub fn new(items: Vec<T>, next_cursor: Option<Cursor>, has_more: bool) -> Self {
        Self {
            items,
            next_cursor: next_cursor.map(|c| c.encode()),
            has_more,
        }
    }

    pub fn empty() -> Self {
        Self {
            items: vec![],
            next_cursor: None,
            has_more: false,
        }
    }
}

/// Opaque cursor backed by a UUIDv7.
///
/// Wire format: base64url-no-pad encoding of the 16 UUID bytes (22 chars).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor(pub Uuid);

impl Cursor {
    /// Encodes the cursor to the 22-char base64url-nopad wire format.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.0.as_bytes())
    }

    /// Decodes a wire-format cursor.
    ///
    /// Returns `None` if the input is not a valid 22-char base64url-nopad UUID.
    pub fn decode(s: &str) -> Option<Self> {
        if s.len() != 22 {
            return None;
        }
        let bytes = URL_SAFE_NO_PAD.decode(s).ok()?;
        let arr: [u8; 16] = bytes.try_into().ok()?;
        Some(Cursor(Uuid::from_bytes(arr)))
    }
}
