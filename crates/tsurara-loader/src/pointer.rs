//! JSON Pointer (RFC 6901).

use serde_json::Value;

pub fn escape(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

pub fn unescape(token: &str) -> String {
    token.replace("~1", "/").replace("~0", "~")
}

/// Returns `pointer` with one `token` appended.
pub fn push(pointer: &str, token: &str) -> String {
    let mut s = String::with_capacity(pointer.len() + token.len() + 1);
    s.push_str(pointer);
    s.push('/');
    s.push_str(&escape(token));
    s
}

/// Splits a JSON Pointer into tokens. `""` points to the root and has zero tokens.
pub fn tokens(pointer: &str) -> impl Iterator<Item = String> + '_ {
    pointer
        .strip_prefix('/')
        .into_iter()
        .flat_map(|rest| rest.split('/'))
        .map(unescape)
}

pub fn get<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    if !pointer.is_empty() && !pointer.starts_with('/') {
        return None;
    }
    let mut current = value;
    for token in tokens(pointer) {
        current = match current {
            Value::Object(map) => map.get(&token)?,
            Value::Array(items) => {
                if token.len() > 1 && token.starts_with('0') {
                    return None;
                }
                items.get(token.parse::<usize>().ok()?)?
            }
            _ => return None,
        };
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn roundtrip() {
        let p = push(&push("", "paths"), "/pets/{id}");
        assert_eq!(p, "/paths/~1pets~1{id}");
        assert_eq!(tokens(&p).collect::<Vec<_>>(), ["paths", "/pets/{id}"]);
        assert_eq!(push("", "a~b"), "/a~0b");
        assert_eq!(tokens("/a~01").collect::<Vec<_>>(), ["a~1"]);
    }

    #[test]
    fn lookup() {
        let v = json!({ "a": [ { "b/c": 1 } ], "": 2 });
        assert_eq!(get(&v, ""), Some(&v));
        assert_eq!(get(&v, "/a/0/b~1c"), Some(&json!(1)));
        assert_eq!(get(&v, "/"), Some(&json!(2)));
        assert_eq!(get(&v, "/a/01"), None);
        assert_eq!(get(&v, "/x"), None);
    }
}
