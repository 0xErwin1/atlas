/// Splits a document into its YAML frontmatter block (if present) and the body.
///
/// Returns `(Some(yaml_str), body)` when the content begins with `---\n` (or
/// `---\r\n`) followed by a closing `---` line. Returns `(None, full_content)`
/// when no such block is found.
pub fn strip_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let after_open = if let Some(rest) = raw.strip_prefix("---\n") {
        rest
    } else if let Some(rest) = raw.strip_prefix("---\r\n") {
        rest
    } else {
        return (None, raw);
    };

    if let Some(rest) = after_open.strip_prefix("---\n") {
        return (Some(""), rest);
    }
    if after_open.strip_prefix("---\r\n").is_some() {
        let rest = &after_open["---\r\n".len()..];
        return (Some(""), rest);
    }
    if after_open == "---" {
        return (Some(""), "");
    }

    let close_nl = "\n---\n";
    let close_crnl = "\n---\r\n";
    let close_end = "\n---";

    if let Some(yaml_end) = after_open.find(close_crnl) {
        let yaml = &after_open[..yaml_end];
        let body_start = yaml_end + close_crnl.len();
        let body = &after_open[body_start..];
        return (Some(yaml), body);
    }

    if let Some(yaml_end) = after_open.find(close_nl) {
        let yaml = &after_open[..yaml_end];
        let body_start = yaml_end + close_nl.len();
        let body = &after_open[body_start..];
        return (Some(yaml), body);
    }

    if after_open.ends_with(close_end) {
        let yaml_end = after_open.len() - close_end.len();
        let yaml = &after_open[..yaml_end];
        return (Some(yaml), "");
    }

    (None, raw)
}

/// Parses a YAML frontmatter block into a JSON value.
///
/// Only scalar strings, integers, booleans, and null are supported; nested
/// mappings and sequences are stored as raw strings rather than being
/// recursively parsed. This avoids any dependency on `serde_yaml`.
pub fn parse_frontmatter_yaml(yaml: &str) -> serde_json::Value {
    let mut map = serde_json::Map::new();

    for line in yaml.lines() {
        if let Some((key, raw_value)) = line.split_once(':') {
            let key = key.trim().to_string();
            if key.is_empty() {
                continue;
            }

            let raw_value = raw_value.trim();
            let value = parse_scalar(raw_value);
            map.insert(key, value);
        }
    }

    serde_json::Value::Object(map)
}

fn parse_scalar(raw: &str) -> serde_json::Value {
    if raw.is_empty() || raw == "null" || raw == "~" {
        return serde_json::Value::Null;
    }

    if raw == "true" {
        return serde_json::Value::Bool(true);
    }
    if raw == "false" {
        return serde_json::Value::Bool(false);
    }

    if let Ok(n) = raw.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }

    if let Ok(f) = raw.parse::<f64>()
        && let Some(num) = serde_json::Number::from_f64(f)
    {
        return serde_json::Value::Number(num);
    }

    let unquoted = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .or_else(|| raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')))
        .unwrap_or(raw);

    serde_json::Value::String(unquoted.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frontmatter_extracted_and_body_returned() {
        let raw = "---\nstatus: draft\n---\n# Body";
        let (yaml, body) = strip_frontmatter(raw);
        assert_eq!(yaml, Some("status: draft"));
        assert_eq!(body, "# Body");
    }

    #[test]
    fn no_leading_dashes_returns_none_yaml_and_full_content() {
        let raw = "# Just a heading\nsome content";
        let (yaml, body) = strip_frontmatter(raw);
        assert!(yaml.is_none());
        assert_eq!(body, raw);
    }

    #[test]
    fn empty_yaml_block_returns_empty_string() {
        let raw = "---\n---\n# Body";
        let (yaml, body) = strip_frontmatter(raw);
        assert_eq!(yaml, Some(""));
        assert_eq!(body, "# Body");
    }

    #[test]
    fn empty_yaml_block_parses_to_empty_object() {
        let result = parse_frontmatter_yaml("");
        assert_eq!(result, json!({}));
    }

    #[test]
    fn string_scalar_value_parsed() {
        let result = parse_frontmatter_yaml("title: My Document");
        assert_eq!(result["title"], json!("My Document"));
    }

    #[test]
    fn integer_scalar_value_parsed() {
        let result = parse_frontmatter_yaml("priority: 5");
        assert_eq!(result["priority"], json!(5));
    }

    #[test]
    fn boolean_scalar_value_parsed() {
        let result = parse_frontmatter_yaml("draft: true\npublished: false");
        assert_eq!(result["draft"], json!(true));
        assert_eq!(result["published"], json!(false));
    }

    #[test]
    fn nested_value_stored_as_raw_string() {
        let result = parse_frontmatter_yaml("tags: [a, b, c]");
        assert_eq!(result["tags"], json!("[a, b, c]"));
    }

    #[test]
    fn full_parse_roundtrip() {
        let raw = "---\nstatus: draft\nversion: 2\n---\n# Body content here";
        let (yaml, body) = strip_frontmatter(raw);
        let yaml_str = yaml.expect("yaml block must be present");
        let parsed = parse_frontmatter_yaml(yaml_str);

        assert_eq!(parsed["status"], json!("draft"));
        assert_eq!(parsed["version"], json!(2));
        assert_eq!(body, "# Body content here");
    }

    #[test]
    fn null_scalar_value_parsed() {
        let result = parse_frontmatter_yaml("owner: null");
        assert_eq!(result["owner"], json!(null));
    }
}
