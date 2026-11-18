use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A field that may be missing, `null`, or a value (optional and nullable, design.md 18.16 Q3).
/// Use with `#[serde(default, skip_serializing_if = "Nullable::is_missing")]`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub enum Nullable<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}

impl<T> Nullable<T> {
    pub fn is_missing(&self) -> bool {
        matches!(self, Nullable::Missing)
    }

    /// The value, if there is one.
    pub fn as_option(&self) -> Option<&T> {
        match self {
            Nullable::Value(v) => Some(v),
            _ => None,
        }
    }
}

impl<T: Serialize> Serialize for Nullable<T> {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Nullable::Value(v) => v.serialize(s),
            _ => s.serialize_none(),
        }
    }
}

impl<'de, T: Deserialize<'de>> Deserialize<'de> for Nullable<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match Option::<T>::deserialize(d)? {
            Some(v) => Nullable::Value(v),
            None => Nullable::Null,
        })
    }
}
