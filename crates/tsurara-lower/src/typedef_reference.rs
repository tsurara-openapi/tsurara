//! A reference converter for TypeDefs, used to check the conformance vectors and the TypeDefs that
//! backends emit. It is not a runtime: it works on values in a neutral JSON form, where records are
//! objects with `$type` and member names, unlisted enum and union values are `{ "$unknown": value }`,
//! external values are `{ "$external": converter, "value": value }`, and extra properties go in
//! `$additional`.

use serde_json::{Map, Value, json};
use std::collections::BTreeSet;

/// Converts with a TypeDef document (`spec/type-def.schema.json`).
pub struct Converter<'a>(&'a Map<String, Value>);

impl<'a> Converter<'a> {
    /// `type_defs` is a whole TypeDef document (`{ "v": 1, "types": { ... } }`).
    pub fn new(type_defs: &'a Value) -> Self {
        Converter(type_defs["types"].as_object().expect("a TypeDef document"))
    }

    /// Decodes `json` as `ty`.
    pub fn decode(&self, ty: &Value, json: &Value) -> Result<Value, String> {
        match ty {
            Value::String(k) => match (k.as_str(), json) {
                ("any", v) => Ok(v.clone()),
                ("null", Value::Null) => Ok(Value::Null),
                ("bool", Value::Bool(b)) => Ok(json!(b)),
                _ => Err(format!("{json} is not {k}")),
            },
            Value::Object(t) => {
                if let Some(kind) = t.get("scalar").and_then(Value::as_str) {
                    return match (kind, json) {
                        ("string", Value::String(_)) => Ok(json.clone()),
                        ("number", Value::Number(_)) => Ok(json.clone()),
                        ("integer", Value::Number(n)) => match n.as_f64() {
                            Some(f) if f.fract() == 0.0 => Ok(json!(f as i64)),
                            _ => Err(format!("{json} is not an integer")),
                        },
                        _ => Err(format!("{json} is not {kind}")),
                    };
                }
                if let Some(item) = t.get("list") {
                    let a = json.as_array().ok_or("not a list")?;
                    return a
                        .iter()
                        .map(|v| self.decode(item, v))
                        .collect::<Result<_, _>>()
                        .map(Value::Array);
                }
                if let Some(item) = t.get("map") {
                    let o = json.as_object().ok_or("not a map")?;
                    return o
                        .iter()
                        .map(|(k, v)| Ok((k.clone(), self.decode(item, v)?)))
                        .collect::<Result<Map<_, _>, String>>()
                        .map(Value::Object);
                }
                if let Some(items) = t.get("tuple").and_then(Value::as_array) {
                    let a = json.as_array().ok_or("not a tuple")?;
                    let required = t["required"].as_u64().unwrap_or(0) as usize;
                    if a.len() < required {
                        return Err("too few tuple items".into());
                    }
                    return a
                        .iter()
                        .enumerate()
                        .map(|(i, v)| match items.get(i).or(t.get("rest")) {
                            Some(ty) => self.decode(ty, v),
                            None => Err("too many tuple items".into()),
                        })
                        .collect::<Result<_, _>>()
                        .map(Value::Array);
                }
                if let Some(name) = t.get("ref").and_then(Value::as_str) {
                    return self.decode_decl(name, json);
                }
                if let Some(inner) = t.get("nullable") {
                    return if json.is_null() {
                        Ok(Value::Null)
                    } else {
                        self.decode(inner, json)
                    };
                }
                if let Some(conv) = t.get("external") {
                    return Ok(
                        json!({ "$external": conv, "value": self.decode(&t["schema"], json)? }),
                    );
                }
                if let Some(l) = t.get("literal") {
                    return if l == json {
                        Ok(json.clone())
                    } else {
                        Err(format!("{json} is not {l}"))
                    };
                }
                if let Some(ms) = t.get("union").and_then(Value::as_array) {
                    return ms
                        .iter()
                        .find_map(|m| self.decode(m, json).ok())
                        .ok_or_else(|| format!("{json} matches no member"));
                }
                Err(format!("unknown type {ty}"))
            }
            _ => Err(format!("unknown type {ty}")),
        }
    }

    fn decode_decl(&self, name: &str, json: &Value) -> Result<Value, String> {
        let d = &self.0[name];
        match d["kind"].as_str() {
            Some("record") => {
                let o = json.as_object().ok_or("not an object")?;
                let mut out = Map::new();
                out.insert("$type".into(), json!(name));
                self.decode_fields(d, o, &mut out)?;
                Ok(Value::Object(out))
            }
            Some("enum") => {
                let values = d["values"].as_array().unwrap();
                if values.contains(json) {
                    Ok(json.clone())
                } else if d["open"] == true {
                    Ok(json!({ "$unknown": json }))
                } else {
                    Err(format!("{json} is not listed"))
                }
            }
            Some("taggedUnion") => {
                let tag = &json[d["property"].as_str().unwrap()];
                let variant = d["variants"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["values"].as_array().unwrap().contains(tag));
                match (variant, d.get("default")) {
                    (Some(v), _) => self.decode(&v["type"], json),
                    (None, Some(def)) => self.decode(def, json),
                    (None, None) if d["open"] == true => Ok(json!({ "$unknown": json })),
                    _ => Err(format!("unlisted discriminator {tag}")),
                }
            }
            Some("untaggedUnion") => d["members"]
                .as_array()
                .unwrap()
                .iter()
                .find_map(|m| self.decode(m, json).ok())
                .ok_or_else(|| format!("{json} matches no member")),
            Some("alias") => self.decode(&d["type"], json),
            _ => Err(format!("unknown declaration {name}")),
        }
    }

    fn decode_fields(
        &self,
        d: &Value,
        o: &Map<String, Value>,
        out: &mut Map<String, Value>,
    ) -> Result<(), String> {
        if let Some(base) = d.get("base").and_then(Value::as_str) {
            self.decode_fields(&self.0[base], o, out)?;
        }
        let fields = d["fields"].as_array().unwrap();
        for f in fields {
            let wire = f["wire"].as_str().unwrap();
            let member = f["member"].as_str().unwrap();
            let presence = f["presence"].as_str().unwrap();
            match o.get(wire) {
                None if matches!(presence, "required" | "nullable") => {
                    return Err(format!("`{wire}` is missing"));
                }
                None => {}
                Some(Value::Null) if matches!(presence, "nullable" | "optionalNullable") => {
                    out.insert(member.into(), Value::Null);
                }
                Some(v) => {
                    out.insert(member.into(), self.decode(&f["type"], v)?);
                }
            }
        }
        if let Some(extra) = d.get("additional") {
            let known: BTreeSet<&str> = self.wire_names(d);
            let mut add = Map::new();
            for (k, v) in o {
                if !known.contains(k.as_str()) {
                    add.insert(k.clone(), self.decode(extra, v)?);
                }
            }
            if !add.is_empty() {
                out.insert("$additional".into(), Value::Object(add));
            }
        }
        Ok(())
    }

    fn wire_names<'b>(&'b self, d: &'b Value) -> BTreeSet<&'b str> {
        let mut out: BTreeSet<&str> = d["fields"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["wire"].as_str().unwrap())
            .collect();
        if let Some(base) = d.get("base").and_then(Value::as_str) {
            out.extend(self.wire_names(&self.0[base]));
        }
        out
    }

    /// Encodes a value in neutral form as `ty`.
    pub fn encode(&self, ty: &Value, v: &Value) -> Value {
        if let Some(u) = v.get("$unknown") {
            return u.clone();
        }
        if let Some(e) = v.get("$external") {
            let schema = &ty
                .get("schema")
                .cloned()
                .unwrap_or_else(|| self.external_schema(ty, e));
            return self.encode(schema, &v["value"]);
        }
        if let Some(name) = v.get("$type").and_then(Value::as_str) {
            let mut out = Map::new();
            self.encode_fields(&self.0[name], v, &mut out);
            if let Some(Value::Object(add)) = v.get("$additional") {
                out.extend(add.clone());
            }
            return Value::Object(out);
        }
        match (ty.get("list"), v) {
            (Some(item), Value::Array(a)) => {
                Value::Array(a.iter().map(|x| self.encode(item, x)).collect())
            }
            _ => v.clone(),
        }
    }

    fn external_schema(&self, ty: &Value, _: &Value) -> Value {
        let name = ty["ref"].as_str().unwrap();
        self.0[name]["type"]["schema"].clone()
    }

    fn encode_fields(&self, d: &Value, v: &Value, out: &mut Map<String, Value>) {
        if let Some(base) = d.get("base").and_then(Value::as_str) {
            self.encode_fields(&self.0[base], v, out);
        }
        for f in d["fields"].as_array().unwrap() {
            let member = f["member"].as_str().unwrap();
            if let Some(x) = v.get(member) {
                out.insert(
                    f["wire"].as_str().unwrap().into(),
                    self.encode(&f["type"], x),
                );
            }
        }
    }
}
