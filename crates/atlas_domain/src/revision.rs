use crate::error::DomainError;
use diffy_imara::{apply, create_patch};

/// Creates a unified diff patch from `old` to `new`.
pub fn create_revision_patch(old: &str, new: &str) -> String {
    create_patch(old, new).to_string()
}

/// Applies a unified diff patch to a base string, returning the result.
///
/// Returns `Err(DomainError::InvalidInput)` if the patch cannot be applied cleanly.
pub fn apply_revision_patch(base: &str, patch: &str) -> Result<String, DomainError> {
    let parsed: diffy_imara::Patch<str> =
        diffy_imara::Patch::from_str(patch).map_err(|e| DomainError::InvalidInput {
            message: format!("invalid patch: {e}"),
        })?;

    apply(base, &parsed).map_err(|e| DomainError::InvalidInput {
        message: format!("patch apply failed: {e}"),
    })
}

/// Reconstructs content by applying a sequence of patches to a snapshot.
///
/// The `snapshot` is the anchor content and `patches` are applied in order.
pub fn reconstruct(snapshot: &str, patches: &[&str]) -> Result<String, DomainError> {
    let mut content = snapshot.to_owned();

    for patch in patches {
        content = apply_revision_patch(&content, patch)?;
    }

    Ok(content)
}

/// Returns true when `seq` should be stored as an anchor snapshot.
///
/// Seq 1 is always an anchor (initial content). For subsequent sequences, an anchor
/// is created whenever `seq % interval == 0`. With default interval=50, anchors
/// occur at seq 1, 50, 100, 150, ...
pub fn is_anchor_seq(seq: i64, interval: u32) -> bool {
    seq == 1 || seq % (interval as i64) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patch_roundtrip() {
        let old = "hello world";
        let new = "hello atlas";
        let patch = create_revision_patch(old, new);
        let result = apply_revision_patch(old, &patch).expect("apply");
        assert_eq!(result, new);
    }

    #[test]
    fn reconstruct_across_anchor_boundary() {
        let initial = "version one";
        let v2 = "version two";
        let v3 = "version three";
        let p2 = create_revision_patch(initial, v2);
        let p3 = create_revision_patch(v2, v3);
        let result = reconstruct(initial, &[&p2, &p3]).expect("reconstruct");
        assert_eq!(result, v3);
    }

    #[test]
    fn anchor_cadence_with_interval_3() {
        assert!(is_anchor_seq(1, 3));
        assert!(!is_anchor_seq(2, 3));
        assert!(is_anchor_seq(3, 3));
        assert!(!is_anchor_seq(4, 3));
        assert!(!is_anchor_seq(5, 3));
        assert!(is_anchor_seq(6, 3));
        assert!(!is_anchor_seq(7, 3));
    }

    #[test]
    fn anchor_seq_1_always_anchor_regardless_of_interval() {
        assert!(is_anchor_seq(1, 2));
        assert!(is_anchor_seq(1, 50));
        assert!(is_anchor_seq(1, 100));
    }

    #[test]
    fn anchor_cadence_default_interval() {
        assert!(is_anchor_seq(1, 50));
        assert!(!is_anchor_seq(49, 50));
        assert!(is_anchor_seq(50, 50));
        assert!(!is_anchor_seq(51, 50));
        assert!(is_anchor_seq(100, 50));
    }
}
