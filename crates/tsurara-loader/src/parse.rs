use crate::yaml::{YamlError, parse_yaml};
use serde_json::Value;

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the document is not valid UTF-8")]
    Utf8(#[from] std::str::Utf8Error),
    #[error("failed to parse as JSON ({json}) and as YAML ({yaml})")]
    JsonAndYaml {
        json: serde_json::Error,
        yaml: YamlError,
    },
    #[error("failed to parse as YAML: {0}")]
    Yaml(#[from] YamlError),
}

/// Parses a body into a JSON value. Reads it as JSON if it starts with `{` or `[`, and as YAML if that fails.
pub fn parse_document(bytes: &[u8]) -> Result<Value, ParseError> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    let text = std::str::from_utf8(bytes)?;
    let trimmed = text.trim_start();
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        match serde_json::from_str(trimmed) {
            Ok(v) => Ok(v),
            Err(json) => parse_yaml(text).map_err(|yaml| ParseError::JsonAndYaml { json, yaml }),
        }
    } else {
        Ok(parse_yaml(text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn json() {
        assert_eq!(
            parse_document(br#"{"b":1,"a":2}"#).unwrap(),
            json!({"b":1,"a":2})
        );
        let v = parse_document(br#"{"b":1,"a":2}"#).unwrap();
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["b", "a"], "key order is preserved");
    }

    #[test]
    fn bom_is_removed() {
        assert_eq!(
            parse_document(b"\xEF\xBB\xBF{\"a\":1}").unwrap(),
            json!({"a":1})
        );
        assert_eq!(
            parse_document(b"\xEF\xBB\xBFa: 1\n").unwrap(),
            json!({"a":1})
        );
    }

    #[test]
    fn invalid_json_falls_back_to_yaml() {
        assert_eq!(
            parse_document(b"{a: 1, 200: x}").unwrap(),
            json!({"a":1,"200":"x"})
        );
    }

    #[test]
    fn both_fail() {
        assert!(matches!(
            parse_document(b"{\"a\": [}"),
            Err(ParseError::JsonAndYaml { .. })
        ));
    }
}
