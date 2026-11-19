//! Serialization of parameters and request bodies (design.md 8.5), as in @tsurara/client and
//! tsurara-client for Python. Values are JSON values: generated code serializes typed values first.

use crate::def::{BodyDef, EncodingDef, ParamDef};
use crate::{Body, File, Part};
use serde_json::Value;

const COMPONENT_SAFE: &[u8] = b"-_.!~*'()";
const RESERVED_SAFE: &[u8] = b"-_.!~*'();,/:@$";

fn encode_with(text: &str, safe: &[u8]) -> String {
    let mut out = String::with_capacity(text.len());
    for &b in text.as_bytes() {
        if b.is_ascii_alphanumeric() || safe.contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// `encodeURIComponent`, or `encodeURI` without `#?&=+` for `allowReserved`.
pub fn encode(text: &str, allow_reserved: bool) -> String {
    encode_with(
        text,
        if allow_reserved {
            RESERVED_SAFE
        } else {
            COMPONENT_SAFE
        },
    )
}

pub fn component(text: &str) -> String {
    encode_with(text, COMPONENT_SAFE)
}

/// `JSON.stringify`.
pub fn json_text(value: &Value) -> String {
    serde_json::to_string(value).expect("JSON values serialize")
}

/// A value as text: strings as they are, numbers as JavaScript prints them, `null` empty.
pub fn to_text(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => match n.as_f64() {
            Some(f) if !n.is_i64() && !n.is_u64() && f.fract() == 0.0 && f.abs() < 1e21 => {
                format!("{}", f as i64)
            }
            _ => n.to_string(),
        },
        other => json_text(other),
    }
}

fn content_value(value: &Value, content_type: &str) -> String {
    if content_type.to_ascii_lowercase().contains("json") {
        json_text(value)
    } else {
        to_text(value)
    }
}

pub fn serialize_path_param(name: &str, value: &Value, def: &ParamDef) -> String {
    let style = def.style.as_deref().unwrap_or("simple");
    let explode = def.explode.unwrap_or(false);
    let enc = |s: &str| encode(s, def.allow_reserved);
    if let Some(ct) = &def.content_type {
        return enc(&content_value(value, ct));
    }
    let prefix = match style {
        "label" => ".",
        "matrix" => ";",
        _ => "",
    };
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(|v| enc(&to_text(v))).collect();
            if style == "matrix" {
                return if explode {
                    items.iter().map(|i| format!(";{name}={i}")).collect()
                } else {
                    format!(";{name}={}", items.join(","))
                };
            }
            let sep = if explode && style == "label" {
                "."
            } else {
                ","
            };
            format!("{prefix}{}", items.join(sep))
        }
        Value::Object(map) => {
            if explode {
                let sep = match style {
                    "label" => ".",
                    "matrix" => ";",
                    _ => ",",
                };
                let kv: Vec<String> = map
                    .iter()
                    .map(|(k, v)| format!("{}={}", enc(k), enc(&to_text(v))))
                    .collect();
                return format!("{prefix}{}", kv.join(sep));
            }
            let flat: Vec<String> = map
                .iter()
                .flat_map(|(k, v)| [enc(k), enc(&to_text(v))])
                .collect();
            if style == "matrix" {
                format!(";{name}={}", flat.join(","))
            } else {
                format!("{prefix}{}", flat.join(","))
            }
        }
        _ => {
            let text = enc(&to_text(value));
            if style == "matrix" {
                format!(";{name}={text}")
            } else {
                format!("{prefix}{text}")
            }
        }
    }
}

/// A segment that URL parsing would remove or merge: empty, `.` or `..` (also percent-encoded).
fn is_dot_or_empty(segment: &str) -> bool {
    let s = segment.replace("%2e", ".").replace("%2E", ".");
    s.is_empty() || s == "." || s == ".."
}

/// Embeds path parameters. Fails when one is missing or would make a whole segment empty, `.` or
/// `..`, which would change the path (spec/README.md).
pub fn expand_path(
    template: &str,
    values: &[(String, Value)],
    defs: &indexmap::IndexMap<String, ParamDef>,
) -> Result<String, String> {
    // Appended to a base URL, a path without a leading `/` could change the host or port.
    if !template.starts_with('/') {
        return Err(format!("the path {template:?} does not start with /"));
    }
    let default = ParamDef::default();
    let mut segments = Vec::new();
    for segment in template.split('/') {
        if !segment.contains('{') {
            segments.push(segment.to_owned());
            continue;
        }
        let mut expanded = String::new();
        let mut rest = segment;
        while let Some(start) = rest.find('{') {
            expanded.push_str(&rest[..start]);
            let end = rest[start..]
                .find('}')
                .map(|e| start + e)
                .unwrap_or(rest.len() - 1);
            let name = &rest[start + 1..end];
            let value = values
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v)
                .ok_or_else(|| format!("tsurara: the path parameter {name} is missing"))?;
            expanded.push_str(&serialize_path_param(
                name,
                value,
                defs.get(name).unwrap_or(&default),
            ));
            rest = &rest[end + 1..];
        }
        expanded.push_str(rest);
        if expanded.split('/').any(is_dot_or_empty) {
            return Err(format!(
                "tsurara: a path parameter value makes the segment {expanded:?}, which would change the path"
            ));
        }
        segments.push(expanded);
    }
    Ok(segments.join("/"))
}

pub fn serialize_query_param(name: &str, value: &Value, def: &ParamDef) -> Vec<String> {
    let style = def.style.as_deref().unwrap_or("form");
    let explode = def.explode.unwrap_or(style == "form");
    let enc = |s: &str| encode(s, def.allow_reserved);
    let key = component(name);
    if let Some(ct) = &def.content_type {
        return vec![format!("{key}={}", enc(&content_value(value, ct)))];
    }
    if style == "deepObject" {
        if let Value::Object(_) = value {
            let mut out = Vec::new();
            fn walk(
                prefix: String,
                v: &Value,
                out: &mut Vec<String>,
                enc: &dyn Fn(&str) -> String,
            ) {
                match v {
                    Value::Object(m) => {
                        for (k, x) in m {
                            walk(format!("{prefix}[{k}]"), x, out, enc);
                        }
                    }
                    Value::Array(items) => {
                        for x in items {
                            out.push(format!("{}={}", component(&prefix), enc(&to_text(x))));
                        }
                    }
                    _ => out.push(format!("{}={}", component(&prefix), enc(&to_text(v)))),
                }
            }
            walk(name.to_owned(), value, &mut out, &enc);
            return out;
        }
    }
    let sep = match style {
        "spaceDelimited" => "%20",
        "pipeDelimited" => "|",
        _ => ",",
    };
    match value {
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(|v| enc(&to_text(v))).collect();
            if explode {
                items.iter().map(|i| format!("{key}={i}")).collect()
            } else {
                vec![format!("{key}={}", items.join(sep))]
            }
        }
        Value::Object(map) => {
            if explode {
                map.iter()
                    .map(|(k, v)| format!("{}={}", component(k), enc(&to_text(v))))
                    .collect()
            } else {
                let flat: Vec<String> = map
                    .iter()
                    .flat_map(|(k, v)| [enc(k), enc(&to_text(v))])
                    .collect();
                vec![format!("{key}={}", flat.join(sep))]
            }
        }
        _ => vec![format!("{key}={}", enc(&to_text(value)))],
    }
}

/// The query string. `null` values are left out: RFC 6570, which the styles come from, treats
/// null as undefined.
pub fn build_query(
    values: &[(String, Value)],
    defs: &indexmap::IndexMap<String, ParamDef>,
) -> String {
    let default = ParamDef::default();
    values
        .iter()
        .filter(|(_, v)| !v.is_null())
        .flat_map(|(name, v)| serialize_query_param(name, v, defs.get(name).unwrap_or(&default)))
        .collect::<Vec<_>>()
        .join("&")
}

/// `in: querystring` (3.2): the whole query string as one value.
pub fn build_query_string(value: &Value, content_type: &str) -> String {
    if content_type
        .to_ascii_lowercase()
        .contains("x-www-form-urlencoded")
    {
        if let Value::Object(map) = value {
            let def = ParamDef {
                style: Some("form".into()),
                explode: Some(true),
                ..ParamDef::default()
            };
            return map
                .iter()
                .filter(|(_, v)| !v.is_null())
                .flat_map(|(k, v)| serialize_query_param(k, v, &def))
                .collect::<Vec<_>>()
                .join("&");
        }
    }
    component(&content_value(value, content_type))
}

pub fn serialize_header_param(value: &Value, def: &ParamDef) -> String {
    if let Some(ct) = &def.content_type {
        return content_value(value, ct);
    }
    match value {
        Value::Array(items) => items.iter().map(to_text).collect::<Vec<_>>().join(","),
        Value::Object(map) => {
            if def.explode.unwrap_or(false) {
                map.iter()
                    .map(|(k, v)| format!("{k}={}", to_text(v)))
                    .collect::<Vec<_>>()
                    .join(",")
            } else {
                map.iter()
                    .flat_map(|(k, v)| [k.clone(), to_text(v)])
                    .collect::<Vec<_>>()
                    .join(",")
            }
        }
        _ => to_text(value),
    }
}

pub fn build_cookie(
    values: &[(String, Value)],
    defs: &indexmap::IndexMap<String, ParamDef>,
) -> String {
    let default = ParamDef::default();
    let mut parts = Vec::new();
    for (name, v) in values {
        let def = defs.get(name).unwrap_or(&default);
        let explode = def.explode.unwrap_or(true);
        if let Some(ct) = &def.content_type {
            parts.push(format!("{name}={}", component(&content_value(v, ct))));
            continue;
        }
        match v {
            Value::Array(items) => {
                let items: Vec<String> = items.iter().map(|x| component(&to_text(x))).collect();
                if explode {
                    parts.extend(items.iter().map(|i| format!("{name}={i}")));
                } else {
                    parts.push(format!("{name}={}", items.join(",")));
                }
            }
            Value::Object(map) => {
                if explode {
                    parts.extend(
                        map.iter()
                            .map(|(k, x)| format!("{k}={}", component(&to_text(x)))),
                    );
                } else {
                    let flat: Vec<String> = map
                        .iter()
                        .flat_map(|(k, x)| [k.clone(), component(&to_text(x))])
                        .collect();
                    parts.push(format!("{name}={}", flat.join(",")));
                }
            }
            _ => parts.push(format!("{name}={}", component(&to_text(v)))),
        }
    }
    parts.join("; ")
}

pub fn media_type_of(content_type: Option<&str>) -> String {
    content_type
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}

/// The body and its Content-Type (none for no header).
pub struct Encoded {
    pub content: Vec<u8>,
    pub content_type: Option<String>,
}

fn quote_param(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\r', "%0D")
        .replace('\n', "%0A")
}

fn multipart(
    parts: &[(String, Part)],
    encoding: &indexmap::IndexMap<String, EncodingDef>,
) -> Encoded {
    let boundary = format!(
        "tsurara{:016x}{:016x}",
        fastrand::u64(..),
        fastrand::u64(..)
    );
    let mut out = Vec::new();
    let mut part =
        |name: &str, content: &[u8], content_type: Option<&str>, filename: Option<&str>| {
            let mut head = format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{}\"",
                quote_param(name)
            );
            if let Some(f) = filename {
                head.push_str(&format!("; filename=\"{}\"", quote_param(f)));
            }
            head.push_str("\r\n");
            if let Some(ct) = content_type {
                head.push_str(&format!("Content-Type: {ct}\r\n"));
            }
            head.push_str("\r\n");
            out.extend_from_slice(head.as_bytes());
            out.extend_from_slice(content);
            out.extend_from_slice(b"\r\n");
        };
    let mut append = |name: &str, value: &Part, enc: Option<&EncodingDef>| {
        let enc_type = enc.and_then(|e| e.content_type.as_deref());
        match value {
            Part::File(f) => part(
                name,
                &f.content,
                Some(
                    f.content_type
                        .as_deref()
                        .unwrap_or("application/octet-stream"),
                ),
                f.filename.as_deref(),
            ),
            Part::Value(v @ (Value::Object(_) | Value::Array(_))) if enc_type.is_some() => {
                part(name, json_text(v).as_bytes(), enc_type, None)
            }
            // A field with the JSON text, as the TypeScript runtime sends it (FormData can only
            // give a part a Content-Type by making it a file).
            Part::Value(v @ Value::Object(_)) => part(name, json_text(v).as_bytes(), None, None),
            Part::Value(v) if enc_type.is_some_and(|t| t.to_ascii_lowercase().contains("json")) => {
                part(name, json_text(v).as_bytes(), enc_type, None)
            }
            Part::Value(v) => part(name, to_text(v).as_bytes(), None, None),
        }
    };
    for (name, value) in parts {
        let enc = encoding.get(name);
        match value {
            Part::Value(Value::Array(items))
                if enc.and_then(|e| e.content_type.as_ref()).is_none() =>
            {
                for item in items {
                    append(name, &Part::Value(item.clone()), enc);
                }
            }
            _ => append(name, value, enc),
        }
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Encoded {
        content: out,
        content_type: Some(format!("multipart/form-data; boundary={boundary}")),
    }
}

/// Builds the request body for the requested media type, or the first one of the definition.
pub fn encode_body(body: &Body, def: &BodyDef, requested: Option<&str>) -> Result<Encoded, String> {
    let content_type = requested
        .map(str::to_owned)
        .or_else(|| def.contents.keys().next().cloned())
        .unwrap_or_else(|| "application/json".into());
    let mt = media_type_of(Some(&content_type));
    let media = def
        .contents
        .get(&content_type)
        .or_else(|| def.contents.get(&mt));
    let empty = indexmap::IndexMap::new();
    let encoding = media.map_or(&empty, |m| &m.encoding);
    if mt.starts_with("multipart/") {
        return Ok(match body {
            Body::Multipart(parts) => multipart(parts, encoding),
            Body::Value(Value::Object(map)) => {
                let parts: Vec<(String, Part)> = map
                    .iter()
                    .map(|(k, v)| (k.clone(), Part::Value(v.clone())))
                    .collect();
                multipart(&parts, encoding)
            }
            _ => multipart(&[], encoding),
        });
    }
    let binary = |f: &File, ct: Option<String>| Encoded {
        content: f.content.clone(),
        content_type: ct,
    };
    if mt == "application/x-www-form-urlencoded" {
        let mut parts = Vec::new();
        if let Body::Value(Value::Object(map)) = body {
            for (name, v) in map {
                let e = encoding.get(name);
                let def = ParamDef {
                    style: Some(
                        e.and_then(|e| e.style.clone())
                            .unwrap_or_else(|| "form".into()),
                    ),
                    explode: Some(e.and_then(|e| e.explode).unwrap_or(true)),
                    allow_reserved: false,
                    content_type: e.and_then(|e| e.content_type.clone()),
                };
                parts.extend(serialize_query_param(name, v, &def));
            }
        }
        return Ok(Encoded {
            content: parts.join("&").replace("%20", "+").into_bytes(),
            content_type: Some(content_type),
        });
    }
    let value = match body {
        Body::Binary(f) => {
            if mt.contains('*') {
                return Ok(binary(f, f.content_type.clone()));
            }
            if mt == "application/json" || mt.ends_with("+json") {
                return Err(format!("binary values cannot be sent as a {mt} body"));
            }
            return Ok(binary(f, Some(content_type)));
        }
        Body::Multipart(_) => return Err(format!("parts cannot be sent as a {mt} body")),
        Body::Value(v) => v,
    };
    if mt.starts_with("text/") {
        return Ok(Encoded {
            content: to_text(value).into_bytes(),
            content_type: Some(content_type),
        });
    }
    if mt.contains('*') {
        return Ok(Encoded {
            content: json_text(value).into_bytes(),
            content_type: Some("application/json".into()),
        });
    }
    if mt == "application/json" || mt.ends_with("+json") {
        return Ok(Encoded {
            content: json_text(value).into_bytes(),
            content_type: Some(content_type),
        });
    }
    Err(format!("a {mt} body takes a binary value"))
}
