use fractional_index::FractionalIndex;

/// Returns a position key that sorts between `before` and `after`, or `None` if the
/// fractional space between the two anchors is exhausted.
///
/// When `before` is `None`, the result sorts before `after` (or at the default position
/// if `after` is also `None`). When `after` is `None`, the result sorts after `before`.
/// When both are `None`, a default midpoint key is returned.
///
/// Callers that cannot handle exhaustion (e.g. column reordering where exhaustion is
/// practically impossible) may fall back to `between`, which silently appends after
/// `before` instead of returning `None`.
pub fn try_between(before: Option<&str>, after: Option<&str>) -> Option<String> {
    let before_idx = before.and_then(|s| FractionalIndex::from_string(s).ok());
    let after_idx = after.and_then(|s| FractionalIndex::from_string(s).ok());

    let result = match (before_idx.as_ref(), after_idx.as_ref()) {
        (None, None) => FractionalIndex::default(),
        (Some(b), None) => FractionalIndex::new_after(b),
        (None, Some(a)) => FractionalIndex::new_before(a),
        (Some(b), Some(a)) => FractionalIndex::new_between(b, a)?,
    };

    Some(result.to_string())
}

/// Returns a position key that sorts between `before` and `after`.
///
/// When the fractional space is exhausted (i.e. `try_between` would return `None`),
/// this falls back to appending after `before` rather than erroring. Use `try_between`
/// when exhaustion must be surfaced as an error (e.g. task positioning).
pub fn between(before: Option<&str>, after: Option<&str>) -> String {
    let before_idx = before.and_then(|s| FractionalIndex::from_string(s).ok());
    let after_idx = after.and_then(|s| FractionalIndex::from_string(s).ok());

    let result = match (before_idx.as_ref(), after_idx.as_ref()) {
        (None, None) => FractionalIndex::default(),
        (Some(b), None) => FractionalIndex::new_after(b),
        (None, Some(a)) => FractionalIndex::new_before(a),
        (Some(b), Some(a)) => {
            FractionalIndex::new_between(b, a).unwrap_or_else(|| FractionalIndex::new_after(b))
        }
    };

    result.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn between_none_none_produces_midpoint() {
        let key = between(None, None);
        assert!(!key.is_empty());
    }

    #[test]
    fn between_two_keys_sorts_between_them() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        let mid = between(Some(&a), Some(&b));
        assert!(mid > a);
        assert!(mid < b);
    }

    #[test]
    fn sequential_keys_maintain_order() {
        let k1 = between(None, None);
        let k2 = between(Some(&k1), None);
        let k3 = between(Some(&k2), None);
        assert!(k1 < k2);
        assert!(k2 < k3);
    }

    #[test]
    fn try_between_returns_some_for_normal_inputs() {
        let a = between(None, None);
        let b = between(Some(&a), None);
        let mid = try_between(Some(&a), Some(&b));
        assert!(mid.is_some());
        let mid = mid.unwrap();
        assert!(mid > a);
        assert!(mid < b);
    }

    #[test]
    fn try_between_none_none_returns_default() {
        let key = try_between(None, None);
        assert!(key.is_some());
        assert!(!key.unwrap().is_empty());
    }

    #[test]
    fn try_between_before_only_returns_after_key() {
        let a = between(None, None);
        let result = try_between(Some(&a), None);
        assert!(result.is_some());
        assert!(result.unwrap() > a);
    }

    #[test]
    fn try_between_equal_keys_returns_none() {
        // Equal keys cannot produce a midpoint — try_between returns None.
        let key = between(None, None);
        let result = try_between(Some(&key), Some(&key));
        assert!(result.is_none());
    }
}
