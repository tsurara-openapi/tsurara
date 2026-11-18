//! Converts Swagger 2.0 definitions to OpenAPI 3.0.

use crate::docset::DocumentSet;
use serde_json::{Map, Value, json};
use url::Url;

/// Definition version.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase")]
pub enum SpecVersion {
    Swagger2,
    V3_0,
    V3_1,
    V3_2,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VersionError {
    #[error("the document has neither `openapi` nor `swagger`")]
    Missing,
    #[error("unsupported version `{0}`")]
    Unsupported(String),
}

/// Detects the version from the `openapi` / `swagger` field.
pub fn detect_version(root: &Value) -> Result<SpecVersion, VersionError> {
    if let Some(v) = root.get("openapi") {
        let v = v
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string());
        let mut parts = v.split('.');
        return match (parts.next(), parts.next()) {
            (Some("3"), Some("0")) => Ok(SpecVersion::V3_0),
            (Some("3"), Some("1")) => Ok(SpecVersion::V3_1),
            (Some("3"), Some("2")) => Ok(SpecVersion::V3_2),
            _ => Err(VersionError::Unsupported(v)),
        };
    }
    if let Some(v) = root.get("swagger") {
        let v = v
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| v.to_string());
        return if v == "2.0" || v == "2" {
            Ok(SpecVersion::Swagger2)
        } else {
            Err(VersionError::Unsupported(v))
        };
    }
    Err(VersionError::Missing)
}

/// If the root is Swagger 2.0, converts every document in the set to OpenAPI 3.0.
/// Returns `true` if it converted.
pub fn normalize_swagger2(set: &mut DocumentSet) -> Result<bool, VersionError> {
    if detect_version(set.root())? != SpecVersion::Swagger2 {
        return Ok(false);
    }
    let snapshot = set.clone();
    let root_url = set.root_url().clone();
    set.transform(|url, value| {
        let conv = Converter {
            set: &snapshot,
            doc: url,
        };
        if *url == root_url {
            *value = conv.convert_root(value);
        } else {
            conv.convert_fragment(value);
        }
    });
    Ok(true)
}

struct Converter<'a> {
    set: &'a DocumentSet,
    doc: &'a Url,
}

const DEFAULT_MEDIA_TYPE: &str = "application/json";

impl Converter<'_> {
    fn convert_root(&self, root: &Value) -> Value {
        let Value::Object(src) = root else {
            return root.clone();
        };
        let global_consumes = media_types(src.get("consumes"));
        let global_produces = media_types(src.get("produces"));
        let mut out = Map::new();
        out.insert("openapi".into(), json!("3.0.3"));
        for (k, v) in src {
            match k.as_str() {
                "swagger"
                | "host"
                | "basePath"
                | "schemes"
                | "consumes"
                | "produces"
                | "definitions"
                | "parameters"
                | "responses"
                | "securityDefinitions"
                | "paths" => {}
                _ => {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(servers) = self.servers(src) {
            out.insert("servers".into(), servers);
        }
        let ctx = Media {
            consumes: &global_consumes,
            produces: &global_produces,
        };
        if let Some(Value::Object(paths)) = src.get("paths") {
            let mut new_paths = Map::new();
            for (path, item) in paths {
                new_paths.insert(path.clone(), self.convert_path_item(item, &ctx));
            }
            out.insert("paths".into(), Value::Object(new_paths));
        }
        let components = self.components(src, &ctx);
        if !components.is_empty() {
            out.insert("components".into(), Value::Object(components));
        }
        let mut out = Value::Object(out);
        fix_schemas_and_refs(&mut out);
        reorder(out, &["openapi", "info", "servers"])
    }

    /// A non-root document. Moves `definitions` and the like under `components` and rewrites references.
    fn convert_fragment(&self, value: &mut Value) {
        if let Value::Object(map) = value {
            let ctx = Media {
                consumes: &[],
                produces: &[],
            };
            let components = self.components(map, &ctx);
            if !components.is_empty() {
                map.remove("definitions");
                map.remove("parameters");
                map.remove("responses");
                map.insert("components".into(), Value::Object(components));
            }
        }
        fix_schemas_and_refs(value);
    }

    fn components(&self, src: &Map<String, Value>, ctx: &Media) -> Map<String, Value> {
        let mut components = Map::new();
        if let Some(Value::Object(defs)) = src.get("definitions") {
            components.insert("schemas".into(), Value::Object(defs.clone()));
        }
        if let Some(Value::Object(params)) = src.get("parameters") {
            let converted: Map<String, Value> = params
                .iter()
                .filter(|(_, p)| !matches!(param_in(p), Some("body" | "formData")))
                .map(|(k, p)| (k.clone(), convert_parameter(p)))
                .collect();
            if !converted.is_empty() {
                components.insert("parameters".into(), Value::Object(converted));
            }
        }
        if let Some(Value::Object(responses)) = src.get("responses") {
            let converted: Map<String, Value> = responses
                .iter()
                .map(|(k, r)| (k.clone(), convert_response(r, ctx.produces)))
                .collect();
            components.insert("responses".into(), Value::Object(converted));
        }
        if let Some(Value::Object(defs)) = src.get("securityDefinitions") {
            let converted: Map<String, Value> = defs
                .iter()
                .map(|(k, d)| (k.clone(), convert_security_scheme(d)))
                .collect();
            components.insert("securitySchemes".into(), Value::Object(converted));
        }
        components
    }

    fn servers(&self, src: &Map<String, Value>) -> Option<Value> {
        let host = src.get("host").and_then(Value::as_str);
        let base_path = src.get("basePath").and_then(Value::as_str).unwrap_or("");
        let base_path = base_path.trim_end_matches('/');
        let Some(host) = host else {
            return Some(json!([{ "url": if base_path.is_empty() { "/" } else { base_path } }]));
        };
        let mut schemes: Vec<String> = src
            .get("schemes")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        if schemes.is_empty() {
            let scheme = match self.set.root_url().scheme() {
                s @ ("http" | "https") => s.to_owned(),
                _ => "https".to_owned(),
            };
            schemes.push(scheme);
        }
        Some(Value::Array(
            schemes
                .iter()
                .map(|s| json!({ "url": format!("{s}://{host}{base_path}") }))
                .collect(),
        ))
    }

    fn convert_path_item(&self, item: &Value, ctx: &Media) -> Value {
        let Value::Object(src) = item else {
            return item.clone();
        };
        if src.contains_key("$ref") {
            return item.clone();
        }
        let path_params = self.resolve_params(src.get("parameters"));
        let mut out = Map::new();
        let shared: Vec<Value> = path_params
            .iter()
            .filter(|(_, resolved)| !matches!(param_in(resolved), Some("body" | "formData")))
            .map(|(orig, _)| convert_parameter_or_ref(orig))
            .collect();
        for (k, v) in src {
            match k.as_str() {
                "parameters" => {
                    if !shared.is_empty() {
                        out.insert(k.clone(), Value::Array(shared.clone()));
                    }
                }
                "get" | "put" | "post" | "delete" | "options" | "head" | "patch" => {
                    out.insert(k.clone(), self.convert_operation(v, &path_params, ctx));
                }
                _ => {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        Value::Object(out)
    }

    /// Turns a parameter list into pairs of the original value (still a `$ref`) and the resolved value.
    fn resolve_params(&self, params: Option<&Value>) -> Vec<(Value, Value)> {
        let Some(Value::Array(params)) = params else {
            return Vec::new();
        };
        params
            .iter()
            .map(|p| {
                let resolved = match p.get("$ref").and_then(Value::as_str) {
                    Some(r) => self
                        .set
                        .resolve_url(self.doc, r)
                        .ok()
                        .and_then(|loc| self.set.deref_all(&loc).ok())
                        .map(|(_, v)| v.clone())
                        .unwrap_or_else(|| p.clone()),
                    None => p.clone(),
                };
                (p.clone(), resolved)
            })
            .collect()
    }

    fn convert_operation(&self, op: &Value, path_params: &[(Value, Value)], ctx: &Media) -> Value {
        let Value::Object(src) = op else {
            return op.clone();
        };
        let consumes = src
            .get("consumes")
            .map(|v| media_types(Some(v)))
            .unwrap_or_else(|| ctx.consumes.to_vec());
        let produces = src
            .get("produces")
            .map(|v| media_types(Some(v)))
            .unwrap_or_else(|| ctx.produces.to_vec());
        let own = self.resolve_params(src.get("parameters"));
        // Operation parameters override path-level ones with the same name and in.
        let key = |p: &Value| {
            (
                p.get("name").and_then(Value::as_str).map(str::to_owned),
                param_in(p).map(str::to_owned),
            )
        };
        let mut merged: Vec<(Value, Value)> = path_params
            .iter()
            .filter(|(_, r)| !own.iter().any(|(_, o)| key(o) == key(r)))
            .cloned()
            .collect();
        merged.extend(own.iter().cloned());

        let body = merged
            .iter()
            .find(|(_, r)| param_in(r) == Some("body"))
            .map(|(_, r)| r);
        let form: Vec<&Value> = merged
            .iter()
            .filter(|(_, r)| param_in(r) == Some("formData"))
            .map(|(_, r)| r)
            .collect();

        let mut out = Map::new();
        for (k, v) in src {
            match k.as_str() {
                "consumes" | "produces" | "schemes" => {}
                "parameters" => {
                    let params: Vec<Value> = own
                        .iter()
                        .filter(|(_, r)| !matches!(param_in(r), Some("body" | "formData")))
                        .map(|(o, _)| convert_parameter_or_ref(o))
                        .collect();
                    if !params.is_empty() {
                        out.insert(k.clone(), Value::Array(params));
                    }
                }
                "responses" => {
                    let Value::Object(responses) = v else {
                        continue;
                    };
                    let converted: Map<String, Value> = responses
                        .iter()
                        .map(|(code, r)| (code.clone(), convert_response(r, &produces)))
                        .collect();
                    out.insert(k.clone(), Value::Object(converted));
                }
                _ => {
                    out.insert(k.clone(), v.clone());
                }
            }
        }
        if let Some(rb) = request_body(body, &form, &consumes) {
            out.insert("requestBody".into(), rb);
        }
        Value::Object(out)
    }
}

struct Media<'a> {
    consumes: &'a [String],
    produces: &'a [String],
}

fn media_types(v: Option<&Value>) -> Vec<String> {
    v.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

fn param_in(p: &Value) -> Option<&str> {
    p.get("in").and_then(Value::as_str)
}

fn convert_parameter_or_ref(p: &Value) -> Value {
    if p.get("$ref").is_some() {
        p.clone()
    } else {
        convert_parameter(p)
    }
}

/// path / query / header parameters. Moves type information into `schema` and turns
/// `collectionFormat` into `style` and `explode`.
fn convert_parameter(p: &Value) -> Value {
    let Value::Object(src) = p else {
        return p.clone();
    };
    if src.contains_key("$ref") {
        return p.clone();
    }
    let location = src.get("in").and_then(Value::as_str).unwrap_or("query");
    let mut out = Map::new();
    let mut schema = Map::new();
    for (k, v) in src {
        match k.as_str() {
            "name" | "in" | "description" | "required" | "deprecated" | "allowEmptyValue" => {
                out.insert(k.clone(), v.clone());
            }
            "collectionFormat" => {}
            k if k.starts_with("x-") && k != "x-nullable" => {
                out.insert(k.to_owned(), v.clone());
            }
            _ => {
                schema.insert(k.clone(), v.clone());
            }
        }
    }
    if location == "path" {
        out.insert("required".into(), json!(true));
    }
    if schema.get("type").and_then(Value::as_str) == Some("array") {
        let format = src
            .get("collectionFormat")
            .and_then(Value::as_str)
            .unwrap_or("csv");
        let (style, explode) = match (format, location) {
            ("multi", _) => ("form", true),
            ("ssv", "query") => ("spaceDelimited", false),
            ("pipes", "query") => ("pipeDelimited", false),
            (_, "query" | "cookie") => ("form", false),
            _ => ("simple", false),
        };
        out.insert("style".into(), json!(style));
        out.insert("explode".into(), json!(explode));
        if let Some(Value::Object(items)) = schema.get_mut("items") {
            items.remove("collectionFormat");
        }
    }
    out.insert("schema".into(), Value::Object(schema));
    Value::Object(out)
}

fn convert_response(r: &Value, produces: &[String]) -> Value {
    let Value::Object(src) = r else {
        return r.clone();
    };
    if src.contains_key("$ref") {
        return r.clone();
    }
    let mut out = Map::new();
    for (k, v) in src {
        match k.as_str() {
            "schema" => {
                let types: Vec<String> = if produces.is_empty() {
                    vec![DEFAULT_MEDIA_TYPE.to_owned()]
                } else {
                    produces.to_vec()
                };
                let examples = src.get("examples").and_then(Value::as_object);
                let content: Map<String, Value> = types
                    .iter()
                    .map(|t| {
                        let mut media = Map::new();
                        media.insert("schema".into(), v.clone());
                        if let Some(ex) = examples.and_then(|e| e.get(t)) {
                            media.insert("example".into(), ex.clone());
                        }
                        (t.clone(), Value::Object(media))
                    })
                    .collect();
                out.insert("content".into(), Value::Object(content));
            }
            "examples" => {}
            "headers" => {
                let Value::Object(headers) = v else { continue };
                let converted: Map<String, Value> = headers
                    .iter()
                    .map(|(name, h)| (name.clone(), convert_header(h)))
                    .collect();
                out.insert(k.clone(), Value::Object(converted));
            }
            _ => {
                out.insert(k.clone(), v.clone());
            }
        }
    }
    if !out.contains_key("description") {
        out.insert("description".into(), json!(""));
    }
    Value::Object(out)
}

fn convert_header(h: &Value) -> Value {
    let Value::Object(src) = h else {
        return h.clone();
    };
    let mut out = Map::new();
    let mut schema = Map::new();
    for (k, v) in src {
        match k.as_str() {
            "description" => {
                out.insert(k.clone(), v.clone());
            }
            "collectionFormat" => {}
            k if k.starts_with("x-") && k != "x-nullable" => {
                out.insert(k.to_owned(), v.clone());
            }
            _ => {
                schema.insert(k.clone(), v.clone());
            }
        }
    }
    out.insert("schema".into(), Value::Object(schema));
    Value::Object(out)
}

fn request_body(body: Option<&Value>, form: &[&Value], consumes: &[String]) -> Option<Value> {
    if let Some(body) = body {
        let types: Vec<String> = if consumes.is_empty() {
            vec![DEFAULT_MEDIA_TYPE.to_owned()]
        } else {
            consumes
                .iter()
                .filter(|t| !is_form_media_type(t))
                .cloned()
                .collect()
        };
        let types = if types.is_empty() {
            vec![DEFAULT_MEDIA_TYPE.to_owned()]
        } else {
            types
        };
        let schema = body.get("schema").cloned().unwrap_or_else(|| json!({}));
        let content: Map<String, Value> = types
            .into_iter()
            .map(|t| (t, json!({ "schema": schema })))
            .collect();
        let mut rb = Map::new();
        if let Some(d) = body.get("description") {
            rb.insert("description".into(), d.clone());
        }
        rb.insert("content".into(), Value::Object(content));
        if body.get("required").and_then(Value::as_bool) == Some(true) {
            rb.insert("required".into(), json!(true));
        }
        return Some(Value::Object(rb));
    }
    if form.is_empty() {
        return None;
    }
    let has_file = form
        .iter()
        .any(|p| p.get("type").and_then(Value::as_str) == Some("file"));
    let media_type = if has_file || consumes.iter().any(|t| t == "multipart/form-data") {
        "multipart/form-data"
    } else {
        "application/x-www-form-urlencoded"
    };
    let mut properties = Map::new();
    let mut required = Vec::new();
    let mut encoding = Map::new();
    for p in form {
        let Some(name) = p.get("name").and_then(Value::as_str) else {
            continue;
        };
        let converted = convert_parameter(p);
        let mut schema = converted
            .get("schema")
            .cloned()
            .unwrap_or_else(|| json!({}));
        if let (Some(d), Value::Object(s)) = (p.get("description"), &mut schema) {
            s.insert("description".into(), d.clone());
        }
        properties.insert(name.to_owned(), schema);
        if p.get("required").and_then(Value::as_bool) == Some(true) {
            required.push(json!(name));
        }
        if let (Some(style), Some(explode)) = (converted.get("style"), converted.get("explode")) {
            encoding.insert(
                name.to_owned(),
                json!({ "style": style, "explode": explode }),
            );
        }
    }
    let mut schema = Map::new();
    schema.insert("type".into(), json!("object"));
    schema.insert("properties".into(), Value::Object(properties));
    if !required.is_empty() {
        schema.insert("required".into(), Value::Array(required));
    }
    let mut media = Map::new();
    media.insert("schema".into(), Value::Object(schema));
    if !encoding.is_empty() {
        media.insert("encoding".into(), Value::Object(encoding));
    }
    let mut content = Map::new();
    content.insert(media_type.into(), Value::Object(media));
    let mut rb = Map::new();
    rb.insert("content".into(), Value::Object(content));
    if form
        .iter()
        .any(|p| p.get("required").and_then(Value::as_bool) == Some(true))
    {
        rb.insert("required".into(), json!(true));
    }
    Some(Value::Object(rb))
}

fn is_form_media_type(t: &str) -> bool {
    t == "multipart/form-data" || t == "application/x-www-form-urlencoded"
}

fn convert_security_scheme(d: &Value) -> Value {
    let Value::Object(src) = d else {
        return d.clone();
    };
    let kind = src.get("type").and_then(Value::as_str).unwrap_or("");
    let mut out = Map::new();
    match kind {
        "basic" => {
            out.insert("type".into(), json!("http"));
            out.insert("scheme".into(), json!("basic"));
        }
        "apiKey" => {
            out.insert("type".into(), json!("apiKey"));
            for k in ["name", "in"] {
                if let Some(v) = src.get(k) {
                    out.insert(k.into(), v.clone());
                }
            }
        }
        "oauth2" => {
            out.insert("type".into(), json!("oauth2"));
            let flow = src.get("flow").and_then(Value::as_str).unwrap_or("");
            let name = match flow {
                "implicit" => "implicit",
                "password" => "password",
                "application" => "clientCredentials",
                "accessCode" => "authorizationCode",
                _ => "implicit",
            };
            let mut f = Map::new();
            for k in ["authorizationUrl", "tokenUrl"] {
                if let Some(v) = src.get(k) {
                    f.insert(k.into(), v.clone());
                }
            }
            f.insert(
                "scopes".into(),
                src.get("scopes").cloned().unwrap_or_else(|| json!({})),
            );
            out.insert("flows".into(), json!({ name: f }));
        }
        _ => return d.clone(),
    }
    for (k, v) in src {
        if k == "description" || k.starts_with("x-") {
            out.insert(k.clone(), v.clone());
        }
    }
    Value::Object(out)
}

/// Walks everything and converts `$ref` rewrites, `x-nullable`, `type: file`, and string `discriminator`s.
fn fix_schemas_and_refs(value: &mut Value) {
    fix_value(value, false);
}

/// Keys whose values are maps from names (property names, response codes, …) to objects. Keys in
/// those maps are names, so `default` or `example` there is not a keyword holding a plain value.
const NAME_MAPS: &[&str] = &[
    "properties",
    "patternProperties",
    "definitions",
    "schemas",
    "responses",
    "parameters",
    "headers",
    "paths",
    "callbacks",
    "securitySchemes",
    "requestBodies",
    "content",
    "encoding",
    "links",
];

/// `names` is true when the keys of `value` are names rather than keywords.
fn fix_value(value: &mut Value, names: bool) {
    match value {
        Value::Object(map) => {
            if !names {
                if let Some(Value::String(r)) = map.get_mut("$ref") {
                    *r = rewrite_ref(r);
                }
                if let Some(nullable) = map.remove("x-nullable") {
                    map.insert("nullable".into(), nullable);
                }
                if map.get("type").and_then(Value::as_str) == Some("file") {
                    map.insert("type".into(), json!("string"));
                    map.insert("format".into(), json!("binary"));
                }
                if let Some(Value::String(prop)) = map.get("discriminator") {
                    let prop = prop.clone();
                    map.insert("discriminator".into(), json!({ "propertyName": prop }));
                }
            }
            for (k, v) in map.iter_mut() {
                if !names
                    && (matches!(k.as_str(), "example" | "default" | "enum" | "examples")
                        || k.starts_with("x-"))
                {
                    continue;
                }
                fix_value(v, !names && NAME_MAPS.contains(&k.as_str()));
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| fix_value(v, false)),
        _ => {}
    }
}

fn rewrite_ref(r: &str) -> String {
    let Some(idx) = r.find('#') else {
        return r.to_owned();
    };
    let (doc, frag) = r.split_at(idx);
    for (from, to) in [
        ("#/definitions/", "#/components/schemas/"),
        ("#/parameters/", "#/components/parameters/"),
        ("#/responses/", "#/components/responses/"),
    ] {
        if let Some(rest) = frag.strip_prefix(from) {
            return format!("{doc}{to}{rest}");
        }
    }
    r.to_owned()
}

fn reorder(value: Value, first: &[&str]) -> Value {
    let Value::Object(mut map) = value else {
        return value;
    };
    let mut out = Map::new();
    for k in first {
        if let Some(v) = map.remove(*k) {
            out.insert((*k).to_owned(), v);
        }
    }
    out.extend(map);
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docset::Loc;
    use crate::fetch::MemoryFetcher;

    fn get<'a>(set: &'a DocumentSet, pointer: &str) -> &'a Value {
        set.get(&Loc::new(set.root_url().clone(), pointer)).unwrap()
    }

    #[test]
    fn versions() {
        assert_eq!(
            detect_version(&json!({"swagger": "2.0"})),
            Ok(SpecVersion::Swagger2)
        );
        assert_eq!(
            detect_version(&json!({"openapi": "3.0.3"})),
            Ok(SpecVersion::V3_0)
        );
        assert_eq!(
            detect_version(&json!({"openapi": "3.1.1"})),
            Ok(SpecVersion::V3_1)
        );
        assert_eq!(
            detect_version(&json!({"openapi": "3.2.0"})),
            Ok(SpecVersion::V3_2)
        );
        assert!(detect_version(&json!({"openapi": "4.0.0"})).is_err());
        assert_eq!(detect_version(&json!({})), Err(VersionError::Missing));
    }

    fn convert(doc: Value) -> DocumentSet {
        let mut set = DocumentSet::from_documents([(
            Url::parse("https://api.example.com/swagger.json").unwrap(),
            doc,
        )]);
        assert!(normalize_swagger2(&mut set).unwrap());
        set
    }

    #[test]
    fn openapi3_is_left_alone() {
        let mut set = DocumentSet::from_value(json!({"openapi": "3.1.0"}));
        assert!(!normalize_swagger2(&mut set).unwrap());
    }

    #[test]
    fn servers_from_host() {
        let set = convert(json!({
            "swagger": "2.0", "host": "api.example.com", "basePath": "/v1/",
            "schemes": ["https", "http"], "paths": {}
        }));
        assert_eq!(
            get(&set, "/servers"),
            &json!([{"url": "https://api.example.com/v1"}, {"url": "http://api.example.com/v1"}])
        );
        let set = convert(json!({ "swagger": "2.0", "basePath": "/v2", "paths": {} }));
        assert_eq!(get(&set, "/servers"), &json!([{"url": "/v2"}]));
        let set = convert(json!({ "swagger": "2.0", "host": "h.example", "paths": {} }));
        assert_eq!(
            get(&set, "/servers"),
            &json!([{"url": "https://h.example"}])
        );
    }

    #[test]
    fn definitions_and_refs() {
        let set = convert(json!({
            "swagger": "2.0",
            "paths": {},
            "definitions": {
                "Pet": {
                    "type": "object",
                    "discriminator": "kind",
                    "properties": {
                        "owner": { "$ref": "#/definitions/Owner" },
                        "photo": { "type": "file" },
                        "nick": { "type": "string", "x-nullable": true }
                    }
                },
                "Owner": { "type": "object" }
            }
        }));
        let pet = get(&set, "/components/schemas/Pet");
        assert_eq!(pet["discriminator"], json!({"propertyName": "kind"}));
        assert_eq!(
            pet["properties"]["owner"]["$ref"],
            "#/components/schemas/Owner"
        );
        assert_eq!(
            pet["properties"]["photo"],
            json!({"type": "string", "format": "binary"})
        );
        assert_eq!(
            pet["properties"]["nick"],
            json!({"type": "string", "nullable": true})
        );
        assert_eq!(get(&set, "/openapi"), "3.0.3");
        assert!(set.root().get("definitions").is_none());
    }

    #[test]
    fn refs_under_keys_named_default() {
        let set = convert(json!({
            "swagger": "2.0",
            "paths": {
                "/x": { "get": { "responses": {
                    "default": { "description": "e", "schema": { "$ref": "#/definitions/Q" } }
                } } }
            },
            "definitions": {
                "T": {
                    "type": "object",
                    "properties": {
                        "default": { "$ref": "#/definitions/Q" },
                        "example": { "additionalProperties": { "$ref": "#/definitions/Q" } },
                        "x-name": { "$ref": "#/definitions/Q" },
                        "plain": { "type": "string", "default": { "$ref": "#/definitions/Q" } }
                    }
                },
                "Q": { "type": "string" }
            }
        }));
        let q = json!("#/components/schemas/Q");
        let props = &get(&set, "/components/schemas/T")["properties"];
        assert_eq!(props["default"]["$ref"], q);
        assert_eq!(props["example"]["additionalProperties"]["$ref"], q);
        assert_eq!(props["x-name"]["$ref"], q);
        // A default value is left as is.
        assert_eq!(props["plain"]["default"]["$ref"], "#/definitions/Q");
        assert_eq!(
            get(
                &set,
                "/paths/~1x/get/responses/default/content/application~1json/schema/$ref"
            ),
            &q
        );
    }

    #[test]
    fn body_and_form_parameters() {
        let set = convert(json!({
            "swagger": "2.0",
            "consumes": ["application/json", "application/xml"],
            "produces": ["application/json"],
            "parameters": {
                "PetBody": { "name": "body", "in": "body", "required": true,
                             "schema": { "$ref": "#/definitions/Pet" } },
                "Limit": { "name": "limit", "in": "query", "type": "integer" }
            },
            "paths": {
                "/pets": {
                    "post": {
                        "parameters": [ { "$ref": "#/parameters/PetBody" },
                                        { "$ref": "#/parameters/Limit" } ],
                        "responses": { "200": { "description": "ok",
                                                "schema": { "$ref": "#/definitions/Pet" } } }
                    }
                },
                "/pets/{id}/photo": {
                    "parameters": [ { "name": "id", "in": "path", "type": "string" } ],
                    "post": {
                        "consumes": ["multipart/form-data"],
                        "parameters": [
                            { "name": "file", "in": "formData", "type": "file", "required": true },
                            { "name": "tags", "in": "formData", "type": "array",
                              "items": { "type": "string" }, "collectionFormat": "multi" }
                        ],
                        "responses": { "204": { "description": "done" } }
                    }
                },
                "/login": {
                    "post": {
                        "consumes": ["application/x-www-form-urlencoded"],
                        "parameters": [ { "name": "user", "in": "formData", "type": "string" } ],
                        "responses": { "default": { "description": "x" } }
                    }
                }
            },
            "definitions": { "Pet": { "type": "object" } }
        }));
        let post = get(&set, "/paths/~1pets/post");
        assert_eq!(
            post["requestBody"],
            json!({
                "content": {
                    "application/json": { "schema": { "$ref": "#/components/schemas/Pet" } },
                    "application/xml": { "schema": { "$ref": "#/components/schemas/Pet" } }
                },
                "required": true
            })
        );
        assert_eq!(
            post["parameters"],
            json!([{ "$ref": "#/components/parameters/Limit" }])
        );
        assert_eq!(
            post["responses"]["200"]["content"]["application/json"]["schema"]["$ref"],
            "#/components/schemas/Pet"
        );
        assert_eq!(
            get(&set, "/components/parameters"),
            &json!({ "Limit": { "name": "limit", "in": "query", "schema": { "type": "integer" } } })
        );

        let photo = get(&set, "/paths/~1pets~1{id}~1photo");
        assert_eq!(
            photo["parameters"],
            json!([{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }])
        );
        assert_eq!(
            photo["post"]["requestBody"],
            json!({
                "content": { "multipart/form-data": {
                    "schema": {
                        "type": "object",
                        "properties": {
                            "file": { "type": "string", "format": "binary" },
                            "tags": { "type": "array", "items": { "type": "string" } }
                        },
                        "required": ["file"]
                    },
                    "encoding": { "tags": { "style": "form", "explode": true } }
                }},
                "required": true
            })
        );
        let login = get(&set, "/paths/~1login/post");
        assert!(login["requestBody"]["content"]["application/x-www-form-urlencoded"].is_object());
    }

    #[test]
    fn query_arrays() {
        let set = convert(json!({
            "swagger": "2.0",
            "paths": { "/a": { "get": {
                "parameters": [
                    { "name": "csv", "in": "query", "type": "array", "items": { "type": "string" } },
                    { "name": "ssv", "in": "query", "type": "array", "collectionFormat": "ssv",
                      "items": { "type": "string" } },
                    { "name": "h", "in": "header", "type": "array", "items": { "type": "string" } }
                ],
                "responses": { "200": { "description": "ok", "headers": {
                    "X-Rate": { "type": "integer", "description": "rate" }
                }}}
            }}}
        }));
        let op = get(&set, "/paths/~1a/get");
        assert_eq!(op["parameters"][0]["style"], "form");
        assert_eq!(op["parameters"][0]["explode"], false);
        assert_eq!(op["parameters"][1]["style"], "spaceDelimited");
        assert_eq!(op["parameters"][2]["style"], "simple");
        assert_eq!(
            op["responses"]["200"]["headers"]["X-Rate"],
            json!({ "description": "rate", "schema": { "type": "integer" } })
        );
    }

    #[test]
    fn security_definitions() {
        let set = convert(json!({
            "swagger": "2.0", "paths": {},
            "securityDefinitions": {
                "basic": { "type": "basic" },
                "key": { "type": "apiKey", "name": "api_key", "in": "header" },
                "oauth": { "type": "oauth2", "flow": "accessCode",
                           "authorizationUrl": "https://a", "tokenUrl": "https://t",
                           "scopes": { "read": "r" } }
            }
        }));
        assert_eq!(
            get(&set, "/components/securitySchemes"),
            &json!({
                "basic": { "type": "http", "scheme": "basic" },
                "key": { "type": "apiKey", "name": "api_key", "in": "header" },
                "oauth": { "type": "oauth2", "flows": { "authorizationCode": {
                    "authorizationUrl": "https://a", "tokenUrl": "https://t", "scopes": { "read": "r" }
                }}}
            })
        );
    }

    #[test]
    fn path_level_body_parameter() {
        let set = convert(json!({
            "swagger": "2.0",
            "paths": { "/a": {
                "parameters": [ { "name": "b", "in": "body", "schema": { "type": "string" } } ],
                "put": { "responses": { "200": { "description": "ok" } } }
            }}
        }));
        let item = get(&set, "/paths/~1a");
        assert!(item.get("parameters").is_none());
        assert_eq!(
            item["put"]["requestBody"],
            json!({ "content": { "application/json": { "schema": { "type": "string" } } } })
        );
    }

    #[test]
    fn external_fragments() {
        let mut f = MemoryFetcher::new();
        f.insert(
            "file:///s/swagger.yaml",
            "swagger: '2.0'\npaths: {}\ndefinitions:\n  Pet:\n    $ref: 'defs.yaml#/definitions/Pet'\n",
        );
        f.insert(
            "file:///s/defs.yaml",
            "definitions:\n  Pet:\n    type: object\n    properties:\n      tag:\n        $ref: '#/definitions/Tag'\n  Tag:\n    type: string\n",
        );
        let mut set =
            DocumentSet::load(&Url::parse("file:///s/swagger.yaml").unwrap(), &f).unwrap();
        normalize_swagger2(&mut set).unwrap();
        let pet = Loc::new(set.root_url().clone(), "/components/schemas/Pet");
        let (loc, v) = set.deref(&pet).unwrap();
        assert_eq!(
            loc.to_string(),
            "file:///s/defs.yaml#/components/schemas/Pet"
        );
        assert_eq!(v["properties"]["tag"]["$ref"], "#/components/schemas/Tag");
    }

    #[test]
    fn petstore() {
        let bytes = include_bytes!("../tests/fixtures/petstore-swagger2.json");
        let value = crate::parse_document(bytes).unwrap();
        let mut set = DocumentSet::from_documents([(
            Url::parse("https://petstore.swagger.io/v2/swagger.json").unwrap(),
            value,
        )]);
        normalize_swagger2(&mut set).unwrap();
        let root = set.root();
        assert_eq!(detect_version(root), Ok(SpecVersion::V3_0));
        assert_eq!(root["servers"][0]["url"], "https://petstore.swagger.io/v2");
        let upload = &root["paths"]["/pet/{petId}/uploadImage"]["post"];
        assert!(
            upload["requestBody"]["content"]["multipart/form-data"]["schema"]["properties"]["file"]
                .get("format")
                .is_some_and(|f| f == "binary")
        );
        let add = &root["paths"]["/pet"]["post"];
        assert!(
            add["requestBody"]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .is_some_and(|r| r == "#/components/schemas/Pet")
        );
        let text = serde_json::to_string(root).unwrap();
        assert!(!text.contains("#/definitions/"));
        assert!(!text.contains("\"in\":\"body\""));
        assert!(!text.contains("\"in\":\"formData\""));
    }
}
