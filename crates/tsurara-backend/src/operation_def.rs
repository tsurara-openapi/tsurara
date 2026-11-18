//! OperationDefs as JSON values (spec/operation-def.schema.json, design.md 18.9), for backends that
//! print them as literals of their language. The TypeScript backend prints its own literals and
//! agrees with these (`tsurara-lang-typescript/tests/operation_def_schema.rs`).

use serde_json::{Map, Value, json};
use tsurara_sem::operation::{BodyKind, JsonEvents, MediaContent, Operation, ParamIn};

/// Version of the OperationDef format.
pub const OPERATION_DEF_VERSION: u32 = 1;

/// TypeDef references for the runtimes that convert values with TypeDefs (design.md 18.7): one per
/// parameter, per request media type, and per response and media type, following the operation's
/// lists. `None` leaves the reference out (values that need no conversion).
#[derive(Debug, Clone, Default)]
pub struct TypeRefs {
    pub params: Vec<Option<Value>>,
    pub body: Vec<Option<Value>>,
    pub responses: Vec<Vec<Option<Value>>>,
}

/// The `parseAs` of a response media type: `events`, `jsonLines` or `jsonSeq` for a typed sequence
/// (design.md 8.5), and the body kind otherwise.
pub fn parse_as(c: &MediaContent) -> &'static str {
    if c.events.is_some() {
        "events"
    } else if let Some(items) = &c.items {
        items.framing.parse_as()
    } else {
        c.body_kind.as_str()
    }
}

/// The OperationDef of `op`, with TypeDef references when `types` is given.
pub fn operation_def(op: &Operation, types: Option<&TypeRefs>) -> Value {
    let mut def = Map::new();
    def.insert("v".into(), json!(OPERATION_DEF_VERSION));
    def.insert("id".into(), json!(op.id));
    def.insert("method".into(), json!(op.method));
    def.insert("path".into(), json!(op.path));
    if !op.tags.is_empty() {
        def.insert("tags".into(), json!(op.tags));
    }
    let mut params = Map::new();
    for location in [
        ParamIn::Path,
        ParamIn::Query,
        ParamIn::Header,
        ParamIn::Cookie,
    ] {
        let mut group = Map::new();
        for (i, p) in op.params.iter().enumerate() {
            if p.location != location {
                continue;
            }
            let mut d = Map::new();
            if let Some(s) = &p.style {
                d.insert("style".into(), json!(s));
            }
            if let Some(e) = p.explode {
                d.insert("explode".into(), json!(e));
            }
            if p.allow_reserved {
                d.insert("allowReserved".into(), json!(true));
            }
            if let Some(ct) = &p.content_type {
                d.insert("contentType".into(), json!(ct));
            }
            if let Some(t) = types.and_then(|t| t.params.get(i).cloned().flatten()) {
                d.insert("type".into(), t);
            }
            group.insert(p.name.clone(), Value::Object(d));
        }
        if !group.is_empty() {
            params.insert(location.group().into(), Value::Object(group));
        }
    }
    if let Some(qs) = op
        .params
        .iter()
        .find(|p| p.location == ParamIn::QueryString)
    {
        let ct = qs
            .content_type
            .as_deref()
            .unwrap_or("application/x-www-form-urlencoded");
        params.insert("querystring".into(), json!({ "contentType": ct }));
    }
    if !params.is_empty() {
        def.insert("params".into(), Value::Object(params));
    }
    if let Some(body) = &op.body {
        let mut contents = Map::new();
        for (i, c) in body.contents.iter().enumerate() {
            let mut media = Map::new();
            if !c.encoding.is_empty() {
                let mut encs = Map::new();
                for e in &c.encoding {
                    let mut f = Map::new();
                    if let Some(ct) = &e.content_type {
                        f.insert("contentType".into(), json!(ct));
                    }
                    if let Some(st) = &e.style {
                        f.insert("style".into(), json!(st));
                    }
                    if let Some(ex) = e.explode {
                        f.insert("explode".into(), json!(ex));
                    }
                    encs.insert(e.property.clone(), Value::Object(f));
                }
                media.insert("encoding".into(), Value::Object(encs));
            }
            if let Some(t) = types.and_then(|t| t.body.get(i).cloned().flatten()) {
                media.insert("type".into(), t);
            }
            contents.insert(c.media_type.clone(), Value::Object(media));
        }
        let mut b = Map::new();
        if body.required {
            b.insert("required".into(), json!(true));
        }
        b.insert("contents".into(), Value::Object(contents));
        def.insert("body".into(), Value::Object(b));
    }
    let mut responses = Map::new();
    for (i, r) in op.responses.iter().enumerate() {
        let first = r.contents.first().map_or(BodyKind::None.as_str(), parse_as);
        let mut d = Map::new();
        d.insert("parseAs".into(), json!(first));
        if r.contents.iter().any(|c| parse_as(c) != first) {
            let map: Map<String, Value> = r
                .contents
                .iter()
                .map(|c| (c.media_type.clone(), json!(parse_as(c))))
                .collect();
            d.insert("contentTypes".into(), Value::Object(map));
        }
        match r
            .contents
            .iter()
            .find_map(|c| c.events.as_ref()?.json.as_ref())
        {
            Some(JsonEvents::All) => {
                d.insert("jsonEvents".into(), json!(true));
            }
            Some(JsonEvents::Named(names)) => {
                d.insert("jsonEvents".into(), json!(names));
            }
            None => {}
        }
        let mut refs = Map::new();
        for (j, c) in r.contents.iter().enumerate() {
            // Sequences refer to the type of one item.
            let typed = matches!(c.body_kind, BodyKind::Json | BodyKind::Auto)
                || c.events.is_some()
                || c.items.is_some();
            if !typed {
                continue;
            }
            let t =
                types.and_then(|t| t.responses.get(i).and_then(|r| r.get(j)).cloned().flatten());
            if let Some(t) = t {
                refs.insert(c.media_type.clone(), t);
            }
        }
        if !refs.is_empty() {
            d.insert("types".into(), Value::Object(refs));
        }
        responses.insert(r.status.key(), Value::Object(d));
    }
    def.insert("responses".into(), Value::Object(responses));
    if !op.security.is_empty() {
        def.insert(
            "security".into(),
            serde_json::to_value(&op.security).expect("security serializes"),
        );
    }
    Value::Object(def)
}
