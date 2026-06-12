use fractional_index::FractionalIndex;

/// Returns a position key that sorts between `before` and `after`.
///
/// When `before` is `None`, the result sorts before `after` (or at the default position
/// if `after` is also `None`). When `after` is `None`, the result sorts after `before`.
/// When both are `None`, a default midpoint key is returned.
///
/// The returned string is the hex representation of the fractional index byte string,
/// which preserves lexicographic ordering.
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
}
