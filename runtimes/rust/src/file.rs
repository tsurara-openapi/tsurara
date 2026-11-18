//! Binary values (types layer): generated types hold them in properties with `format:
//! binary`, so their serde implementations work without the client.

use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cell::RefCell;

/// A binary value with an optional file name and media type.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct File {
    pub content: Vec<u8>,
    pub filename: Option<String>,
    pub content_type: Option<String>,
}

thread_local! {
    /// Files met while a multipart body is serialized; `None` outside of one.
    pub(crate) static FILES: RefCell<Option<Vec<File>>> = const { RefCell::new(None) };
}

/// The key of the object a file serializes to inside a multipart body.
pub(crate) const MARKER: &str = "\u{0}tsurara-file";

/// Outside a multipart body, a file is its bytes (an array of numbers, as `Vec<u8>`).
impl Serialize for File {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let index = FILES.with(|f| {
            f.borrow_mut().as_mut().map(|files| {
                files.push(self.clone());
                files.len() - 1
            })
        });
        match index {
            Some(i) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(MARKER, &i)?;
                map.end()
            }
            None => serializer.collect_seq(&self.content),
        }
    }
}

impl<'de> Deserialize<'de> for File {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Vec::<u8>::deserialize(deserializer).map(|content| File {
            content,
            ..File::default()
        })
    }
}
