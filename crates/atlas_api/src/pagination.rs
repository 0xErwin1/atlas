use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[cfg(feature = "openapi")]
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// Sort-aware search cursor
// ---------------------------------------------------------------------------

const SEARCH_CURSOR_BYTES: usize = 25;
const SEARCH_CURSOR_CHARS: usize = 34;
const TAG_RELEVANCE: u8 = 0;
const TAG_UPDATED: u8 = 1;

/// Sort discriminant carried inside `SearchCursor`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SortKey {
    /// Relevance sort: holds the `ts_rank_cd` score (`f32`).
    Relevance(f32),
    /// Updated-at sort: holds the epoch in microseconds (`i64`).
    Updated(i64),
}

/// Sort-aware opaque cursor for search result pagination.
///
/// Wire format: base64url-nopad over a fixed 25-byte payload (→ 34 chars).
///
/// Layout:
/// - byte 0: sort tag (`TAG_RELEVANCE=0` or `TAG_UPDATED=1`)
/// - bytes 1..9: 8-byte big-endian ordering key
///   - relevance: f32 score in bytes 5..9; bytes 1..5 = 0x00
///   - updated: i64 epoch-microseconds
/// - bytes 9..25: 16 UUID bytes (tiebreak)
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SearchCursor {
    pub key: SortKey,
    pub id: Uuid,
}

impl SearchCursor {
    /// Encodes the cursor to the 34-char base64url-nopad wire format.
    pub fn encode(&self) -> String {
        // Layout: [tag(1)] [key(8)] [uuid(16)] = 25 bytes
        let tag: u8;
        let key_bytes: [u8; 8];

        match self.key {
            SortKey::Relevance(score) => {
                tag = TAG_RELEVANCE;
                // relevance f32 is in the LOW 4 bytes (bytes 5..9 of the payload);
                // HIGH 4 bytes (bytes 1..5) remain 0x00.
                let score_be = score.to_be_bytes();
                key_bytes = [0, 0, 0, 0, score_be[0], score_be[1], score_be[2], score_be[3]];
            }
            SortKey::Updated(micros) => {
                tag = TAG_UPDATED;
                key_bytes = micros.to_be_bytes();
            }
        }

        let id_bytes = *self.id.as_bytes();
        let buf: [u8; SEARCH_CURSOR_BYTES] = [
            tag,
            key_bytes[0], key_bytes[1], key_bytes[2], key_bytes[3],
            key_bytes[4], key_bytes[5], key_bytes[6], key_bytes[7],
            id_bytes[0],  id_bytes[1],  id_bytes[2],  id_bytes[3],
            id_bytes[4],  id_bytes[5],  id_bytes[6],  id_bytes[7],
            id_bytes[8],  id_bytes[9],  id_bytes[10], id_bytes[11],
            id_bytes[12], id_bytes[13], id_bytes[14], id_bytes[15],
        ];

        URL_SAFE_NO_PAD.encode(buf)
    }

    /// Decodes a wire-format search cursor.
    ///
    /// Returns `None` when the input is not exactly `SEARCH_CURSOR_CHARS` chars,
    /// is not valid base64url, or carries an unknown sort tag byte.
    pub fn decode(s: &str) -> Option<Self> {
        if s.len() != SEARCH_CURSOR_CHARS {
            return None;
        }

        let bytes = URL_SAFE_NO_PAD.decode(s).ok()?;
        if bytes.len() != SEARCH_CURSOR_BYTES {
            return None;
        }

        let tag = *bytes.first()?;
        let key = match tag {
            TAG_RELEVANCE => {
                // score is in bytes 5..9 (low 4 bytes of the 8-byte key field)
                let score_bytes: [u8; 4] = bytes.get(5..9)?.try_into().ok()?;
                SortKey::Relevance(f32::from_be_bytes(score_bytes))
            }
            TAG_UPDATED => {
                let micros_bytes: [u8; 8] = bytes.get(1..9)?.try_into().ok()?;
                SortKey::Updated(i64::from_be_bytes(micros_bytes))
            }
            _ => return None,
        };

        let id_bytes: [u8; 16] = bytes.get(9..25)?.try_into().ok()?;
        let id = Uuid::from_bytes(id_bytes);

        Some(SearchCursor { key, id })
    }
}

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

    /// Constructs a page for search results using a sort-aware `SearchCursor`.
    pub fn new_search(items: Vec<T>, next_cursor: Option<SearchCursor>, has_more: bool) -> Self {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_uuid() -> Uuid {
        Uuid::parse_str("018f4a1b-2c3d-7e4f-a5b6-c7d8e9f01234").unwrap()
    }

    #[test]
    fn search_cursor_relevance_round_trip() {
        let score = 0.75_f32;
        let id = fixed_uuid();
        let cursor = SearchCursor {
            key: SortKey::Relevance(score),
            id,
        };
        let encoded = cursor.encode();
        assert_eq!(encoded.len(), SEARCH_CURSOR_CHARS);

        let decoded = SearchCursor::decode(&encoded).expect("must decode");
        assert_eq!(decoded.id, id);
        match decoded.key {
            SortKey::Relevance(s) => assert_eq!(s.to_bits(), score.to_bits()),
            SortKey::Updated(_) => panic!("wrong tag"),
        }
    }

    #[test]
    fn search_cursor_updated_round_trip() {
        let micros = 1_700_000_000_000_000_i64;
        let id = fixed_uuid();
        let cursor = SearchCursor {
            key: SortKey::Updated(micros),
            id,
        };
        let encoded = cursor.encode();
        assert_eq!(encoded.len(), SEARCH_CURSOR_CHARS);

        let decoded = SearchCursor::decode(&encoded).expect("must decode");
        assert_eq!(decoded.id, id);
        match decoded.key {
            SortKey::Updated(m) => assert_eq!(m, micros),
            SortKey::Relevance(_) => panic!("wrong tag"),
        }
    }

    #[test]
    fn search_cursor_decode_rejects_22_char_string() {
        // A valid Cursor (22 chars) must NOT decode as SearchCursor
        let ordinary = Cursor(fixed_uuid());
        let encoded = ordinary.encode();
        assert_eq!(encoded.len(), 22);
        assert!(SearchCursor::decode(&encoded).is_none());
    }

    #[test]
    fn search_cursor_decode_rejects_unknown_tag() {
        // Build a 25-byte payload with tag byte = 0xFF and encode it
        let mut buf = [0u8; SEARCH_CURSOR_BYTES];
        buf[0] = 0xFF;
        buf[9..25].copy_from_slice(fixed_uuid().as_bytes());
        let s = URL_SAFE_NO_PAD.encode(buf);
        assert_eq!(s.len(), SEARCH_CURSOR_CHARS);
        assert!(SearchCursor::decode(&s).is_none());
    }

    #[test]
    fn search_cursor_decode_rejects_non_base64() {
        assert!(SearchCursor::decode("not-valid-base64-at-all!!!!!!!!!!").is_none());
    }

    #[test]
    fn search_cursor_encode_tag_byte_is_correct() {
        let relevance_cursor = SearchCursor {
            key: SortKey::Relevance(0.5),
            id: fixed_uuid(),
        };
        let updated_cursor = SearchCursor {
            key: SortKey::Updated(42),
            id: fixed_uuid(),
        };
        let rel_bytes = URL_SAFE_NO_PAD
            .decode(relevance_cursor.encode())
            .unwrap();
        let upd_bytes = URL_SAFE_NO_PAD
            .decode(updated_cursor.encode())
            .unwrap();
        assert_eq!(rel_bytes[0], TAG_RELEVANCE);
        assert_eq!(upd_bytes[0], TAG_UPDATED);
    }

    #[test]
    fn search_cursor_relevance_zero_score_round_trip() {
        let cursor = SearchCursor {
            key: SortKey::Relevance(0.0_f32),
            id: fixed_uuid(),
        };
        let decoded = SearchCursor::decode(&cursor.encode()).unwrap();
        match decoded.key {
            SortKey::Relevance(s) => assert_eq!(s, 0.0_f32),
            _ => panic!("wrong tag"),
        }
    }

    #[test]
    fn search_cursor_negative_updated_round_trip() {
        let cursor = SearchCursor {
            key: SortKey::Updated(-1_000_000_i64),
            id: fixed_uuid(),
        };
        let decoded = SearchCursor::decode(&cursor.encode()).unwrap();
        match decoded.key {
            SortKey::Updated(m) => assert_eq!(m, -1_000_000_i64),
            _ => panic!("wrong tag"),
        }
    }

    #[test]
    fn ordinary_cursor_encode_decode_unchanged() {
        let id = fixed_uuid();
        let c = Cursor(id);
        let decoded = Cursor::decode(&c.encode()).expect("must decode");
        assert_eq!(decoded.0, id);
    }

    // Golden-vector tests: pin the exact 25-byte payload layout so any
    // regression in the byte-position of the sort key is caught immediately.
    // This is load-bearing because the layout is a committed wire contract —
    // a cursor issued by one server version must decode on the next.

    #[test]
    fn relevance_cursor_golden_byte_layout() {
        // Known score chosen so its BE bytes are distinct and non-zero.
        let score = 0.5_f32;
        let id = fixed_uuid();
        let cursor = SearchCursor {
            key: SortKey::Relevance(score),
            id,
        };
        let encoded = cursor.encode();
        let raw = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
        assert_eq!(raw.len(), SEARCH_CURSOR_BYTES);

        // byte 0: tag must be 0 (relevance)
        assert_eq!(raw[0], TAG_RELEVANCE);

        // bytes 1..5: high bytes of key field must be 0x00 (left-zero-extended)
        assert_eq!(&raw[1..5], &[0u8, 0, 0, 0], "bytes 1..5 must be zero for relevance");

        // bytes 5..9: f32 in BE
        let expected_score_bytes = score.to_be_bytes();
        assert_eq!(&raw[5..9], &expected_score_bytes, "bytes 5..9 must hold f32 BE");

        // bytes 9..25: UUID
        assert_eq!(&raw[9..25], id.as_bytes().as_slice());
    }

    #[test]
    fn updated_cursor_golden_byte_layout() {
        let micros: i64 = 1_718_000_000_000_000;
        let id = fixed_uuid();
        let cursor = SearchCursor {
            key: SortKey::Updated(micros),
            id,
        };
        let encoded = cursor.encode();
        let raw = URL_SAFE_NO_PAD.decode(&encoded).unwrap();
        assert_eq!(raw.len(), SEARCH_CURSOR_BYTES);

        // byte 0: tag must be 1 (updated)
        assert_eq!(raw[0], TAG_UPDATED);

        // bytes 1..9: i64 epoch-micros in BE
        let expected_micros_bytes = micros.to_be_bytes();
        assert_eq!(&raw[1..9], &expected_micros_bytes, "bytes 1..9 must hold i64 BE");

        // bytes 9..25: UUID
        assert_eq!(&raw[9..25], id.as_bytes().as_slice());
    }
}
