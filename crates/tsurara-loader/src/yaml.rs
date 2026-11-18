//! Converts YAML into JSON values.
//!
//! Keys become strings as written (`200:` becomes `"200"`). Merge keys (`<<`) are expanded.

use serde_json::{Map, Number, Value};
use std::collections::{HashMap, HashSet};
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

#[derive(Debug, thiserror::Error)]
pub enum YamlError {
    #[error("{0}")]
    Scan(#[from] yaml_rust2::ScanError),
    #[error("line {line}: {message}")]
    Structure { line: usize, message: String },
    #[error("the document is empty")]
    Empty,
}

pub fn parse_yaml(text: &str) -> Result<Value, YamlError> {
    let mut builder = Builder::default();
    let mut parser = Parser::new_from_str(text);
    parser.load(&mut builder, false)?;
    if let Some(err) = builder.error {
        return Err(err);
    }
    builder.root.ok_or(YamlError::Empty)
}

enum Entry {
    Pair(String, Value),
    Merge(Vec<Map<String, Value>>),
}

enum Frame {
    Seq {
        items: Vec<Value>,
        anchor: usize,
    },
    Map {
        entries: Vec<Entry>,
        key: Option<Key>,
        anchor: usize,
    },
}

enum Key {
    Name(String),
    Merge,
}

/// Aliases may add at most this many nodes, or this many per node of the document if that is more.
/// Real definitions stay far below; nested aliases ("billion laughs") would exhaust memory.
const ALIAS_NODES: usize = 1_000_000;
const ALIAS_NODES_PER_EVENT: usize = 100;

#[derive(Default)]
struct Builder {
    stack: Vec<Frame>,
    anchors: HashMap<usize, Value>,
    /// Number of nodes in each anchored value.
    anchor_sizes: HashMap<usize, usize>,
    /// Nodes added by expanding aliases so far.
    expanded: usize,
    /// Events read so far.
    events: usize,
    root: Option<Value>,
    error: Option<YamlError>,
    done: bool,
}

impl Builder {
    fn fail(&mut self, mark: Marker, message: impl Into<String>) {
        if self.error.is_none() {
            self.error = Some(YamlError::Structure {
                line: mark.line(),
                message: message.into(),
            });
        }
    }

    fn expecting_key(&self) -> bool {
        matches!(self.stack.last(), Some(Frame::Map { key: None, .. }))
    }

    fn push_value(&mut self, value: Value, anchor: usize, mark: Marker) {
        if anchor != 0 {
            self.anchor_sizes.insert(anchor, nodes(&value));
            self.anchors.insert(anchor, value.clone());
        }
        match self.stack.last_mut() {
            None => {
                self.root = Some(value);
                self.done = true;
            }
            Some(Frame::Seq { items, .. }) => items.push(value),
            Some(Frame::Map { entries, key, .. }) => match key.take() {
                None => unreachable!("keys are handled before values"),
                Some(Key::Name(name)) => entries.push(Entry::Pair(name, value)),
                Some(Key::Merge) => {
                    let maps = match value {
                        Value::Object(map) => vec![map],
                        Value::Array(items) => {
                            let mut maps = Vec::with_capacity(items.len());
                            for item in items {
                                match item {
                                    Value::Object(map) => maps.push(map),
                                    _ => {
                                        self.fail(mark, "a merge key (<<) must refer to mappings");
                                        return;
                                    }
                                }
                            }
                            maps
                        }
                        _ => {
                            self.fail(mark, "a merge key (<<) must refer to mappings");
                            return;
                        }
                    };
                    entries.push(Entry::Merge(maps));
                }
            },
        }
    }

    fn set_key(&mut self, key: Key) {
        if let Some(Frame::Map { key: slot, .. }) = self.stack.last_mut() {
            *slot = Some(key);
        }
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() || self.done {
            return;
        }
        self.events += 1;
        match ev {
            Event::Scalar(text, style, anchor, tag) => {
                if self.expecting_key() {
                    let key = if style == TScalarStyle::Plain && text == "<<" && tag.is_none() {
                        Key::Merge
                    } else {
                        Key::Name(text.clone())
                    };
                    if anchor != 0 {
                        self.anchors.insert(anchor, Value::String(text));
                    }
                    self.set_key(key);
                } else {
                    let value = resolve_scalar(text, style, tag.as_ref());
                    self.push_value(value, anchor, mark);
                }
            }
            Event::Alias(id) => {
                self.expanded += self.anchor_sizes.get(&id).copied().unwrap_or(1);
                let limit = ALIAS_NODES.max(self.events * ALIAS_NODES_PER_EVENT);
                if self.expanded > limit {
                    self.fail(
                        mark,
                        format!(
                            "aliases expand to more than {limit} nodes; the document is refused"
                        ),
                    );
                    return;
                }
                let Some(value) = self.anchors.get(&id).cloned() else {
                    self.fail(mark, "unknown alias");
                    return;
                };
                if self.expecting_key() {
                    match value {
                        Value::String(s) => self.set_key(Key::Name(s)),
                        Value::Number(n) => self.set_key(Key::Name(n.to_string())),
                        Value::Bool(b) => self.set_key(Key::Name(b.to_string())),
                        _ => self.fail(mark, "a mapping key must be a scalar"),
                    }
                } else {
                    self.push_value(value, 0, mark);
                }
            }
            Event::SequenceStart(anchor, _) => {
                if self.expecting_key() {
                    self.fail(mark, "a mapping key must be a scalar");
                    return;
                }
                self.stack.push(Frame::Seq {
                    items: Vec::new(),
                    anchor,
                });
            }
            Event::MappingStart(anchor, _) => {
                if self.expecting_key() {
                    self.fail(mark, "a mapping key must be a scalar");
                    return;
                }
                self.stack.push(Frame::Map {
                    entries: Vec::new(),
                    key: None,
                    anchor,
                });
            }
            Event::SequenceEnd => {
                if let Some(Frame::Seq { items, anchor }) = self.stack.pop() {
                    self.push_value(Value::Array(items), anchor, mark);
                }
            }
            Event::MappingEnd => {
                if let Some(Frame::Map {
                    entries, anchor, ..
                }) = self.stack.pop()
                {
                    self.push_value(Value::Object(build_map(entries)), anchor, mark);
                }
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => {}
        }
    }
}

/// The number of nodes in a value: itself and everything in it.
fn nodes(value: &Value) -> usize {
    match value {
        Value::Array(items) => 1 + items.iter().map(nodes).sum::<usize>(),
        Value::Object(map) => 1 + map.values().map(nodes).sum::<usize>(),
        _ => 1,
    }
}

/// Explicit keys take precedence over merged keys. Merged keys go where `<<` was written.
fn build_map(entries: Vec<Entry>) -> Map<String, Value> {
    let explicit: HashSet<String> = entries
        .iter()
        .filter_map(|e| match e {
            Entry::Pair(k, _) => Some(k.clone()),
            Entry::Merge(_) => None,
        })
        .collect();
    let mut map = Map::new();
    for entry in entries {
        match entry {
            Entry::Pair(k, v) => {
                map.insert(k, v);
            }
            Entry::Merge(maps) => {
                for m in maps {
                    for (k, v) in m {
                        if !explicit.contains(&k) && !map.contains_key(&k) {
                            map.insert(k, v);
                        }
                    }
                }
            }
        }
    }
    map
}

fn resolve_scalar(text: String, style: TScalarStyle, tag: Option<&Tag>) -> Value {
    if style != TScalarStyle::Plain {
        return Value::String(text);
    }
    if let Some(tag) = tag {
        if tag.handle == "tag:yaml.org,2002:" || tag.handle == "!!" {
            match tag.suffix.as_str() {
                "str" => return Value::String(text),
                "int" | "float" => {
                    return number(&text).map_or(Value::String(text), Value::Number);
                }
                "bool" => {
                    return match text.as_str() {
                        "true" | "True" | "TRUE" => Value::Bool(true),
                        "false" | "False" | "FALSE" => Value::Bool(false),
                        _ => Value::String(text),
                    };
                }
                "null" => return Value::Null,
                _ => {}
            }
        }
        return Value::String(text);
    }
    match text.as_str() {
        "" | "~" | "null" | "Null" | "NULL" => Value::Null,
        "true" | "True" | "TRUE" => Value::Bool(true),
        "false" | "False" | "FALSE" => Value::Bool(false),
        _ => number(&text).map_or(Value::String(text), Value::Number),
    }
}

/// Reads numbers per the YAML 1.2 core schema. `.inf` and `.nan`, which JSON cannot represent, are not numbers.
fn number(text: &str) -> Option<Number> {
    if let Some(hex) = text.strip_prefix("0x") {
        return i64::from_str_radix(hex, 16).ok().map(Number::from);
    }
    if let Some(oct) = text.strip_prefix("0o") {
        return i64::from_str_radix(oct, 8).ok().map(Number::from);
    }
    let body = text.strip_prefix(['+', '-']).unwrap_or(text);
    if body.is_empty() || !body.starts_with(|c: char| c.is_ascii_digit() || c == '.') {
        return None;
    }
    if body.bytes().all(|b| b.is_ascii_digit()) {
        let unsigned = text.strip_prefix('+').unwrap_or(text);
        if let Ok(i) = unsigned.parse::<i64>() {
            return Some(Number::from(i));
        }
        if let Ok(u) = unsigned.parse::<u64>() {
            return Some(Number::from(u));
        }
    }
    let valid = body
        .bytes()
        .all(|b| b.is_ascii_digit() || matches!(b, b'.' | b'e' | b'E' | b'+' | b'-'));
    if !valid {
        return None;
    }
    text.parse::<f64>().ok().and_then(Number::from_f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::{Duration, Instant};

    /// Each anchor refers to the previous one twice: 2^25 copies of `lol` (a "billion laughs").
    fn laughs() -> String {
        let mut s = String::from("a0: &a0 [lol]\n");
        for i in 1..26 {
            s.push_str(&format!("a{i}: &a{i} [*a{}, *a{}]\n", i - 1, i - 1));
        }
        s
    }

    #[test]
    fn alias_expansion_is_limited() {
        let start = Instant::now();
        let err = parse_yaml(&laughs()).unwrap_err().to_string();
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "{:?}",
            start.elapsed()
        );
        assert!(err.contains("aliases expand"), "{err}");
        assert!(err.starts_with("line "), "{err}");
    }

    #[test]
    fn ordinary_aliases_and_merge_keys_still_work() {
        let v = parse_yaml("base: &b { x: 1 }\nuse: [*b, *b]\nmerged: { <<: *b, y: 2 }\n").unwrap();
        assert_eq!(v["use"], json!([{ "x": 1 }, { "x": 1 }]));
        assert_eq!(v["merged"], json!({ "x": 1, "y": 2 }));
    }

    #[test]
    fn numeric_and_boolean_keys_become_strings() {
        let v = parse_yaml("200:\n  a: 1\n404: x\ntrue: y\n1.10: z\n012: w\n").unwrap();
        assert_eq!(
            v,
            json!({ "200": { "a": 1 }, "404": "x", "true": "y", "1.10": "z", "012": "w" })
        );
        let keys: Vec<_> = v.as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["200", "404", "true", "1.10", "012"]);
    }

    #[test]
    fn scalars() {
        let v = parse_yaml(
            "a: 1\nb: -2.5\nc: 'x'\nd: ~\ne: false\nf: .inf\ng: 1e3\nh: 0x10\ni: \"1\"\nj: 18446744073709551615\nk: 1.2.3\n",
        )
        .unwrap();
        assert_eq!(
            v,
            json!({
                "a": 1, "b": -2.5, "c": "x", "d": null, "e": false, "f": ".inf",
                "g": 1000.0, "h": 16, "i": "1", "j": 18446744073709551615u64, "k": "1.2.3"
            })
        );
    }

    #[test]
    fn merge_keys() {
        let v = parse_yaml(
            "base: &b\n  x: 1\n  y: 2\nother: &o\n  z: 3\nchild:\n  y: 20\n  <<: [*b, *o]\n  w: 4\n",
        )
        .unwrap();
        assert_eq!(v["child"], json!({ "y": 20, "x": 1, "z": 3, "w": 4 }));
        let keys: Vec<_> = v["child"].as_object().unwrap().keys().cloned().collect();
        assert_eq!(keys, ["y", "x", "z", "w"]);
    }

    #[test]
    fn quoted_merge_key_is_a_normal_key() {
        let v = parse_yaml("a:\n  '<<': 1\n").unwrap();
        assert_eq!(v, json!({ "a": { "<<": 1 } }));
    }

    #[test]
    fn aliases_are_copied() {
        let v = parse_yaml("a: &x [1, 2]\nb: *x\n").unwrap();
        assert_eq!(v, json!({ "a": [1, 2], "b": [1, 2] }));
    }

    #[test]
    fn complex_keys_are_rejected() {
        assert!(parse_yaml("? [a]\n: 1\n").is_err());
    }

    #[test]
    fn empty_document() {
        assert!(matches!(parse_yaml(""), Err(YamlError::Empty)));
    }
}
