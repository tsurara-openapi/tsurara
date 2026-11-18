//! The OperationDef format (spec/operation-def.schema.json). Generated code embeds it as JSON.

use indexmap::IndexMap;
use serde::Deserialize;

/// Version of the OperationDef format this runtime reads.
pub const OPERATION_DEF_VERSION: u32 = 1;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationDef {
    pub v: u32,
    pub id: String,
    pub method: String,
    pub path: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub params: ParamsDef,
    pub body: Option<BodyDef>,
    pub responses: IndexMap<String, ResponseDef>,
    #[serde(default)]
    pub security: Vec<Vec<SecurityUse>>,
}

impl OperationDef {
    /// Parses an OperationDef that generated code embeds. Panics on invalid JSON, which only a
    /// generator bug produces.
    pub fn from_json(json: &str) -> OperationDef {
        serde_json::from_str(json).expect("tsurara: invalid OperationDef")
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamsDef {
    #[serde(default)]
    pub path: IndexMap<String, ParamDef>,
    #[serde(default)]
    pub query: IndexMap<String, ParamDef>,
    #[serde(default)]
    pub headers: IndexMap<String, ParamDef>,
    #[serde(default)]
    pub cookies: IndexMap<String, ParamDef>,
    pub querystring: Option<QueryStringDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamDef {
    pub style: Option<String>,
    pub explode: Option<bool>,
    #[serde(default)]
    pub allow_reserved: bool,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryStringDef {
    pub content_type: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyDef {
    #[serde(default)]
    pub required: bool,
    /// Per media type; the first is the default.
    pub contents: IndexMap<String, MediaDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaDef {
    #[serde(default)]
    pub encoding: IndexMap<String, EncodingDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingDef {
    pub content_type: Option<String>,
    pub style: Option<String>,
    pub explode: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ParseAs {
    Json,
    Text,
    Blob,
    Stream,
    /// Server-Sent Events, read item by item ([`crate::Items`]).
    Events,
    /// JSON Lines, read item by item.
    JsonLines,
    /// JSON text sequences, read item by item.
    JsonSeq,
    None,
    Auto,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseDef {
    pub parse_as: ParseAs,
    #[serde(default)]
    pub content_types: IndexMap<String, ParseAs>,
    /// With `events`: `true`, or the names of the events whose data is JSON.
    #[serde(default)]
    pub json_events: Option<serde_json::Value>,
}

/// A security scheme an authentication alternative uses.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SecurityUse {
    /// The name in `components.securitySchemes`.
    pub scheme: String,
    /// `apiKey`, `http`, `oauth2`, `openIdConnect` or `mutualTLS`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `apiKey`: `header`, `query` or `cookie`.
    #[serde(rename = "in")]
    pub location: Option<String>,
    /// `apiKey`: the header, query or cookie name.
    pub name: Option<String>,
    /// `http`: the Authorization scheme.
    pub http_scheme: Option<String>,
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// The operation of a request, for hooks and errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationInfo {
    pub id: String,
    pub method: String,
    pub path: String,
    pub tags: Vec<String>,
}

impl From<&OperationDef> for OperationInfo {
    fn from(def: &OperationDef) -> Self {
        OperationInfo {
            id: def.id.clone(),
            method: def.method.clone(),
            path: def.path.clone(),
            tags: def.tags.clone(),
        }
    }
}
