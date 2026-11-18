//! IR of operations (design.md 6.2, 7.4).

use crate::ir::{Choice, Discriminator, Doc, Inline, Object, Property, Scalar, ScalarKind, Type};
use crate::naming::{NameId, NameKind};
use crate::schema::{Converter, Mode, schema_doc};
use crate::warning::WarningKind;
use serde_json::Value;
use std::collections::BTreeMap;
use tsurara_loader::Loc;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum ParamIn {
    Path,
    Query,
    Header,
    Cookie,
    /// `in: querystring` from 3.2. Represents the whole query string as one value.
    QueryString,
}

impl ParamIn {
    /// The location of `in` (`query`, `header`, `path`, `cookie`, `querystring`).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "path" => ParamIn::Path,
            "query" => ParamIn::Query,
            "header" => ParamIn::Header,
            "cookie" => ParamIn::Cookie,
            "querystring" => ParamIn::QueryString,
            _ => return None,
        })
    }

    /// Group name in the `Data` type and the OperationDef.
    pub fn group(self) -> &'static str {
        match self {
            ParamIn::Path => "path",
            ParamIn::Query => "query",
            ParamIn::Header => "headers",
            ParamIn::Cookie => "cookies",
            ParamIn::QueryString => "querystring",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Param {
    pub name: String,
    pub location: ParamIn,
    pub required: bool,
    pub ty: Type,
    pub doc: Option<Doc>,
    pub style: Option<String>,
    pub explode: Option<bool>,
    pub allow_reserved: bool,
    /// Media type of a parameter given with `content`. The value is serialized in this format.
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Encoding {
    pub property: String,
    pub content_type: Option<String>,
    pub style: Option<String>,
    pub explode: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct MediaContent {
    pub media_type: String,
    pub ty: Type,
    pub encoding: Vec<Encoding>,
    pub body_kind: BodyKind,
    /// The events of a `text/event-stream` response whose media type has an item schema. The body
    /// kind stays `stream`; a backend whose runtime reads events uses this instead (design.md 8.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Events>,
    /// The items of a JSON Lines or JSON text sequence response whose media type has an item
    /// schema. The body kind stays `stream`, as with `events` (design.md 8.5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub items: Option<Items>,
}

/// The items of a sequential JSON response, from `itemSchema` (3.2) or `x-itemSchema`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Items {
    /// The type of one item, in the JSON context.
    pub ty: Type,
    pub framing: Framing,
}

/// How the items of a sequential JSON body are separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Framing {
    /// One JSON text per line (`application/jsonl`, `application/x-ndjson`, ...).
    Lines,
    /// JSON texts each starting with the record separator U+001E (`application/json-seq`, RFC 7464).
    RecordSeparator,
}

impl Framing {
    /// The framing of a sequential JSON media type, or `None` for any other media type.
    pub fn of(media_type: &str) -> Option<Framing> {
        match essence(media_type).to_ascii_lowercase().as_str() {
            "application/jsonl"
            | "application/x-ndjson"
            | "application/x-jsonlines"
            | "application/jsonlines" => Some(Framing::Lines),
            "application/json-seq" => Some(Framing::RecordSeparator),
            _ => None,
        }
    }

    /// The `parseAs` of a runtime that reads the items.
    pub fn parse_as(self) -> &'static str {
        match self {
            Framing::Lines => "jsonLines",
            Framing::RecordSeparator => "jsonSeq",
        }
    }
}

/// Gives an inline schema a preferred name of its own instead of the one derived from its path.
fn named_inline(ty: Type, name: String) -> Type {
    match ty {
        Type::Inline(mut i) => {
            i.preferred_name = name;
            Type::Inline(i)
        }
        Type::Union(ts) => Type::Union(
            ts.into_iter()
                .map(|t| match t {
                    Type::Inline(_) => named_inline(t, name.clone()),
                    other => other,
                })
                .collect(),
        ),
        other => other,
    }
}

/// The key of an item schema in a media type object: `itemSchema` (3.2), or `x-itemSchema` for
/// documents of earlier versions.
fn item_schema_key(media: &Value) -> Option<&'static str> {
    ["itemSchema", "x-itemSchema"]
        .into_iter()
        .find(|k| media.get(*k).is_some())
}

/// The events of a Server-Sent Events stream, from `itemSchema` (3.2) or `x-itemSchema`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Events {
    /// One event as a runtime hands it over: an object with `event`, `data`, `id` and `retry`, or a
    /// union of such objects. `data` has the type of its `contentSchema` where it is JSON.
    pub ty: Type,
    /// Which events carry JSON in `data`; `None` when none does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub json: Option<JsonEvents>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum JsonEvents {
    /// Every event.
    All,
    /// The events with these names.
    Named(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Body {
    pub required: bool,
    pub contents: Vec<MediaContent>,
    pub doc: Option<Doc>,
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum Status {
    Code(u16),
    /// A range such as `2XX`. The value is the leading digit.
    Range(u8),
    Default,
}

impl Status {
    fn parse(s: &str) -> Option<Self> {
        if s == "default" {
            return Some(Status::Default);
        }
        let b = s.as_bytes();
        if b.len() == 3 && b[0].is_ascii_digit() && b[1..].eq_ignore_ascii_case(b"XX") {
            return Some(Status::Range(b[0] - b'0'));
        }
        s.parse::<u16>()
            .ok()
            .filter(|c| (100..600).contains(c))
            .map(Status::Code)
    }

    /// Whether it is a success (Responses). 3xx and below are successes, 4xx and 5xx are errors. `default` is neither.
    pub fn is_success(self) -> Option<bool> {
        match self {
            Status::Code(c) => Some(c < 400),
            Status::Range(r) => Some(r < 4),
            Status::Default => None,
        }
    }

    /// Key in the `Responses` type and the OperationDef.
    pub fn key(self) -> String {
        match self {
            Status::Code(c) => c.to_string(),
            Status::Range(r) => format!("{r}XX"),
            Status::Default => "default".to_owned(),
        }
    }
}

/// How to parse a response body (design.md 8.5).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum BodyKind {
    Json,
    Text,
    Blob,
    Stream,
    None,
    /// Decided by the actual Content-Type (wildcard media types with a schema that is not binary).
    Auto,
}

impl BodyKind {
    pub fn as_str(self) -> &'static str {
        match self {
            BodyKind::Json => "json",
            BodyKind::Text => "text",
            BodyKind::Blob => "blob",
            BodyKind::Stream => "stream",
            BodyKind::None => "none",
            BodyKind::Auto => "auto",
        }
    }
}

/// The media type without its parameters.
fn essence(media_type: &str) -> &str {
    media_type.split(';').next().unwrap_or("").trim()
}

/// Decides how to parse from the media type.
pub fn body_kind_of(media_type: &str) -> BodyKind {
    let mt = essence(media_type).to_ascii_lowercase();
    let (kind, sub) = mt.split_once('/').unwrap_or((mt.as_str(), ""));
    const STREAMS: &[&str] = &[
        "text/event-stream",
        "application/jsonl",
        "application/x-ndjson",
        "application/x-jsonlines",
        "application/json-seq",
        "application/jsonlines",
    ];
    if STREAMS.contains(&mt.as_str()) {
        return BodyKind::Stream;
    }
    if mt == "application/json" || sub.ends_with("+json") {
        return BodyKind::Json;
    }
    if kind == "text"
        || mt == "application/xml"
        || sub.ends_with("+xml")
        || mt == "application/x-www-form-urlencoded"
        || mt == "application/javascript"
        || mt == "application/yaml"
    {
        return BodyKind::Text;
    }
    BodyKind::Blob
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Response {
    pub status: Status,
    pub description: Option<String>,
    pub contents: Vec<MediaContent>,
    pub headers: Vec<Property>,
}

impl Response {
    /// Body type. A union if there are several media types, `undefined` if there is no body.
    pub fn data_type(&self) -> Type {
        if self.contents.is_empty() {
            return Type::Undefined;
        }
        Type::union(self.contents.iter().map(|c| c.ty.clone()).collect())
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Callback {
    pub name: String,
    pub operations: Vec<Operation>,
}

/// A security scheme an authentication alternative of an operation uses, with what a runtime needs
/// to place the credential (spec/operation-def.schema.json).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct SecurityUse {
    /// The name in `components.securitySchemes`.
    pub scheme: String,
    /// `apiKey`, `http`, `oauth2`, `openIdConnect` or `mutualTLS`.
    #[serde(rename = "type")]
    pub kind: String,
    /// `apiKey`: `header`, `query` or `cookie`.
    #[serde(rename = "in", default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// `apiKey`: the header, query or cookie name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// `http`: the Authorization scheme (`bearer`, `basic`, ...).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_scheme: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
}

/// An event channel of `x-event-channels`: messages such as the updates of a Mercure hub,
/// which are not the response of an operation.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Channel {
    /// The key in `x-event-channels`.
    pub name: String,
    pub name_id: NameId,
    /// The identifier a backend assigned. Empty in the model; backends fill it in their own copy.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ident: String,
    /// Type names derived from the channel, by template (`{P}Message`). Empty in the model, like
    /// `ident`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub type_names: BTreeMap<String, String>,
    /// `summary` and `description`.
    pub doc: Option<Doc>,
    /// The topic template (`board/{boardId}`), relative to the deployment's topic prefix.
    pub channel: String,
    /// The template's variables with their types, in the order of `parameters`.
    pub parameters: Vec<ChannelParam>,
    /// Whether the hub requires a subscriber claim; informational.
    pub private: bool,
    /// The message payload by media type, as a request or response body's content.
    pub contents: Vec<MediaContent>,
    pub loc: Loc,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct ChannelParam {
    pub name: String,
    pub ty: Type,
}

/// The `{name}` variables of a channel template, in order.
fn template_variables(template: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else {
            break;
        };
        let name = &rest[start + 1..start + end];
        if !name.is_empty() && !out.iter().any(|v| v == name) {
            out.push(name.to_owned());
        }
        rest = &rest[start + end + 1..];
    }
    out
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    /// The `operationId`, or a name built from the method and path if absent.
    pub id: String,
    /// The name request of the operation.
    pub name_id: NameId,
    /// Name of the call function. Empty in the model; backends fill it in their own copy.
    pub name: String,
    /// Type names derived from the operation, by template (`{P}Result` → `GetPetByIdResult`).
    /// Empty in the model, like `name`; a backend fills in the names it assigned (design.md 6.6).
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)]
    pub type_names: BTreeMap<String, String>,
    /// Upper-case method name.
    pub method: String,
    pub path: String,
    pub tags: Vec<String>,
    pub doc: Option<Doc>,
    pub params: Vec<Param>,
    pub body: Option<Body>,
    pub responses: Vec<Response>,
    pub callbacks: Vec<Callback>,
    /// Authentication alternatives in order: the operation's `security`, or the root's. Each lists
    /// the schemes it needs together; an empty alternative means none. Empty without any `security`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub security: Vec<Vec<SecurityUse>>,
    /// How a read is paginated (`x-tsurara-pagination`), for infinite queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    pub loc: Loc,
}

/// How a read is paginated: OpenAPI does not describe it, so it is written in
/// `x-tsurara-pagination` (or in a language's configuration).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    /// The request field that carries the page: `query.cursor`, `header.X-Page`, `path.page` or
    /// `cookie.page` (the parameter's `in`, then its name).
    pub param: String,
    /// The value of the first page; without it, the first request leaves the field out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub initial: Option<serde_json::Value>,
    /// JSON Pointer to the next page's value in a success body; absent or `null` there means no more
    /// pages. Without it, the app says how to find the next page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
    /// JSON Pointer to the previous page's value, for queries read in both directions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
}

impl Pagination {
    /// Reads and checks a pagination object against the operation's parameters.
    pub fn parse(value: &Value, params: &[Param]) -> Result<Pagination, String> {
        let p: Pagination = serde_json::from_value(value.clone())
            .map_err(|e| format!("pagination must be an object with `param`: {e}"))?;
        p.check(params)?;
        Ok(p)
    }

    /// Checks that `param` names a parameter of the operation and that the pointers are JSON Pointers.
    pub fn check(&self, params: &[Param]) -> Result<(), String> {
        let (location, name) = self.param.split_once('.').ok_or_else(|| {
            format!(
                "`param` is `{}`; write the parameter's `in`, a dot and its name (`query.cursor`)",
                self.param
            )
        })?;
        let location = ParamIn::parse(location)
            .filter(|l| *l != ParamIn::QueryString)
            .ok_or_else(|| {
                format!("`param` starts with `{location}`, which is not `query`, `header`, `path` or `cookie`")
            })?;
        if !params
            .iter()
            .any(|p| p.location == location && p.name == name)
        {
            return Err(format!(
                "`param` names `{}`, which is not a parameter of the operation",
                self.param
            ));
        }
        for (key, pointer) in [("next", &self.next), ("previous", &self.previous)] {
            if let Some(pointer) = pointer {
                if !pointer.is_empty() && !pointer.starts_with('/') {
                    return Err(format!(
                        "`{key}` is `{pointer}`, which is not a JSON Pointer (`/nextCursor`)"
                    ));
                }
            }
        }
        Ok(())
    }
}

impl Operation {
    /// Whether there is a required parameter or a body. If not, `options` can be omitted.
    pub fn has_required_input(&self) -> bool {
        self.params.iter().any(|p| p.required) || self.body.as_ref().is_some_and(|b| b.required)
    }
}

const METHODS: &[&str] = &[
    "get", "put", "post", "delete", "options", "head", "patch", "trace", "query",
];

/// Kind of operation. Affects how names are built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpKind {
    Operation,
    Webhook,
    Callback,
}

pub struct OperationBuilder<'c, 'a> {
    pub conv: &'c mut Converter<'a>,
    /// Whether a webhook or callback is being converted. The other server sends the request, so the body and
    /// parameters use response-side (`Mode::Read`) types, and the response we return uses `Mode::Write`.
    pub incoming: bool,
}

impl OperationBuilder<'_, '_> {
    fn request_mode(&self) -> Mode {
        if self.incoming {
            Mode::Read
        } else {
            Mode::Write
        }
    }

    fn response_mode(&self) -> Mode {
        if self.incoming {
            Mode::Write
        } else {
            Mode::Read
        }
    }

    /// Collects the operations in `paths` in document order.
    pub fn collect_paths(&mut self) -> Vec<(Loc, String, String)> {
        let paths = self.conv.set.root_loc().join("paths");
        self.collect_path_items(&paths)
    }

    /// Reads `x-event-channels`: channels whose messages are not responses of an
    /// operation, such as the updates of a Mercure hub.
    pub fn collect_channels(&mut self) -> Vec<Channel> {
        let root = self.conv.set.root_loc().join("x-event-channels");
        let Ok(Value::Object(channels)) = self.conv.set.get(&root).cloned() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for (name, value) in channels {
            let loc = root.join(&name);
            let Value::Object(c) = value else {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &loc,
                    "an event channel must be an object",
                );
                continue;
            };
            let template = c
                .get("channel")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_owned();
            if crate::text::has_control(&name) || crate::text::has_control(&template) {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &loc,
                    "the channel name or template contains a control character or a line break; the channel is left out",
                );
                continue;
            }
            let name_id = self
                .conv
                .request_name(NameKind::Channel { name: name.clone() }, &loc);
            let mut parameters = Vec::new();
            if let Some(Value::Object(ps)) = c.get("parameters") {
                for key in ps.keys() {
                    let ty = self
                        .conv
                        .convert(&loc.join("parameters").join(key), Mode::Write);
                    parameters.push(ChannelParam {
                        name: key.clone(),
                        ty,
                    });
                }
            }
            let variables = template_variables(&template);
            for v in &variables {
                if !parameters.iter().any(|p| p.name == *v) {
                    self.conv.warn(
                        WarningKind::InvalidDefinition,
                        &loc.join("channel"),
                        format!("`{{{v}}}` has no entry in `parameters`; it is a string"),
                    );
                    parameters.push(ChannelParam {
                        name: v.clone(),
                        ty: Type::String,
                    });
                }
            }
            parameters.retain(|p| {
                let used = variables.contains(&p.name);
                if !used {
                    self.conv.warn(
                        WarningKind::InvalidDefinition,
                        &loc.join("parameters").join(&p.name),
                        format!("`{}` is not in the channel `{template}`", p.name),
                    );
                }
                used
            });
            let contents = if c.get("content").is_some() {
                self.contents(&loc.join("content"), Mode::Read, false)
            } else {
                Vec::new()
            };
            let doc = {
                let mut d = Doc::default();
                let summary = c.get("summary").and_then(Value::as_str);
                let description = c.get("description").and_then(Value::as_str);
                d.description = match (summary, description) {
                    (Some(s), Some(d)) => Some(format!("{s}\n\n{d}")),
                    (Some(s), None) => Some(s.to_owned()),
                    (None, Some(d)) => Some(d.to_owned()),
                    (None, None) => None,
                };
                d.into_option()
            };
            out.push(Channel {
                name,
                name_id,
                ident: String::new(),
                type_names: BTreeMap::new(),
                doc,
                channel: template,
                parameters,
                private: c.get("private").and_then(Value::as_bool).unwrap_or(false),
                contents,
                loc,
            });
        }
        out
    }

    /// Collects the operations in `webhooks` (3.1 and later). The name is the webhook name.
    pub fn collect_webhooks(&mut self) -> Vec<(Loc, String, String)> {
        let hooks = self.conv.set.root_loc().join("webhooks");
        self.collect_path_items(&hooks)
    }

    /// From a collection of path items (`paths`, `webhooks`, a callback expression), returns
    /// (operation location, method, path or name) tuples in document order.
    fn collect_path_items(&mut self, parent: &Loc) -> Vec<(Loc, String, String)> {
        let keys: Vec<String> = match self.conv.set.get(parent) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => return Vec::new(),
        };
        // Path keys are appended to the base URL; without a leading `/` they could change its host or
        // port (`@evil.example/x`). Webhook names and callback expressions are not paths.
        let paths = parent.last_token().as_deref() == Some("paths");
        let mut out = Vec::new();
        for key in keys {
            if key.starts_with("x-") {
                continue;
            }
            let item_loc = parent.join(&key);
            if paths && !key.starts_with('/') {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &item_loc,
                    "a path must start with `/`; its operations are left out",
                );
                continue;
            }
            if crate::text::has_control(&key) {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &item_loc,
                    "the key contains a control character or a line break; its operations are left out",
                );
                continue;
            }
            let (item_loc, item) = match self.conv.set.deref_all(&item_loc) {
                Ok((l, v)) => (l, v.clone()),
                Err(e) => {
                    self.conv
                        .warn(WarningKind::UnresolvedRef, &item_loc, e.to_string());
                    continue;
                }
            };
            let Value::Object(item) = item else { continue };
            for (k, _) in &item {
                if METHODS.contains(&k.as_str()) {
                    out.push((item_loc.join(k), k.to_uppercase(), key.clone()));
                }
            }
            if let Some(Value::Object(extra)) = item.get("additionalOperations") {
                for k in extra.keys() {
                    if crate::text::has_control(k) {
                        self.conv.warn(
                            WarningKind::InvalidDefinition,
                            &item_loc.join("additionalOperations").join(k),
                            "the method contains a control character or a line break; the operation is left out",
                        );
                        continue;
                    }
                    out.push((
                        item_loc.join("additionalOperations").join(k),
                        k.to_uppercase(),
                        key.clone(),
                    ));
                }
            }
        }
        out
    }

    /// Compares the `{name}` variables of an operation's path with its `in: path` parameters. A variable without a parameter becomes a required string parameter, so the call can still
    /// be made; a parameter that is not in the template is left out, since it could never be sent.
    /// Both are `invalid-definition` warnings.
    pub fn check_path_template(&mut self, op: &mut Operation) {
        let variables = template_variables(&op.path);
        let who = format!("`{}` ({} {})", op.id, op.method, op.path);
        let quoted = |names: &[String]| {
            names
                .iter()
                .map(|n| format!("`{n}`"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let missing: Vec<String> = variables
            .iter()
            .filter(|v| {
                !op.params
                    .iter()
                    .any(|p| p.location == ParamIn::Path && p.name == **v)
            })
            .cloned()
            .collect();
        for v in &missing {
            op.params.push(Param {
                name: v.clone(),
                location: ParamIn::Path,
                required: true,
                ty: Type::String,
                doc: None,
                style: None,
                explode: None,
                allow_reserved: false,
                content_type: None,
            });
        }
        let mut extra = Vec::new();
        op.params.retain(|p| {
            let keep = p.location != ParamIn::Path || variables.contains(&p.name);
            if !keep {
                extra.push(p.name.clone());
            }
            keep
        });
        // One warning per direction: warnings are kept once per location and kind.
        if !missing.is_empty() {
            self.conv.warn(
                WarningKind::InvalidDefinition,
                &op.loc,
                if missing.len() == 1 {
                    format!(
                        "{who}: the path variable {} has no path parameter; it is a required string",
                        quoted(&missing)
                    )
                } else {
                    format!(
                        "{who}: the path variables {} have no path parameter; they are required strings",
                        quoted(&missing)
                    )
                },
            );
        }
        if !extra.is_empty() {
            self.conv.warn(
                WarningKind::InvalidDefinition,
                &op.loc.join("parameters"),
                if extra.len() == 1 {
                    format!(
                        "{who}: the path parameter {} is not in the path; it is left out",
                        quoted(&extra)
                    )
                } else {
                    format!(
                        "{who}: the path parameters {} are not in the path; they are left out",
                        quoted(&extra)
                    )
                },
            );
        }
    }

    /// Reads an operation's values (`operationId` and tags). Used for filtering.
    pub fn peek(&self, loc: &Loc) -> (Option<String>, Vec<String>) {
        let v = self.conv.set.get(loc).ok();
        let id = v
            .and_then(|v| v.get("operationId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let tags = v
            .and_then(|v| v.get("tags"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        (id, tags)
    }

    /// Requests the name of an operation, and returns its id (the `operationId`, or one built from
    /// the method and path) with the name request. Names are requested in document order.
    pub fn name_operation(
        &mut self,
        loc: &Loc,
        method: &str,
        path: &str,
        kind: OpKind,
        parent: Option<NameId>,
    ) -> (String, NameId) {
        let (op_id, _) = self.peek(loc);
        let id = op_id
            .clone()
            .unwrap_or_else(|| tsurara_naming::function_name_from_path(method, path));
        let kind = match kind {
            OpKind::Operation => NameKind::Operation {
                operation_id: op_id,
                method: method.to_owned(),
                path: path.to_owned(),
            },
            OpKind::Webhook => NameKind::Webhook {
                name: path.to_owned(),
            },
            OpKind::Callback => NameKind::Callback {
                parent: parent.expect("callbacks have a parent"),
                label: path.to_owned(),
                method: method.to_owned(),
            },
        };
        (id, self.conv.request_name(kind, loc))
    }

    /// Converts an operation.
    pub fn build(
        &mut self,
        loc: &Loc,
        method: &str,
        path: &str,
        id: String,
        name_id: NameId,
    ) -> Operation {
        // The operation owns its inline schemas (hoisting candidates such as `GetPetByIdBody`).
        let owner = (loc.clone(), tsurara_naming::pascal_case(&id));
        let saved_owner = self.conv.operation_owner.replace(owner);
        let built = self.build_operation(loc, method, path, id, name_id);
        self.conv.operation_owner = saved_owner;
        built
    }

    fn build_operation(
        &mut self,
        loc: &Loc,
        method: &str,
        path: &str,
        id: String,
        name_id: NameId,
    ) -> Operation {
        let op = self.conv.set.get(loc).cloned().unwrap_or(Value::Null);
        let item_loc = if loc.parent().and_then(|p| p.last_token()).as_deref()
            == Some("additionalOperations")
        {
            loc.parent()
                .and_then(|p| p.parent())
                .unwrap_or_else(|| loc.clone())
        } else {
            loc.parent().unwrap_or_else(|| loc.clone())
        };
        let tags = op
            .get("tags")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut doc = Doc::default();
        let summary = op.get("summary").and_then(Value::as_str);
        let description = op.get("description").and_then(Value::as_str);
        doc.description = match (summary, description) {
            (Some(s), Some(d)) => Some(format!("{s}\n\n{d}")),
            (Some(s), None) => Some(s.to_owned()),
            (None, Some(d)) => Some(d.to_owned()),
            (None, None) => None,
        };
        if op.get("deprecated").and_then(Value::as_bool) == Some(true) {
            doc.tag("deprecated", "");
        }
        let params = self.params(&item_loc, loc);
        let body = self.body(loc);
        let responses = self.responses(loc);
        let callbacks = self.callbacks(loc, name_id);
        let security = self.security(loc);
        let pagination = match op.get("x-tsurara-pagination") {
            None => None,
            Some(value) => match Pagination::parse(value, &params) {
                Ok(p) => Some(p),
                Err(e) => {
                    self.conv.warn(
                        WarningKind::InvalidDefinition,
                        &loc.join("x-tsurara-pagination"),
                        format!("{e}; the operation gets no infinite query options"),
                    );
                    None
                }
            },
        };
        Operation {
            id,
            name_id,
            name: String::new(),
            type_names: BTreeMap::new(),
            method: method.to_owned(),
            path: path.to_owned(),
            tags,
            doc: doc.into_option(),
            params,
            body,
            responses,
            callbacks,
            security,
            pagination,
            loc: loc.clone(),
        }
    }

    /// The security requirements of an operation, resolved against `components.securitySchemes`.
    fn security(&mut self, loc: &Loc) -> Vec<Vec<SecurityUse>> {
        let set = self.conv.set;
        let own = set.get(&loc.join("security")).ok().cloned();
        let root = set.root().get("security").cloned();
        let Some(Value::Array(requirements)) = own.or(root) else {
            return Vec::new();
        };
        if requirements.is_empty() {
            // `security: []`: no authentication, also when the root has some.
            return vec![Vec::new()];
        }
        let schemes = set.root_loc().join("components").join("securitySchemes");
        let mut out = Vec::new();
        'alternatives: for requirement in &requirements {
            let Value::Object(map) = requirement else {
                continue;
            };
            let mut alternative = Vec::new();
            for (name, scopes) in map {
                let Ok((_, Value::Object(scheme))) = set.deref_all(&schemes.join(name)) else {
                    self.conv.warn(
                        WarningKind::InvalidDefinition,
                        loc,
                        format!("the security scheme `{name}` is not defined; the alternative that uses it is ignored"),
                    );
                    continue 'alternatives;
                };
                let text = |key: &str| scheme.get(key).and_then(Value::as_str).map(str::to_owned);
                alternative.push(SecurityUse {
                    scheme: name.clone(),
                    kind: text("type").unwrap_or_default(),
                    location: text("in"),
                    name: text("name"),
                    http_scheme: text("scheme").map(|s| s.to_ascii_lowercase()),
                    scopes: scopes
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .filter_map(Value::as_str)
                                .map(str::to_owned)
                                .collect()
                        })
                        .unwrap_or_default(),
                });
            }
            out.push(alternative);
        }
        out
    }

    /// Merges path-level and operation-level parameters by the pair of `name` and `in`.
    fn params(&mut self, item_loc: &Loc, op_loc: &Loc) -> Vec<Param> {
        let mut out: Vec<Param> = Vec::new();
        for parent in [item_loc, op_loc] {
            let list = parent.join("parameters");
            let len = match self.conv.set.get(&list) {
                Ok(Value::Array(a)) => a.len(),
                _ => 0,
            };
            for i in 0..len {
                let ploc = list.join(&i.to_string());
                let Some(p) = self.param(&ploc) else { continue };
                if let Some(existing) = out
                    .iter_mut()
                    .find(|q| q.name == p.name && q.location == p.location)
                {
                    *existing = p;
                } else {
                    out.push(p);
                }
            }
        }
        out
    }

    fn param(&mut self, ploc: &Loc) -> Option<Param> {
        let (ploc, p) = match self.conv.set.deref_all(ploc) {
            Ok((l, v)) => (l, v.clone()),
            Err(e) => {
                self.conv
                    .warn(WarningKind::UnresolvedRef, ploc, e.to_string());
                return None;
            }
        };
        let name = p.get("name").and_then(Value::as_str)?.to_owned();
        let Some(location) = p.get("in").and_then(Value::as_str).and_then(ParamIn::parse) else {
            self.conv.warn(
                WarningKind::InvalidDefinition,
                &ploc,
                "the parameter has an unknown `in`",
            );
            return None;
        };
        // OpenAPI says these header parameters are ignored.
        if location == ParamIn::Header
            && ["accept", "content-type", "authorization"]
                .contains(&name.to_ascii_lowercase().as_str())
        {
            return None;
        }
        let required =
            location == ParamIn::Path || p.get("required").and_then(Value::as_bool) == Some(true);
        let (ty, content_type, schema_doc_v) =
            if let Some(Value::Object(content)) = p.get("content") {
                match content.keys().next() {
                    Some(mt) => {
                        let sloc = ploc.join("content").join(mt).join("schema");
                        let ty = if content[mt].get("schema").is_some() {
                            self.conv.convert(&sloc, self.request_mode())
                        } else {
                            Type::Unknown
                        };
                        let d = self.conv.set.get(&sloc).ok().and_then(schema_doc);
                        (ty, Some(mt.clone()), d)
                    }
                    None => (Type::Unknown, None, None),
                }
            } else if p.get("schema").is_some() {
                let sloc = ploc.join("schema");
                let d = self.conv.set.get(&sloc).ok().and_then(schema_doc);
                (self.conv.convert(&sloc, self.request_mode()), None, d)
            } else {
                (Type::String, None, None)
            };
        let mut doc = Doc {
            description: p
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_owned),
            tags: Vec::new(),
        };
        if p.get("deprecated").and_then(Value::as_bool) == Some(true) {
            doc.tag("deprecated", "");
        }
        let doc = match schema_doc_v {
            Some(sd) => doc.merge(sd).into_option(),
            None => doc.into_option(),
        };
        Some(Param {
            name,
            location,
            required,
            ty,
            doc,
            style: p.get("style").and_then(Value::as_str).map(str::to_owned),
            explode: p.get("explode").and_then(Value::as_bool),
            allow_reserved: p.get("allowReserved").and_then(Value::as_bool) == Some(true),
            content_type,
        })
    }

    fn body(&mut self, op_loc: &Loc) -> Option<Body> {
        let rb_loc = op_loc.join("requestBody");
        let (rb_loc, rb) = match self.conv.set.get(&rb_loc) {
            Ok(_) => match self.conv.set.deref_all(&rb_loc) {
                Ok((l, v)) => (l, v.clone()),
                Err(e) => {
                    self.conv
                        .warn(WarningKind::UnresolvedRef, &rb_loc, e.to_string());
                    return None;
                }
            },
            Err(_) => return None,
        };
        let contents = self.contents(&rb_loc.join("content"), self.request_mode(), true);
        Some(Body {
            required: rb.get("required").and_then(Value::as_bool) == Some(true),
            contents,
            doc: Doc {
                description: rb
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tags: Vec::new(),
            }
            .into_option(),
        })
    }

    /// Type per media type in `content`.
    fn contents(&mut self, content_loc: &Loc, mode: Mode, request: bool) -> Vec<MediaContent> {
        let content = match self.conv.set.get(content_loc) {
            Ok(Value::Object(m)) => m.clone(),
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        for (mt, media) in &content {
            let mloc = content_loc.join(mt);
            if crate::text::has_control(mt) {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &mloc,
                    "the media type contains a control character or a line break; it is left out",
                );
                continue;
            }
            let schema_key = if media.get("schema").is_some() {
                Some("schema")
            } else if media.get("itemSchema").is_some() {
                Some("itemSchema")
            } else {
                None
            };
            let mt_lower = mt.to_ascii_lowercase();
            let wildcard = is_wildcard(&mt_lower);
            let pa = match schema_key {
                Some(k) if wildcard => {
                    let sloc = mloc.join(k);
                    if self.conv.opts.wildcard_as_json {
                        BodyKind::Json
                    } else if self.conv.is_binary_root(&sloc) {
                        BodyKind::Blob
                    } else {
                        let what = if request {
                            "the body is sent as JSON"
                        } else {
                            "the body is parsed by its actual Content-Type (`auto`)"
                        };
                        self.conv.warn(
                            WarningKind::WildcardMediaType,
                            &mloc,
                            format!("`{mt}` with a schema that is not binary; {what}. Set `wildcardAsJson` if the API only uses JSON"),
                        );
                        BodyKind::Auto
                    }
                }
                None if wildcard => BodyKind::Blob,
                _ => body_kind_of(mt),
            };
            // Items of a sequential JSON response are JSON values.
            let framing = if request {
                None
            } else {
                Framing::of(&mt_lower)
            };
            let item_key = framing.and(item_schema_key(media));
            let schema_ty = schema_key.map(|k| {
                let sloc = mloc.join(k);
                if item_key == Some(k) {
                    self.conv.convert(&sloc, mode)
                } else if mt_lower.starts_with("multipart/") {
                    self.conv.convert_multipart(&sloc, mode)
                } else if matches!(pa, BodyKind::Json | BodyKind::Auto)
                    || mt_lower.starts_with("application/x-www-form-urlencoded")
                {
                    self.conv.convert(&sloc, mode)
                } else {
                    self.conv.convert_root(&sloc, mode)
                }
            });
            // The schema type of the value. How the body is carried (`body_kind`) is applied by
            // each backend: TypeScript turns binary bodies into `Blob` and so on.
            let ty = schema_ty.unwrap_or(Type::Unknown);
            let mut encoding = Vec::new();
            if let Some(Value::Object(enc)) = media.get("encoding") {
                for (prop, e) in enc {
                    encoding.push(Encoding {
                        property: prop.clone(),
                        content_type: e
                            .get("contentType")
                            .and_then(Value::as_str)
                            .map(str::to_owned),
                        style: e.get("style").and_then(Value::as_str).map(str::to_owned),
                        explode: e.get("explode").and_then(Value::as_bool),
                    });
                }
            }
            let items = match (framing, item_key) {
                (Some(framing), Some(k)) => {
                    let item = if schema_key == Some(k) {
                        ty.clone()
                    } else {
                        self.conv.convert(&mloc.join(k), mode)
                    };
                    Some(Items {
                        ty: named_inline(item, format!("{}Item", self.owner_name())),
                        framing,
                    })
                }
                _ => None,
            };
            let events = if !request && essence(&mt_lower) == "text/event-stream" {
                self.events(&mloc, media, mode)
            } else {
                None
            };
            out.push(MediaContent {
                media_type: mt.clone(),
                ty,
                encoding,
                body_kind: pa,
                events,
                items,
            });
        }
        out
    }

    /// The events an item schema describes (`itemSchema` from 3.2, or `x-itemSchema`). The schema is
    /// an object with the properties `event`, `data`, `id` and `retry`; each entry of its `oneOf` or
    /// `anyOf` is one event, completed with the properties written next to the keyword.
    fn events(&mut self, mloc: &Loc, media: &Value, mode: Mode) -> Option<Events> {
        let key = item_schema_key(media)?;
        let (base_loc, base) = match self.conv.set.deref_all(&mloc.join(key)) {
            Ok((l, v)) => (l, v.clone()),
            Err(e) => {
                self.conv
                    .warn(WarningKind::UnresolvedRef, &mloc.join(key), e.to_string());
                return None;
            }
        };
        let mut alternatives: Vec<Option<(Loc, Loc, Value)>> = Vec::new();
        for keyword in ["oneOf", "anyOf"] {
            let Some(Value::Array(items)) = base.get(keyword) else {
                continue;
            };
            for i in 0..items.len() {
                let written = base_loc.join(keyword).join(&i.to_string());
                match self.conv.set.deref_all(&written) {
                    Ok((l, v)) => alternatives.push(Some((written, l, v.clone()))),
                    Err(e) => self
                        .conv
                        .warn(WarningKind::UnresolvedRef, &written, e.to_string()),
                }
            }
            if !alternatives.is_empty() {
                break;
            }
        }
        if alternatives.is_empty() {
            alternatives.push(None);
        }

        struct Event {
            written: Loc,
            event: Type,
            names: Vec<String>,
            data: Type,
            json: bool,
            id_required: bool,
            retry_required: bool,
        }
        let mut events = Vec::new();
        for alt in &alternatives {
            // The schema of a property: the alternative's, or else the one written next to it.
            let property = |name: &str| -> Option<Loc> {
                alt.iter()
                    .map(|(_, l, v)| (l, v))
                    .chain([(&base_loc, &base)])
                    .find(|(_, v)| v.get("properties").and_then(|p| p.get(name)).is_some())
                    .map(|(l, _)| l.join("properties").join(name))
            };
            let required = |name: &str| {
                alt.iter()
                    .map(|(_, _, v)| v)
                    .chain([&base])
                    .filter_map(|v| v.get("required").and_then(Value::as_array))
                    .flatten()
                    .any(|r| r.as_str() == Some(name))
            };
            let (id_required, retry_required) = (required("id"), required("retry"));
            let (event_loc, data_loc) = (property("event"), property("data"));
            let resolved = |conv: &Converter<'_>, loc: &Option<Loc>| {
                loc.as_ref()
                    .and_then(|l| conv.set.deref_all(l).ok())
                    .map(|(l, v)| (l, v.clone()))
            };

            let mut names = Vec::new();
            if let Some((_, v)) = resolved(self.conv, &event_loc) {
                match (v.get("const"), v.get("enum")) {
                    (Some(Value::String(s)), _) => names.push(s.clone()),
                    (None, Some(Value::Array(values))) => {
                        names.extend(values.iter().filter_map(Value::as_str).map(str::to_owned));
                    }
                    _ => {}
                }
            }
            // The names are the event's type: a schema of `event` says no more.
            let event = match names.as_slice() {
                [] => Type::String,
                [one] => Type::string_literal(one.clone()),
                // Several names are an enum, which languages without literal types can declare.
                _ => Type::Inline(Box::new(Inline {
                    preferred_name: format!(
                        "{}{}EventName",
                        self.owner_name(),
                        tsurara_naming::pascal_case(&names[0])
                    ),
                    loc: event_loc.clone().unwrap_or_else(|| mloc.join(key)),
                    ty: Type::Enum(crate::ir::Enum {
                        values: names
                            .iter()
                            .map(|n| crate::ir::Literal::String(n.clone()))
                            .collect(),
                        nullable: false,
                    }),
                })),
            };

            let (mut data, mut json) = (Type::String, false);
            if let (Some(written), Some((l, v))) = (&data_loc, resolved(self.conv, &data_loc)) {
                let media_type = v.get("contentMediaType").and_then(Value::as_str);
                if media_type.is_some_and(|m| body_kind_of(m) == BodyKind::Json) {
                    json = true;
                    data = if v.get("contentSchema").is_some() {
                        let name = match names.first() {
                            Some(n) => format!(
                                "{}{}EventData",
                                self.owner_name(),
                                tsurara_naming::pascal_case(n)
                            ),
                            None => format!("{}EventData", self.owner_name()),
                        };
                        named_inline(self.conv.convert(&l.join("contentSchema"), mode), name)
                    } else {
                        Type::Unknown
                    };
                } else if media_type.is_none()
                    && (v.get("const").is_some() || v.get("enum").is_some())
                {
                    // Literal texts such as `[DONE]` are kept.
                    data = self.conv.convert(written, mode);
                }
            }
            events.push(Event {
                written: alt.as_ref().map_or(mloc.join(key), |(w, _, _)| w.clone()),
                event,
                names,
                data,
                json,
                id_required,
                retry_required,
            });
        }

        // A runtime decides by the event name whether to parse `data`, so that events the definition
        // does not list keep their text. Only an event without a name, which stands for any other
        // name, makes every event JSON.
        let json = if events.iter().all(|e| e.json) && events.iter().any(|e| e.names.is_empty()) {
            Some(JsonEvents::All)
        } else {
            let mut named = Vec::new();
            for e in events.iter_mut().filter(|e| e.json) {
                if e.names.is_empty() {
                    self.conv.warn(
                        WarningKind::Unsupported,
                        &e.written,
                        "`data` is JSON, but the event has no name (`const` or `enum` of `event`) to tell it from the events with text; its data stays a string",
                    );
                    e.data = Type::String;
                    e.json = false;
                } else {
                    for name in &e.names {
                        if !named.contains(name) {
                            named.push(name.clone());
                        }
                    }
                }
            }
            (!named.is_empty()).then_some(JsonEvents::Named(named))
        };
        let owner = self.owner_name();
        let nameless = events.iter().filter(|e| e.names.is_empty()).count();
        let tagged = events.len() > 1 && nameless <= 1;
        let mut members = Vec::new();
        let mut values = Vec::new();
        let mut default = None;
        for e in events {
            let preferred_name = match e.names.first() {
                Some(name) => format!("{owner}{}Event", tsurara_naming::pascal_case(name)),
                None if tagged || nameless > 1 => format!("{owner}OtherEvent"),
                None => format!("{owner}Event"),
            };
            let (names, written) = (e.names.clone(), e.written.clone());
            let object = Type::Object(Object {
                properties: vec![
                    Property::new("event", e.event),
                    // An event without data is not dispatched, so `data` is always there.
                    Property::new("data", e.data),
                    Property::new("id", Type::String).optional(!e.id_required),
                    Property::new(
                        "retry",
                        Type::Scalar(Scalar::new(ScalarKind::Integer, None)),
                    )
                    .optional(!e.retry_required),
                ],
                index: None,
            });
            let member = Type::Inline(Box::new(Inline {
                preferred_name,
                loc: written,
                ty: object,
            }));
            if tagged && names.is_empty() {
                default = Some(Box::new(member));
            } else {
                members.push(member);
                values.push(names);
            }
        }
        // One event is the event type itself; several are a union tagged by `event` (design.md 8.5).
        let ty = if members.len() == 1 && default.is_none() {
            members.pop().expect("one member")
        } else {
            Type::Inline(Box::new(Inline {
                preferred_name: format!("{owner}Event"),
                loc: mloc.join(key),
                ty: Type::Choice(Choice {
                    members,
                    exclusive: true,
                    discriminator: tagged.then(|| Discriminator {
                        property: "event".to_owned(),
                        values,
                        default,
                    }),
                }),
            }))
        };
        Some(Events { ty, json })
    }

    /// The preferred name of the operation being converted, which names its inline types.
    fn owner_name(&self) -> String {
        self.conv
            .operation_owner
            .as_ref()
            .map_or_else(String::new, |(_, n)| n.clone())
    }

    fn responses(&mut self, op_loc: &Loc) -> Vec<Response> {
        let rs_loc = op_loc.join("responses");
        let keys: Vec<String> = match self.conv.set.get(&rs_loc) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => Vec::new(),
        };
        let mut out = Vec::new();
        for key in keys {
            if key.starts_with("x-") {
                continue;
            }
            let Some(status) = Status::parse(&key) else {
                self.conv.warn(
                    WarningKind::InvalidDefinition,
                    &rs_loc.join(&key),
                    format!("`{key}` is not a valid status code"),
                );
                continue;
            };
            let rloc = rs_loc.join(&key);
            let (rloc, r) = match self.conv.set.deref_all(&rloc) {
                Ok((l, v)) => (l, v.clone()),
                Err(e) => {
                    self.conv
                        .warn(WarningKind::UnresolvedRef, &rloc, e.to_string());
                    continue;
                }
            };
            let contents = self.contents(&rloc.join("content"), self.response_mode(), false);
            let headers = self.response_headers(&rloc);
            out.push(Response {
                status,
                description: r
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                contents,
                headers,
            });
        }
        out
    }

    fn response_headers(&mut self, rloc: &Loc) -> Vec<Property> {
        let hloc = rloc.join("headers");
        let keys: Vec<String> = match self.conv.set.get(&hloc) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        for name in keys {
            if name.eq_ignore_ascii_case("content-type") {
                continue;
            }
            let (loc, h) = match self.conv.set.deref_all(&hloc.join(&name)) {
                Ok((l, v)) => (l, v.clone()),
                Err(e) => {
                    self.conv
                        .warn(WarningKind::UnresolvedRef, &hloc.join(&name), e.to_string());
                    continue;
                }
            };
            let ty = if h.get("schema").is_some() {
                self.conv.convert(&loc.join("schema"), self.response_mode())
            } else if let Some(Value::Object(c)) = h.get("content") {
                match c.keys().next() {
                    Some(mt) => self.conv.convert(
                        &loc.join("content").join(mt).join("schema"),
                        self.response_mode(),
                    ),
                    None => Type::String,
                }
            } else {
                Type::String
            };
            let required = h.get("required").and_then(Value::as_bool) == Some(true);
            let doc = Doc {
                description: h
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
                tags: Vec::new(),
            }
            .into_option();
            out.push(
                Property::new(name.to_ascii_lowercase(), ty)
                    .optional(!required)
                    .with_doc(doc),
            );
        }
        out
    }

    fn callbacks(&mut self, op_loc: &Loc, parent: NameId) -> Vec<Callback> {
        let cloc = op_loc.join("callbacks");
        let keys: Vec<String> = match self.conv.set.get(&cloc) {
            Ok(Value::Object(m)) => m.keys().cloned().collect(),
            _ => return Vec::new(),
        };
        let mut out = Vec::new();
        for name in keys {
            let (cb_loc, _) = match self.conv.set.deref_all(&cloc.join(&name)) {
                Ok(x) => x,
                Err(e) => {
                    self.conv
                        .warn(WarningKind::UnresolvedRef, &cloc.join(&name), e.to_string());
                    continue;
                }
            };
            let items = self.collect_path_items(&cb_loc);
            let multiple = items.len() > 1;
            let mut ops = Vec::new();
            for (loc, method, expr) in items {
                let label = if multiple {
                    format!("{name} {method}")
                } else {
                    name.clone()
                };
                let (id, name_id) = self.name_operation(
                    &loc,
                    if multiple { &method } else { "" },
                    &label,
                    OpKind::Callback,
                    Some(parent),
                );
                let saved = std::mem::replace(&mut self.incoming, true);
                let built = self.build(&loc, &method, &expr, id, name_id);
                self.incoming = saved;
                ops.push(built);
            }
            out.push(Callback {
                name,
                operations: ops,
            });
        }
        out
    }
}

/// `*/*`, `application/*` and similar. `text/*` is text, not a wildcard in this sense.
fn is_wildcard(media_type: &str) -> bool {
    let mt = media_type.split(';').next().unwrap_or("").trim();
    mt.contains('*') && !mt.starts_with("text/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_as_by_media_type() {
        let cases = [
            ("application/json", BodyKind::Json),
            ("application/problem+json", BodyKind::Json),
            ("application/vnd.api+json; charset=utf-8", BodyKind::Json),
            // Wildcards are decided with the schema in `contents`; the media type alone says binary.
            ("*/*", BodyKind::Blob),
            ("text/plain", BodyKind::Text),
            ("text/html", BodyKind::Text),
            ("application/xml", BodyKind::Text),
            ("application/octet-stream", BodyKind::Blob),
            ("image/png", BodyKind::Blob),
            ("application/pdf", BodyKind::Blob),
            ("text/event-stream", BodyKind::Stream),
            ("application/jsonl", BodyKind::Stream),
            ("application/x-ndjson", BodyKind::Stream),
        ];
        for (mt, want) in cases {
            assert_eq!(body_kind_of(mt), want, "{mt}");
        }
    }

    #[test]
    fn statuses() {
        assert_eq!(Status::parse("200"), Some(Status::Code(200)));
        assert_eq!(Status::parse("2XX"), Some(Status::Range(2)));
        assert_eq!(Status::parse("4xx"), Some(Status::Range(4)));
        assert_eq!(Status::parse("default"), Some(Status::Default));
        assert_eq!(Status::parse("abc"), None);
        assert_eq!(Status::parse("700"), None);
        assert_eq!(Status::Code(302).is_success(), Some(true));
        assert_eq!(Status::Range(5).is_success(), Some(false));
        assert_eq!(Status::Default.is_success(), None);
    }

    #[test]
    fn responses_without_body_have_no_data() {
        let r = Response {
            status: Status::Code(204),
            description: None,
            contents: vec![],
            headers: vec![],
        };
        assert_eq!(r.data_type(), Type::Undefined);
    }
}
