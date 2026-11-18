//! Sample values from the model (design.md 18.7.1): one JSON value per declaration and per
//! operation parameter, request body and response body, for mocks in every language. The values are
//! deterministic: they come from the definition (examples, defaults, constraints) and from JSON
//! Pointers, never from a clock or a random source.
//!
//! Published for backends outside this repository, with one version for all tsurara crates,
//! released with the CLI and moving with `semVersion`. Stable: the sample functions.

use serde_json::{Map, Value, json};
use std::collections::HashMap;
use tsurara_loader::Loc;
use tsurara_sem::ir::{Doc, Literal, ScalarKind, Type};
use tsurara_sem::naming::NameId;
use tsurara_sem::operation::{BodyKind, MediaContent, Operation, ParamIn};
use tsurara_sem::schema::TypeDecl;
use tsurara_sem::{Model, Warning, WarningKind};

/// How many times a declaration may be entered while sampling one value. A reference that would enter
/// it again is left out where optional, and becomes `{}` with a `sample-cycle` warning where required.
const MAX_DEPTH: usize = 2;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Samples {
    /// One value per declaration of the model that has one, in model order.
    pub types: Vec<(NameId, Value)>,
    /// One entry per operation, in model order.
    pub operations: Vec<OperationSamples>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationSamples {
    pub operation_id: String,
    /// Every parameter that has a value, in definition order.
    pub params: Vec<ParamSample>,
    /// The request body, in the first media type that has a value.
    pub body: Option<BodySample>,
    /// Each documented response body, by status and media type.
    pub responses: Vec<ResponseSample>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ParamSample {
    pub name: String,
    pub location: ParamIn,
    pub required: bool,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BodySample {
    pub media_type: String,
    pub required: bool,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseSample {
    /// `200`, `4XX` or `default`.
    pub status: String,
    pub media_type: String,
    pub value: Value,
}

/// The samples of `model`, with warnings where a value could not follow the definition.
pub fn samples(model: &Model) -> (Samples, Vec<Warning>) {
    let mut s = Sampler {
        decls: model.schemas.iter().map(|d| (d.id, d)).collect(),
        stack: Vec::new(),
        warnings: Vec::new(),
        cuts: 0,
    };
    let types = model
        .schemas
        .iter()
        .filter_map(|d| s.named(d.id).map(|v| (d.id, v)))
        .collect();
    let operations = model.operations.iter().map(|op| s.operation(op)).collect();
    let mut warnings = s.warnings;
    warnings.sort();
    warnings.dedup();
    (Samples { types, operations }, warnings)
}

struct Sampler<'a> {
    decls: HashMap<NameId, &'a TypeDecl>,
    /// The declarations being sampled, outermost first.
    stack: Vec<NameId>,
    warnings: Vec<Warning>,
    /// How many required cycles were cut with `{}`. Optional properties and array items whose sample
    /// needed a cut are left out instead (and the cut's warning taken back).
    cuts: usize,
}

/// What a value is sampled for: the documentation (examples, defaults, constraints), a name to derive
/// strings from, and the location (for derived values and warnings).
#[derive(Clone, Copy)]
struct At<'a> {
    doc: Option<&'a Doc>,
    hint: &'a str,
    loc: &'a Loc,
}

impl<'a> At<'a> {
    fn tag(&self, name: &str) -> Option<&'a Value> {
        self.doc?
            .tags
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }

    fn number(&self, name: &str) -> Option<f64> {
        self.tag(name).and_then(Value::as_f64)
    }

    fn size(&self, name: &str) -> Option<usize> {
        self.tag(name)
            .and_then(Value::as_u64)
            .map(|n| n.min(64) as usize)
    }
}

/// Why sampling produced nothing.
enum Missing {
    /// The type has no value (`never`, `undefined`).
    None,
    /// A reference would enter a declaration more than `MAX_DEPTH` times.
    Cycle,
}

type Sampled = Result<Value, Missing>;

impl<'a> Sampler<'a> {
    fn warn(&mut self, kind: WarningKind, loc: &Loc, message: String) {
        self.warnings.push(Warning {
            kind,
            loc: loc.clone(),
            message,
            language: None,
        });
    }

    fn named(&mut self, id: NameId) -> Option<Value> {
        self.reference(id).ok()
    }

    fn reference(&mut self, id: NameId) -> Sampled {
        let Some(decl) = self.decls.get(&id).copied() else {
            return Ok(json!({}));
        };
        if self.stack.iter().filter(|s| **s == id).count() >= MAX_DEPTH {
            return Err(Missing::Cycle);
        }
        self.stack.push(id);
        let at = At {
            doc: decl.doc.as_ref(),
            hint: "",
            loc: &decl.loc,
        };
        let v = self.sample(&decl.ty, at);
        self.stack.pop();
        v
    }

    fn operation(&mut self, op: &Operation) -> OperationSamples {
        let loc = &op.loc;
        let params = op
            .params
            .iter()
            .filter_map(|p| {
                let ploc = loc.join("parameters").join(&p.name);
                let at = At {
                    doc: p.doc.as_ref(),
                    hint: &p.name,
                    loc: &ploc,
                };
                let value = self.sample(&p.ty, at).ok()?;
                Some(ParamSample {
                    name: p.name.clone(),
                    location: p.location,
                    required: p.required,
                    value,
                })
            })
            .collect();
        let body = op.body.as_ref().and_then(|b| {
            b.contents.iter().find_map(|c| {
                let value = self.content(c, b.doc.as_ref(), &loc.join("requestBody"))?;
                Some(BodySample {
                    media_type: c.media_type.clone(),
                    required: b.required,
                    value,
                })
            })
        });
        let mut responses = Vec::new();
        for r in &op.responses {
            let status = r.status.key();
            for c in &r.contents {
                let rloc = loc.join("responses").join(&status);
                if let Some(value) = self.content(c, None, &rloc) {
                    responses.push(ResponseSample {
                        status: status.clone(),
                        media_type: c.media_type.clone(),
                        value,
                    });
                }
            }
        }
        OperationSamples {
            operation_id: op.id.clone(),
            params,
            body,
            responses,
        }
    }

    /// The body of one media type. Streams have no single value.
    fn content(&mut self, c: &MediaContent, doc: Option<&Doc>, loc: &Loc) -> Option<Value> {
        if c.body_kind == BodyKind::Stream || c.body_kind == BodyKind::None {
            return None;
        }
        let at = At { doc, hint: "", loc };
        self.sample(&c.ty, at).ok()
    }

    fn sample(&mut self, ty: &Type, at: At<'_>) -> Sampled {
        // 1. An example, then a default, when it fits the type.
        for key in ["example", "default"] {
            if let Some(v) = at.tag(key) {
                if self.fits(ty, v, 0) {
                    return Ok(v.clone());
                }
            }
        }
        // 2. Values from the type.
        Ok(match ty {
            Type::Unknown | Type::External(_) => json!({}),
            Type::Never | Type::Undefined | Type::Ref(_) | Type::Builtin(_) => {
                return Err(Missing::None);
            }
            Type::Null => Value::Null,
            Type::Boolean => Value::Bool(true),
            Type::String => self.string(None, at),
            Type::Number => number(at, false),
            Type::Binary => Value::String("binary".into()),
            Type::Scalar(s) => match s.kind {
                ScalarKind::Integer => number(at, true),
                ScalarKind::Number => number(at, false),
                ScalarKind::String => {
                    let format = s
                        .format
                        .as_deref()
                        .or_else(|| at.tag("format").and_then(Value::as_str));
                    self.string(format, at)
                }
            },
            Type::Mapped(m) => return self.sample(&m.schema, at),
            Type::Inline(i) => {
                let inner = At { loc: &i.loc, ..at };
                return self.sample(&i.ty, inner);
            }
            Type::Enum(e) => match e.values.first() {
                Some(l) => literal(l),
                None => Value::Null,
            },
            Type::Literal(l) => literal(l),
            Type::Named(r) => return self.reference(r.id),
            Type::Array(item) => {
                let max = at.size("maxItems");
                if max == Some(0) {
                    return Ok(json!([]));
                }
                let min = at.size("minItems").unwrap_or(0);
                let item_at = At {
                    doc: None,
                    hint: at.hint,
                    loc: at.loc,
                };
                let (cuts, warned) = (self.cuts, self.warnings.len());
                let sampled = self.sample(item, item_at);
                if min == 0 && self.cuts > cuts {
                    self.cuts = cuts;
                    self.warnings.truncate(warned);
                    return Ok(json!([]));
                }
                match sampled {
                    Ok(v) => Value::Array(vec![v; min.max(1)]),
                    Err(_) if min == 0 => json!([]),
                    Err(e) => return Err(e),
                }
            }
            Type::Tuple(t) => {
                let mut items = Vec::new();
                for (i, item) in t.items.iter().take(t.required).enumerate() {
                    let loc = at.loc.join("prefixItems").join(&i.to_string());
                    let item_at = At {
                        doc: None,
                        hint: at.hint,
                        loc: &loc,
                    };
                    items.push(self.sample(item, item_at)?);
                }
                // `minItems` beyond the prefix: rest items.
                let min = at.size("minItems").unwrap_or(0);
                if items.len() < min {
                    let Some(rest) = &t.rest else {
                        return Err(Missing::None);
                    };
                    let rest_at = At {
                        doc: None,
                        hint: at.hint,
                        loc: at.loc,
                    };
                    let v = self.sample(rest, rest_at)?;
                    items.resize(min, v);
                }
                Value::Array(items)
            }
            Type::Union(ts) => return self.first(ts, at),
            Type::Choice(c) => {
                if let Some(d) = &c.discriminator {
                    for (i, m) in c.members.iter().enumerate() {
                        let Some(tag) = d.values.get(i).and_then(|v| v.first()) else {
                            continue;
                        };
                        let mut v = self.sample(m, at)?;
                        let tag = Value::String(tag.clone());
                        // A member whose own property does not allow the tag (a definition whose
                        // implicit mapping disagrees with an enum) keeps its own value.
                        let allowed = self
                            .property_type(m, &d.property, 0)
                            .is_none_or(|t| self.fits(t, &tag, 0));
                        if let (Value::Object(o), true) = (&mut v, allowed) {
                            o.insert(d.property.clone(), tag);
                        }
                        return Ok(v);
                    }
                }
                return self.first(&c.members, at);
            }
            Type::Intersection(ts) => {
                // Objects merge; an object and a value of another JSON type have no common value.
                // Arrays whose samples differ meet in the empty array.
                let mut merged = None::<Map<String, Value>>;
                let mut others: Vec<Value> = Vec::new();
                for t in ts.iter().filter(|t| **t != Type::Unknown) {
                    match self.sample(t, at)? {
                        Value::Object(o) => merged.get_or_insert_with(Map::new).extend(o),
                        v => others.push(v),
                    }
                }
                // A member whose additional properties do not allow a property of another member
                // (`additionalProperties: false`, or another type) leaves it out.
                if let Some(o) = &mut merged {
                    for t in ts {
                        let Some(obj) = self.object_of(t, 0) else {
                            continue;
                        };
                        let Some(index) = &obj.index else {
                            continue;
                        };
                        o.retain(|k, v| {
                            obj.properties.iter().any(|p| p.name == *k) || self.fits(index, v, 0)
                        });
                    }
                }
                match (merged, others.first()) {
                    (Some(_), Some(_)) => return Err(Missing::None),
                    (Some(o), None) => Value::Object(o),
                    (None, None) => json!({}),
                    (None, Some(first)) if others.iter().all(|v| v == first) => first.clone(),
                    (None, Some(_)) if others.iter().all(Value::is_array) => json!([]),
                    (None, Some(_)) => return Err(Missing::None),
                }
            }
            Type::Tag(t) => match t.values.first() {
                Some(v) => json!({ t.property.clone(): v }),
                None => json!({}),
            },
            Type::Object(o) => {
                let mut out = Map::new();
                for p in &o.properties {
                    let loc = at.loc.join("properties").join(&p.name);
                    let p_at = At {
                        doc: p.doc.as_ref(),
                        hint: &p.name,
                        loc: &loc,
                    };
                    let (cuts, warned) = (self.cuts, self.warnings.len());
                    let sampled = self.sample(&p.ty, p_at);
                    if p.optional && self.cuts > cuts {
                        self.cuts = cuts;
                        self.warnings.truncate(warned);
                        continue;
                    }
                    match sampled {
                        // A flattened `allOf` can keep `additionalProperties` of one member that the
                        // properties of another do not satisfy; such optional properties are left out.
                        Ok(v)
                            if p.optional
                                && o.index.as_ref().is_some_and(|ix| !self.fits(ix, &v, 0)) => {}
                        Ok(v) => {
                            out.insert(p.name.clone(), v);
                        }
                        Err(_) if p.optional => {}
                        Err(Missing::Cycle) => {
                            self.cuts += 1;
                            self.warn(
                                WarningKind::SampleCycle,
                                &loc,
                                format!(
                                    "`{}` is required and refers back to itself; its sample stops at depth {MAX_DEPTH} with `{{}}`",
                                    p.name
                                ),
                            );
                            out.insert(p.name.clone(), json!({}));
                        }
                        Err(e) => return Err(e),
                    }
                }
                Value::Object(out)
            }
        })
    }

    /// The first member with a value, preferring members that are not `null`.
    fn first(&mut self, members: &[Type], at: At<'_>) -> Sampled {
        let mut last = Err(Missing::None);
        for m in members.iter().filter(|m| **m != Type::Null) {
            match self.sample(m, at) {
                Ok(v) => return Ok(v),
                Err(e) => last = Err(e),
            }
        }
        if members.contains(&Type::Null) {
            return Ok(Value::Null);
        }
        last
    }

    fn string(&mut self, format: Option<&str>, at: At<'_>) -> Value {
        let fixed = match format {
            Some("date-time") => Some("2024-01-01T00:00:00Z".to_owned()),
            Some("date") => Some("2024-01-01".to_owned()),
            Some("time") => Some("00:00:00Z".to_owned()),
            Some("duration") => Some("P1D".to_owned()),
            Some("uuid") => Some(uuid(&at.loc.pointer)),
            Some("email" | "idn-email") => Some("user@example.com".to_owned()),
            Some("uri" | "url" | "iri") => Some("https://example.com/".to_owned()),
            Some("uri-reference" | "iri-reference") => Some("/".to_owned()),
            Some("hostname" | "idn-hostname") => Some("example.com".to_owned()),
            Some("ipv4") => Some("192.0.2.1".to_owned()),
            Some("ipv6") => Some("2001:db8::1".to_owned()),
            Some("byte") => Some("AAAA".to_owned()),
            Some("binary") => Some("binary".to_owned()),
            _ => None,
        };
        if let Some(v) = fixed {
            return Value::String(v);
        }
        if at.tag("pattern").is_some() {
            self.warn(
                WarningKind::SamplePattern,
                at.loc,
                "the sample cannot follow `pattern`; add an `example` to the schema".into(),
            );
        }
        let mut s: String = if at.hint.is_empty() {
            "string".into()
        } else {
            at.hint.to_owned()
        };
        if let Some(min) = at.size("minLength") {
            while s.chars().count() < min {
                s.push('x');
            }
        }
        if let Some(max) = at.size("maxLength") {
            s = s.chars().take(max).collect();
        }
        Value::String(s)
    }

    /// The object type `ty` is, through references and inline types.
    fn object_of<'t>(&'t self, ty: &'t Type, depth: usize) -> Option<&'t tsurara_sem::ir::Object> {
        if depth > 4 {
            return None;
        }
        match ty {
            Type::Object(o) => Some(o),
            Type::Named(r) => self.object_of(&self.decls.get(&r.id)?.ty, depth + 1),
            Type::Inline(i) => self.object_of(&i.ty, depth + 1),
            _ => None,
        }
    }

    /// The type of property `name` of an object type, through references and intersections.
    fn property_type<'t>(&'t self, ty: &'t Type, name: &str, depth: usize) -> Option<&'t Type> {
        if depth > 4 {
            return None;
        }
        match ty {
            Type::Object(o) => o.properties.iter().find(|p| p.name == name).map(|p| &p.ty),
            Type::Named(r) => {
                let decl = self.decls.get(&r.id)?;
                self.property_type(&decl.ty, name, depth + 1)
            }
            Type::Inline(i) => self.property_type(&i.ty, name, depth + 1),
            Type::Intersection(ts) => ts
                .iter()
                .find_map(|t| self.property_type(t, name, depth + 1)),
            _ => None,
        }
    }

    /// Whether `v` has the shape of `ty`: its JSON type, and enum and literal values. Constraints are
    /// not checked; an example that breaks them is the definition's own mistake.
    fn fits(&self, ty: &Type, v: &Value, depth: usize) -> bool {
        match ty {
            Type::Unknown | Type::External(_) => true,
            Type::Never | Type::Undefined | Type::Ref(_) | Type::Builtin(_) => false,
            Type::Null => v.is_null(),
            Type::Boolean => v.is_boolean(),
            Type::String | Type::Binary => v.is_string(),
            Type::Number => v.is_number(),
            Type::Scalar(s) => match s.kind {
                ScalarKind::Integer => v.as_f64().is_some_and(|n| n.fract() == 0.0),
                ScalarKind::Number => v.is_number(),
                ScalarKind::String => v.is_string(),
            },
            Type::Mapped(m) => self.fits(&m.schema, v, depth),
            Type::Inline(i) => self.fits(&i.ty, v, depth),
            Type::Enum(e) => {
                (e.nullable && v.is_null()) || e.values.iter().any(|l| literal(l) == *v)
            }
            Type::Literal(l) => literal(l) == *v,
            Type::Named(r) => {
                depth > 4
                    || self
                        .decls
                        .get(&r.id)
                        .is_none_or(|d| self.fits(&d.ty, v, depth + 1))
            }
            Type::Array(item) => match v {
                Value::Array(items) => {
                    depth > 6 || items.iter().all(|x| self.fits(item, x, depth + 1))
                }
                _ => false,
            },
            Type::Tuple(_) => v.is_array(),
            Type::Union(ts) => ts.iter().any(|t| self.fits(t, v, depth)),
            Type::Choice(c) => c.members.iter().any(|t| self.fits(t, v, depth)),
            Type::Intersection(ts) => ts.iter().all(|t| self.fits(t, v, depth)),
            Type::Tag(_) => v.is_object(),
            // Every property the value has must fit its type; missing ones are not checked.
            Type::Object(o) => match v {
                Value::Object(map) => {
                    depth > 6
                        || o.properties.iter().all(|p| {
                            map.get(&p.name)
                                .is_none_or(|x| self.fits(&p.ty, x, depth + 1))
                        })
                }
                _ => false,
            },
        }
    }
}

fn literal(l: &Literal) -> Value {
    match l {
        Literal::String(s) => Value::String(s.clone()),
        Literal::Number(n) => Value::Number(n.clone()),
        Literal::Boolean(b) => Value::Bool(*b),
    }
}

/// 1 for integers and 1.5 for numbers, moved into `minimum`/`maximum` (inclusive or exclusive) and
/// up to a `multipleOf`.
fn number(at: At<'_>, integer: bool) -> Value {
    let mut n: f64 = if integer { 1.0 } else { 1.5 };
    let step = if integer { 1.0 } else { 0.5 };
    // OpenAPI 3.0 writes exclusive bounds as booleans next to `minimum` and `maximum`.
    let exclusive = |name: &str| at.tag(name).and_then(Value::as_bool) == Some(true);
    if let Some(min) = at.number("minimum") {
        if exclusive("exclusiveMinimum") {
            if n <= min {
                n = min + step;
            }
        } else if n < min {
            n = min;
        }
    }
    if let Some(min) = at.number("exclusiveMinimum") {
        if n <= min {
            n = min + step;
        }
    }
    if let Some(max) = at.number("maximum") {
        if exclusive("exclusiveMaximum") {
            if n >= max {
                n = max - step;
            }
        } else if n > max {
            n = max;
        }
    }
    if let Some(max) = at.number("exclusiveMaximum") {
        if n >= max {
            n = max - step;
        }
    }
    if let Some(m) = at.number("multipleOf").filter(|m| *m > 0.0) {
        n = (n / m).ceil() * m;
    }
    if integer {
        n = n.ceil();
        json!(n as i64)
    } else {
        json!(n)
    }
}

/// A UUID derived from a JSON Pointer: the low 48 bits of its 64-bit FNV-1a hash as the node.
fn uuid(pointer: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in pointer.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("00000000-0000-4000-8000-{:012x}", hash & 0xffff_ffff_ffff)
}
