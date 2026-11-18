//! Lowers the model to the types TypeScript prints (design.md 18.5). Scalars become `number` and
//! `string`, enums become unions of literals, `oneOf`/`anyOf` become unions with discriminator tags,
//! inherited discriminators become literal objects; unions and intersections that changed are normalized again, so that `integer | number`
//! becomes `number` as before.

use crate::TypeScriptOptions;
use crate::naming::{self, Assigned};
use std::collections::HashMap;
use tsurara_sem::ir::{Literal, Object, Property, Tuple, Type};
use tsurara_sem::model::Model;
use tsurara_sem::naming::NameId;
use tsurara_sem::operation::{BodyKind, Callback, Operation};
use tsurara_sem::{Options, Warning, WarningKind};

/// What lowering needs besides the type: the assigned names, and where warnings go.
pub struct Ctx<'a> {
    names: Option<&'a Assigned>,
    /// The model's type of each declaration, to look into the members of a union.
    decls: HashMap<NameId, &'a Type>,
    warnings: Vec<Warning>,
}

/// Names the model and lowers it to what TypeScript prints. Returns the lowered copy, with the
/// identifiers filled in, and the warnings of naming and of TypeScript's approximations (such as
/// mapped types without a `typescript` entry).
pub fn lower_model(
    model: &Model,
    common: &Options,
    ts: &TypeScriptOptions,
) -> (Model, Vec<Warning>) {
    let (assigned, naming_warnings) = naming::assign(model, common, ts);
    let mut w = Ctx {
        names: Some(&assigned),
        decls: model.schemas.iter().map(|d| (d.id, &d.ty)).collect(),
        warnings: naming_warnings,
    };
    let mut model = model.clone();
    for decl in &mut model.schemas {
        decl.name = assigned.name(decl.id).to_owned();
        decl.ty = lower_with(&decl.ty, &mut w);
    }
    for op in model.operations.iter_mut().chain(model.webhooks.iter_mut()) {
        lower_operation(op, &mut w);
    }
    for c in &mut model.channels {
        if let Some(names) = w.names {
            c.ident = names.name(c.name_id).to_owned();
            c.type_names = names.type_names(c.name_id);
        }
        for p in &mut c.parameters {
            p.ty = lower_with(&p.ty, &mut w);
        }
        for content in &mut c.contents {
            content.ty = lower_with(&content.ty, &mut w);
        }
    }
    (model, w.warnings)
}

fn lower_operation(op: &mut Operation, w: &mut Ctx<'_>) {
    if let Some(names) = w.names {
        op.name = names.name(op.name_id).to_owned();
        op.type_names = names.type_names(op.name_id);
    }
    for p in &mut op.params {
        p.ty = lower_with(&p.ty, w);
    }
    if let Some(body) = &mut op.body {
        for c in &mut body.contents {
            c.ty = request_type(&c.media_type, c.body_kind, lower_with(&c.ty, w));
        }
    }
    for r in &mut op.responses {
        for c in &mut r.contents {
            // Typed events, for operations that got a name for them (not webhooks or callbacks).
            match (c.events.as_mut(), op.type_names.get("{P}Event")) {
                (Some(events), Some(name)) => {
                    events.ty = lower_with(&events.ty, w);
                    c.ty = Type::Builtin(format!("AsyncIterable<{name}>").into());
                }
                _ => {
                    c.events = None;
                    match (c.items.as_mut(), op.type_names.get("{P}Item")) {
                        (Some(items), Some(name)) => {
                            items.ty = lower_with(&items.ty, w);
                            c.ty = Type::Builtin(format!("AsyncIterable<{name}>").into());
                        }
                        _ => {
                            c.items = None;
                            c.ty = response_type(c.body_kind, lower_with(&c.ty, w));
                        }
                    }
                }
            }
        }
        for h in &mut r.headers {
            h.ty = lower_with(&h.ty, w);
        }
    }
    for Callback { operations, .. } in &mut op.callbacks {
        for o in operations.iter_mut() {
            lower_operation(o, w);
        }
    }
}

/// Type of a request body, from the schema type and how the body is carried.
fn request_type(media_type: &str, kind: BodyKind, schema: Type) -> Type {
    let mt = media_type.to_ascii_lowercase();
    if mt.starts_with("multipart/") || mt.starts_with("application/x-www-form-urlencoded") {
        return schema;
    }
    match kind {
        BodyKind::Json | BodyKind::Auto => schema,
        BodyKind::Text => string_like(schema),
        _ => Type::Union(vec![
            Type::builtin("Blob"),
            Type::builtin("ArrayBuffer"),
            Type::builtin("Uint8Array"),
            Type::builtin("ReadableStream<Uint8Array>"),
        ]),
    }
}

/// Type of a response body. Aligned with the type implied by `parseAs` so it matches the runtime value.
fn response_type(kind: BodyKind, schema: Type) -> Type {
    match kind {
        BodyKind::Json => schema,
        BodyKind::Text => string_like(schema),
        BodyKind::Blob => Type::builtin("Blob"),
        BodyKind::Stream => Type::builtin("ReadableStream<Uint8Array>"),
        BodyKind::None => Type::Undefined,
        BodyKind::Auto => Type::union(vec![schema, Type::String, Type::builtin("Blob")]),
    }
}

/// A body parsed as text becomes a string. String literals in the schema are kept.
fn string_like(schema: Type) -> Type {
    fn is_string(t: &Type) -> bool {
        match t {
            Type::String | Type::Literal(Literal::String(_)) => true,
            Type::Union(ts) => ts.iter().all(is_string),
            _ => false,
        }
    }
    if is_string(&schema) {
        schema
    } else {
        Type::String
    }
}

/// Lowers one type without names (references print their preferred names) and drops warnings.
pub fn lower(ty: &Type) -> Type {
    lower_with(
        ty,
        &mut Ctx {
            names: None,
            decls: HashMap::new(),
            warnings: Vec::new(),
        },
    )
}

fn lower_with(ty: &Type, w: &mut Ctx<'_>) -> Type {
    lower_changed(ty, w).unwrap_or_else(|| ty.clone())
}

/// The lowered type, or `None` if nothing in it changes. Unchanged unions and intersections are not
/// rebuilt, so their member order stays exactly as the model has it.
fn lower_changed(ty: &Type, w: &mut Ctx<'_>) -> Option<Type> {
    match ty {
        Type::Named(n) => Some(Type::Ref(match w.names {
            Some(names) => names.name(n.id).to_owned(),
            None => n.hint.clone(),
        })),
        Type::Inline(i) => Some(lower_with(&i.ty, w)),
        Type::Scalar(s) => Some(s.base()),
        Type::Enum(e) => Some(e.literal_union()),
        Type::Binary => Some(tsurara_sem::print::binary_union()),
        Type::Mapped(m) => Some(match m.entries.get("typescript") {
            Some(e) => match crate::external::check(e) {
                Ok(()) => Type::External(e.clone()),
                Err(why) => {
                    w.warnings.push(Warning {
                        kind: WarningKind::InvalidDefinition,
                        loc: m.loc.clone(),
                        message: format!("{why}; the schema's type is used"),
                        language: None,
                    });
                    lower_with(&m.schema, w)
                }
            },
            None => {
                w.warnings.push(Warning {
                    kind: WarningKind::UnmappedType,
                    loc: m.loc.clone(),
                    message: "no `typescript` entry; the schema's type is used".to_owned(),
                    language: None,
                });
                lower_with(&m.schema, w)
            }
        }),
        Type::Tag(t) => Some(t.object()),
        Type::Choice(c) => {
            // The members are lowered first, then tagged and joined into a union.
            let mut c = c.clone();
            if let Some(d) = &mut c.discriminator {
                // A member that pins the discriminator itself needs no tag.
                for (m, values) in c.members.iter().zip(&mut d.values) {
                    if pins(m, &d.property, values, &w.decls, 0) {
                        values.clear();
                    }
                }
            }
            c.members = c.members.iter().map(|m| lower_with(m, w)).collect();
            if let Some(d) = &mut c.discriminator {
                d.default = d.default.as_deref().map(|t| Box::new(lower_with(t, w)));
            }
            Some(c.union())
        }
        Type::Array(t) => lower_changed(t, w).map(Type::array),
        Type::Tuple(t) => {
            let items = lower_all(&t.items, w);
            let rest = t.rest.as_deref().map(|r| (lower_changed(r, w), r));
            let rest_changed = rest.as_ref().is_some_and(|(c, _)| c.is_some());
            if items.is_none() && !rest_changed {
                return None;
            }
            Some(Type::Tuple(Tuple {
                items: items.unwrap_or_else(|| t.items.clone()),
                required: t.required,
                rest: rest.map(|(c, r)| Box::new(c.unwrap_or_else(|| r.clone()))),
            }))
        }
        Type::Union(ts) => lower_all(ts, w).map(Type::union),
        Type::Intersection(ts) => lower_all(ts, w).map(Type::intersection),
        Type::Object(o) => {
            let mut changed = false;
            let properties: Vec<Property> = o
                .properties
                .iter()
                .map(|p| match lower_changed(&p.ty, w) {
                    Some(ty) => {
                        changed = true;
                        Property { ty, ..p.clone() }
                    }
                    None => p.clone(),
                })
                .collect();
            let index = o.index.as_deref().map(|i| match lower_changed(i, w) {
                Some(t) => {
                    changed = true;
                    Box::new(t)
                }
                None => Box::new(i.clone()),
            });
            // `{ a: string; [key: string]: number }` is a TypeScript error, so the index value type is
            // widened with the property types (design.md 6.4).
            if let (Some(i), false) = (&index, properties.is_empty()) {
                let mut all = vec![(**i).clone()];
                all.extend(properties.iter().map(|p| p.ty.clone()));
                if properties.iter().any(|p| p.optional) {
                    all.push(Type::Undefined);
                }
                return Some(Type::Object(Object {
                    index: Some(Box::new(Type::union(all))),
                    properties,
                }));
            }
            changed.then_some(Type::Object(Object { properties, index }))
        }
        _ => None,
    }
}

/// Whether `ty` requires `property` and allows only `values` for it, so that intersecting it with
/// `{ property: values }` changes nothing.
fn pins(
    ty: &Type,
    property: &str,
    values: &[String],
    decls: &HashMap<NameId, &Type>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    match ty {
        Type::Named(n) => decls
            .get(&n.id)
            .is_some_and(|t| pins(t, property, values, decls, depth + 1)),
        Type::Inline(i) => pins(&i.ty, property, values, decls, depth + 1),
        Type::Intersection(ts) => ts
            .iter()
            .any(|t| pins(t, property, values, decls, depth + 1)),
        Type::Tag(t) => t.property == property && t.values.iter().all(|v| values.contains(v)),
        Type::Object(o) => o
            .properties
            .iter()
            .find(|p| p.name == property)
            .is_some_and(|p| !p.optional && only_strings(&p.ty, values, decls, depth + 1)),
        _ => false,
    }
}

/// Whether `ty` is a string literal among `values`, or an enum or union of such.
fn only_strings(
    ty: &Type,
    values: &[String],
    decls: &HashMap<NameId, &Type>,
    depth: usize,
) -> bool {
    if depth > 8 {
        return false;
    }
    let is = |l: &Literal| matches!(l, Literal::String(s) if values.contains(s));
    match ty {
        Type::Literal(l) => is(l),
        Type::Enum(e) => !e.nullable && !e.values.is_empty() && e.values.iter().all(is),
        Type::Union(ts) => {
            !ts.is_empty() && ts.iter().all(|t| only_strings(t, values, decls, depth + 1))
        }
        Type::Named(n) => decls
            .get(&n.id)
            .is_some_and(|t| only_strings(t, values, decls, depth + 1)),
        Type::Inline(i) => only_strings(&i.ty, values, decls, depth + 1),
        _ => false,
    }
}

fn lower_all(ts: &[Type], w: &mut Ctx<'_>) -> Option<Vec<Type>> {
    let lowered: Vec<Option<Type>> = ts.iter().map(|t| lower_changed(t, w)).collect();
    if lowered.iter().all(Option::is_none) {
        return None;
    }
    Some(
        lowered
            .into_iter()
            .zip(ts)
            .map(|(l, t)| l.unwrap_or_else(|| t.clone()))
            .collect(),
    )
}
