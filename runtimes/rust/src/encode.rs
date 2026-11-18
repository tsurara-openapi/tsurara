//! Typed values into an [`Input`]: generated functions call these methods with their arguments.
//!
//! Values become JSON values that the OperationDef then serializes. A value that fails to encode
//! (only possible with hand-written `Serialize` implementations) is kept as `encode_error`, and the
//! request returns `Failure::Unexpected` without being sent.

use crate::file::{FILES, MARKER};
use crate::{Body, File, Input, Part};
use serde::Serialize;
use serde_json::Value;

/// Resets the file list when a multipart serialization ends, also by a panic.
struct Collecting;

impl Collecting {
    fn start() -> Collecting {
        FILES.with(|f| *f.borrow_mut() = Some(Vec::new()));
        Collecting
    }

    fn files(&self) -> Vec<File> {
        FILES.with(|f| f.borrow_mut().take().unwrap_or_default())
    }
}

impl Drop for Collecting {
    fn drop(&mut self) {
        FILES.with(|f| *f.borrow_mut() = None);
    }
}

fn file_index(v: &Value) -> Option<usize> {
    match v {
        Value::Object(m) if m.len() == 1 => m.get(MARKER)?.as_u64().map(|i| i as usize),
        _ => None,
    }
}

impl Input {
    fn encode(&mut self, value: &impl Serialize) -> Option<Value> {
        match serde_json::to_value(value) {
            Ok(v) => Some(v),
            Err(e) => {
                self.encode_error.get_or_insert_with(|| e.to_string());
                None
            }
        }
    }

    pub fn path(&mut self, name: &str, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.path.push((name.to_owned(), v));
        }
        self
    }

    pub fn query(&mut self, name: &str, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.query.push((name.to_owned(), v));
        }
        self
    }

    pub fn header(&mut self, name: &str, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.headers.push((name.to_owned(), v));
        }
        self
    }

    pub fn cookie(&mut self, name: &str, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.cookies.push((name.to_owned(), v));
        }
        self
    }

    /// The whole query string of an `in: querystring` parameter.
    pub fn querystring(&mut self, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.querystring = Some(v);
        }
        self
    }

    /// A body that the media type serializes: JSON, form, text.
    pub fn value_body(&mut self, value: &impl Serialize) -> &mut Self {
        if let Some(v) = self.encode(value) {
            self.body = Some(Body::Value(v));
        }
        self
    }

    /// A binary body.
    pub fn binary_body(&mut self, file: File) -> &mut Self {
        self.body = Some(Body::Binary(file));
        self
    }

    /// A multipart body from a struct: one part per field, in order. Files (also in lists, one
    /// part each) are file parts; other values are serialized by the encoding of their part.
    pub fn multipart_body(&mut self, value: &impl Serialize) -> &mut Self {
        let collecting = Collecting::start();
        let encoded = self.encode(value);
        let files = collecting.files();
        drop(collecting);
        let Some(encoded) = encoded else {
            return self;
        };
        let Value::Object(fields) = encoded else {
            self.encode_error
                .get_or_insert_with(|| "a multipart body must be an object".into());
            return self;
        };
        let file = |i: usize| Part::File(files[i].clone());
        let mut parts = Vec::new();
        for (name, v) in fields {
            if let Some(i) = file_index(&v) {
                parts.push((name, file(i)));
            } else if let Value::Array(items) = &v {
                let indices: Option<Vec<usize>> = items.iter().map(file_index).collect();
                match indices {
                    Some(indices) if !indices.is_empty() => {
                        parts.extend(indices.into_iter().map(|i| (name.clone(), file(i))));
                    }
                    _ => parts.push((name, Part::Value(v))),
                }
            } else {
                parts.push((name, Part::Value(v)));
            }
        }
        self.body = Some(Body::Multipart(parts));
        self
    }

    /// The request media type, for operations with several.
    pub fn content_type(&mut self, media_type: &str) -> &mut Self {
        self.content_type = Some(media_type.to_owned());
        self
    }
}
