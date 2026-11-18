//! Lowering of the model to `tir` for a capability set (design.md 18.5.3).

use crate::caps::{Capabilities, Enums, Inheritance};
use crate::tir::*;
use std::collections::{HashMap, HashSet, VecDeque};
use tsurara_loader::Loc;
use tsurara_sem::ir::{Choice, Enum, Inline, Literal, Object, Scalar, ScalarKind, Type};
use tsurara_sem::naming::NameId;
use tsurara_sem::operation::{BodyKind, Framing, Operation};
use tsurara_sem::{Model, Warning, WarningKind};

/// Lowers `model` for `caps`. Declarations of the model keep their positions (`DeclId(i)` is
/// `model.schemas[i]`); hoisted declarations follow in discovery order.
pub fn lower(model: &Model, caps: &Capabilities) -> (Module, Vec<Warning>) {
    let mut l = Lowerer {
        model,
        caps,
        decls: Vec::new(),
        by_name: model
            .schemas
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id, DeclId(i)))
            .collect(),
        hoisted: HashMap::new(),
        warnings: Vec::new(),
    };
    let sites = sites(model, &l.by_name);
    for (i, d) in model.schemas.iter().enumerate() {
        l.decls.push(Decl {
            preferred_name: d.preferred_name.clone(),
            origin: Origin::Schema(d.id),
            loc: d.loc.clone(),
            kind: DeclKind::Alias(Expr::Any),
            doc: d.doc.clone(),
            sites: sites[i],
        });
    }
    for (i, d) in model.schemas.iter().enumerate() {
        let cx = Cx {
            site: site_of(sites[i]),
            owner: d.preferred_name.clone(),
            loc: d.loc.clone(),
        };
        let kind = l.decl_kind(&d.ty, &cx);
        l.decls[i].kind = kind;
    }
    let operations = model.operations.iter().map(|op| l.operation(op)).collect();
    l.mark_recursion();
    (
        Module {
            decls: l.decls,
            operations,
        },
        l.warnings,
    )
}

struct Lowerer<'a> {
    model: &'a Model,
    caps: &'a Capabilities,
    decls: Vec<Decl>,
    by_name: HashMap<NameId, DeclId>,
    /// Hoisted inline schemas by location, so that one schema becomes one declaration.
    hoisted: HashMap<Loc, DeclId>,
    warnings: Vec<Warning>,
}

/// Where a type is being lowered: the direction of its values, and the owner for derived names.
#[derive(Clone)]
struct Cx {
    site: Site,
    owner: String,
    loc: Loc,
}

impl Cx {
    fn named(&self, suffix: &str) -> Cx {
        Cx {
            owner: format!("{}{suffix}", self.owner),
            ..self.clone()
        }
    }
}

fn site_of(s: Sites) -> Site {
    match (s.sent, s.received) {
        (true, false) => Site::Sent,
        (false, true) => Site::Received,
        _ => Site::Both,
    }
}

fn received(site: Site) -> bool {
    matches!(site, Site::Received | Site::Both)
}

impl Lowerer<'_> {
    fn warn(&mut self, loc: &Loc, message: String) {
        self.warnings.push(Warning {
            kind: WarningKind::Unsupported,
            loc: loc.clone(),
            message,
            language: None,
        });
    }

    /// The kind of a declaration from its whole type.
    fn decl_kind(&mut self, ty: &Type, cx: &Cx) -> DeclKind {
        match ty {
            Type::Object(o) if is_map(o) => DeclKind::Alias(self.expr(ty, cx)),
            Type::Object(o) => DeclKind::Record(self.record(o, cx)),
            Type::Enum(e) => DeclKind::Enum(self.enum_decl(e, cx.site)),
            Type::Choice(c) => self.choice_decl(c, cx),
            Type::Intersection(ms) => self.all_of(ms, cx),
            Type::Inline(i) => self.decl_kind(&i.ty, cx),
            other => DeclKind::Alias(self.expr(other, cx)),
        }
    }

    fn push_decl(&mut self, name: String, loc: Loc, site: Site, kind: DeclKind) -> DeclId {
        let id = DeclId(self.decls.len());
        self.decls.push(Decl {
            preferred_name: name,
            origin: Origin::Candidate,
            loc,
            kind,
            doc: None,
            sites: Sites {
                sent: matches!(site, Site::Sent | Site::Both),
                received: received(site),
            },
        });
        id
    }

    /// Pass 1: an inline schema the language cannot write inline becomes a declaration.
    fn hoist(&mut self, name: &str, loc: &Loc, ty: &Type, cx: &Cx) -> Expr {
        if let Some(&id) = self.hoisted.get(loc) {
            return Expr::Named {
                decl: id,
                site: cx.site,
            };
        }
        // Reserve the slot first, so that a schema containing itself refers to its declaration.
        let id = self.push_decl(
            name.to_owned(),
            loc.clone(),
            cx.site,
            DeclKind::Alias(Expr::Any),
        );
        self.hoisted.insert(loc.clone(), id);
        let inner = Cx {
            owner: name.to_owned(),
            loc: loc.clone(),
            site: cx.site,
        };
        let kind = self.decl_kind(ty, &inner);
        self.decls[id.0].kind = kind;
        Expr::Named {
            decl: id,
            site: cx.site,
        }
    }

    fn expr(&mut self, ty: &Type, cx: &Cx) -> Expr {
        let caps = *self.caps;
        match ty {
            Type::Unknown | Type::Undefined | Type::Ref(_) | Type::Builtin(_) => Expr::Any,
            Type::Never => Expr::Never,
            Type::Null => Expr::Null,
            Type::Boolean => Expr::Bool,
            Type::String => Expr::Scalar(Scalar::new(ScalarKind::String, None)),
            Type::Number => Expr::Scalar(Scalar::new(ScalarKind::Number, None)),
            Type::Scalar(s) => Expr::Scalar(s.clone()),
            Type::Binary => Expr::Bytes,
            Type::Literal(l) => self.literal(l),
            Type::Enum(e) => {
                let kind = DeclKind::Enum(self.enum_decl(e, cx.site));
                let id = self.push_decl(format!("{}Enum", cx.owner), cx.loc.clone(), cx.site, kind);
                Expr::Named {
                    decl: id,
                    site: cx.site,
                }
            }
            Type::Named(n) => match self.by_name.get(&n.id) {
                Some(&decl) => Expr::Named {
                    decl,
                    site: cx.site,
                },
                None => Expr::Any,
            },
            Type::Array(t) => Expr::List(Box::new(self.expr(t, &cx.named("Item")))),
            Type::Tuple(t) => {
                let items: Vec<Expr> = t
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, t)| self.expr(t, &cx.named(&format!("Item{}", i + 1))))
                    .collect();
                let rest = t.rest.as_deref().map(|r| self.expr(r, &cx.named("Item")));
                if caps.tuples {
                    return Expr::Tuple {
                        items,
                        required: t.required,
                        rest: rest.map(Box::new),
                    };
                }
                // Pass 9: a list of the members' union, or of `Any`.
                let mut all = items;
                all.extend(rest);
                all.dedup();
                let item = match all.len() {
                    1 => all.pop().expect("one"),
                    _ if caps.structural_unions && !all.is_empty() => Expr::Union(all),
                    _ => Expr::Any,
                };
                Expr::List(Box::new(item))
            }
            // Pass 11: an object with only `additionalProperties` is a map.
            Type::Object(o) if is_map(o) => {
                let index = o.index.as_deref().expect("a map has an index");
                Expr::Map(Box::new(self.expr(index, &cx.named("Value"))))
            }
            Type::Object(o) => {
                if caps.inline_records {
                    Expr::InlineRecord(self.record(o, cx))
                } else {
                    let name = format!("{}Record", cx.owner);
                    let loc = cx.loc.clone();
                    self.hoist(&name, &loc, ty, cx)
                }
            }
            Type::Inline(i) => self.inline(i, cx),
            Type::Union(ts) => self.union(ts, cx),
            Type::Choice(c) => {
                if !caps.structural_unions || (c.discriminator.is_some() && caps.tagged_unions) {
                    let name = format!("{}Choice", cx.owner);
                    let loc = cx.loc.clone();
                    self.hoist(&name, &loc, ty, cx)
                } else {
                    Expr::Union(c.members.iter().map(|m| self.expr(m, cx)).collect())
                }
            }
            Type::Intersection(ms) => {
                if caps.intersections {
                    Expr::Intersection(ms.iter().map(|m| self.expr(m, cx)).collect())
                } else {
                    let name = format!("{}Record", cx.owner);
                    let loc = cx.loc.clone();
                    self.hoist(&name, &loc, ty, cx)
                }
            }
            Type::Tag(t) => self.tag_type(&t.values),
            Type::Mapped(m) => Expr::External {
                entries: m.entries.clone(),
                schema: Box::new(self.expr(&m.schema, cx)),
            },
            Type::External(_) => Expr::Any,
        }
    }

    fn inline(&mut self, i: &Inline, cx: &Cx) -> Expr {
        let caps = *self.caps;
        let hoist = match &i.ty {
            Type::Object(o) => !caps.inline_records && !is_map(o),
            Type::Enum(_) => true,
            Type::Choice(c) => {
                !caps.structural_unions || (c.discriminator.is_some() && caps.tagged_unions)
            }
            _ => false,
        };
        if hoist {
            self.hoist(&i.preferred_name, &i.loc, &i.ty, cx)
        } else {
            let inner = Cx {
                owner: i.preferred_name.clone(),
                loc: i.loc.clone(),
                site: cx.site,
            };
            self.expr(&i.ty, &inner)
        }
    }

    /// Passes 4 and 8 for unions that are not `oneOf`/`anyOf`: nullable types and type arrays.
    fn union(&mut self, ts: &[Type], cx: &Cx) -> Expr {
        let nullable = ts.iter().any(|t| matches!(t, Type::Null));
        let others: Vec<&Type> = ts.iter().filter(|t| !matches!(t, Type::Null)).collect();
        let inner = if others.len() == 1 {
            self.expr(others[0], cx)
        } else if self.caps.structural_unions {
            Expr::Union(others.iter().map(|t| self.expr(t, cx)).collect())
        } else {
            let members = others.iter().map(|t| self.expr(t, cx)).collect();
            let name = format!("{}Union", cx.owner);
            let kind = DeclKind::UntaggedUnion(UntaggedUnion {
                members,
                discriminator: None,
            });
            Expr::Named {
                decl: self.push_decl(name, cx.loc.clone(), cx.site, kind),
                site: cx.site,
            }
        };
        if nullable {
            self.nullable(inner, cx)
        } else {
            inner
        }
    }

    fn nullable(&mut self, inner: Expr, cx: &Cx) -> Expr {
        if !self.caps.nullable_primitives && matches!(inner, Expr::Scalar(_) | Expr::Bool) {
            self.warn(
                &cx.loc,
                format!(
                    "a nullable primitive in `{}` is represented as any value",
                    cx.owner
                ),
            );
            return Expr::Any;
        }
        Expr::Nullable(Box::new(inner))
    }

    /// Pass 7.
    fn literal(&self, l: &Literal) -> Expr {
        if self.caps.literal_types {
            return Expr::Literal(l.clone());
        }
        match l {
            Literal::String(_) => Expr::Scalar(Scalar::new(ScalarKind::String, None)),
            Literal::Number(n) if n.is_i64() || n.is_u64() => {
                Expr::Scalar(Scalar::new(ScalarKind::Integer, None))
            }
            Literal::Number(_) => Expr::Scalar(Scalar::new(ScalarKind::Number, None)),
            Literal::Boolean(_) => Expr::Bool,
        }
    }

    fn tag_type(&self, values: &[String]) -> Expr {
        match values {
            [one] if self.caps.literal_types => Expr::Literal(Literal::String(one.clone())),
            _ => Expr::Scalar(Scalar::new(ScalarKind::String, None)),
        }
    }

    /// Pass 5.
    fn enum_decl(&self, e: &Enum, site: Site) -> EnumDecl {
        let integers = e
            .values
            .iter()
            .all(|v| matches!(v, Literal::Number(n) if n.is_i64() || n.is_u64()));
        let repr = match self.caps.enums {
            Enums::Any => EnumRepr::Native,
            Enums::IntegerOnly if integers => EnumRepr::Native,
            _ => EnumRepr::Constants,
        };
        EnumDecl {
            values: e.values.clone(),
            open: self.caps.strict_decoding && received(site),
            repr,
        }
    }

    /// Passes 3 and 6.
    fn choice_decl(&mut self, c: &Choice, cx: &Cx) -> DeclKind {
        // Each member has its own location, so that a member hoisted from there (an `allOf`) is
        // not taken for the union, which is hoisted at the union's location.
        let key = if c.exclusive { "oneOf" } else { "anyOf" };
        let members: Vec<Expr> = c
            .members
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let member = Cx {
                    loc: cx.loc.join(key).join(&i.to_string()),
                    ..cx.named(&(i + 1).to_string())
                };
                self.expr(m, &member)
            })
            .collect();
        let default = c
            .discriminator
            .as_ref()
            .and_then(|d| d.default.as_deref())
            .map(|t| self.expr(t, cx));
        match &c.discriminator {
            // A member no value selects (an inline member without a `const` or `enum` on the
            // property) cannot be decoded by its tag: the union is tried member by member.
            Some(d) if self.caps.tagged_unions && d.values.iter().all(|v| !v.is_empty()) => {
                DeclKind::TaggedUnion(TaggedUnion {
                    property: d.property.clone(),
                    variants: members
                        .into_iter()
                        .zip(&d.values)
                        .map(|(ty, values)| Variant {
                            values: values.clone(),
                            ty,
                        })
                        .collect(),
                    default: default.map(Box::new),
                    open: self.caps.strict_decoding && received(cx.site),
                })
            }
            _ if self.caps.structural_unions => {
                let mut all = members;
                all.extend(default);
                DeclKind::Alias(Expr::Union(all))
            }
            d => {
                let mut all = members;
                all.extend(default);
                DeclKind::UntaggedUnion(UntaggedUnion {
                    members: all,
                    discriminator: d.as_ref().map(|d| d.property.clone()),
                })
            }
        }
    }

    /// The types of an operation. Locations follow the definition, so that inline schemas hoisted
    /// from different places stay apart.
    fn operation(&mut self, op: &Operation) -> OperationTypes {
        let owner = tsurara_naming::pascal_case(&op.id);
        let params = op
            .params
            .iter()
            .map(|p| {
                let cx = Cx {
                    site: Site::Sent,
                    owner: format!("{owner}{}", tsurara_naming::pascal_case(&p.name)),
                    loc: op.loc.join("parameters").join(&p.name),
                };
                self.expr(&p.ty, &cx)
            })
            .collect();
        let body = op
            .body
            .iter()
            .flat_map(|b| &b.contents)
            .map(|c| match c.body_kind {
                // Multipart bodies are read as blobs, but sent from their schema's fields.
                BodyKind::Blob | BodyKind::Stream
                    if !c.media_type.to_ascii_lowercase().starts_with("multipart/") =>
                {
                    Expr::Bytes
                }
                _ => {
                    let cx = Cx {
                        site: Site::Sent,
                        owner: format!("{owner}Body"),
                        loc: op
                            .loc
                            .join("requestBody")
                            .join("content")
                            .join(&c.media_type),
                    };
                    self.expr(&c.ty, &cx)
                }
            })
            .collect();
        let responses = op
            .responses
            .iter()
            .map(|r| {
                r.contents
                    .iter()
                    .map(|c| {
                        let cx = Cx {
                            site: Site::Received,
                            owner: format!("{owner}Response"),
                            loc: op
                                .loc
                                .join("responses")
                                .join(&r.status.key())
                                .join("content")
                                .join(&c.media_type),
                        };
                        let string = Expr::Scalar(Scalar {
                            kind: ScalarKind::String,
                            format: None,
                        });
                        if let Some(events) = &c.events {
                            return self.expr(&events.ty, &cx);
                        }
                        if let Some(items) = &c.items {
                            return self.expr(&items.ty, &cx);
                        }
                        match c.body_kind {
                            BodyKind::None => Expr::Null,
                            BodyKind::Blob | BodyKind::Stream => Expr::Bytes,
                            BodyKind::Text => string,
                            BodyKind::Auto => {
                                Expr::Union(vec![self.expr(&c.ty, &cx), string, Expr::Bytes])
                            }
                            BodyKind::Json => self.expr(&c.ty, &cx),
                        }
                    })
                    .collect()
            })
            .collect();
        let sequences = op
            .responses
            .iter()
            .map(|r| {
                r.contents
                    .iter()
                    .map(|c| match (&c.events, &c.items) {
                        (Some(_), _) => Some(Sequence::Events),
                        (None, Some(items)) => Some(match items.framing {
                            Framing::Lines => Sequence::JsonLines,
                            Framing::RecordSeparator => Sequence::JsonSeq,
                        }),
                        (None, None) => None,
                    })
                    .collect()
            })
            .collect();
        OperationTypes {
            params,
            body,
            responses,
            sequences,
        }
    }

    /// Pass 2.
    fn all_of(&mut self, ms: &[Type], cx: &Cx) -> DeclKind {
        if self.caps.intersections {
            return DeclKind::Alias(Expr::Intersection(
                ms.iter().map(|m| self.expr(m, cx)).collect(),
            ));
        }
        let unwrap = |t: &Type| -> Type {
            match t {
                Type::Inline(i) => i.ty.clone(),
                other => other.clone(),
            }
        };
        let ms: Vec<Type> = ms.iter().map(unwrap).collect();
        // Members that become fields: objects, tags, and references to declarations that are records.
        let model = self.model;
        if let Some(i) = ms
            .iter()
            .position(|t| !record_like(model, t, &mut HashSet::new()))
        {
            // Empty objects add nothing (as in `{ type: object, $ref: … }`), so keeping the other
            // member loses nothing.
            let others_empty = ms.iter().enumerate().all(|(j, t)| {
                j == i
                    || matches!(t, Type::Object(o) if o.properties.is_empty() && o.index.is_none())
            });
            if !others_empty {
                self.warn(
                    &cx.loc,
                    format!(
                        "`allOf` of `{}` combines values that are not objects; only one member is kept",
                        cx.owner
                    ),
                );
            }
            // The kept member is this declaration itself, not a reference to another one, so that
            // a member hoisted at the same location does not refer back to it.
            let other = ms[i].clone();
            return self.decl_kind(&other, cx);
        }
        let refs: Vec<NameId> = ms
            .iter()
            .filter_map(|t| match t {
                Type::Named(n) => Some(n.id),
                _ => None,
            })
            .collect();
        // The fields of the members other than references.
        let mut own = Record {
            fields: Vec::new(),
            base: None,
            additional: None,
        };
        for t in &ms {
            match t {
                Type::Object(o) => {
                    let r = self.record(o, cx);
                    merge_fields(&mut own, r);
                }
                Type::Tag(tag) => {
                    let ty = self.tag_type(&tag.values);
                    merge_fields(
                        &mut own,
                        Record {
                            fields: vec![Field {
                                wire_name: tag.property.clone(),
                                ty,
                                presence: Presence::Required,
                                boxed: false,
                                doc: None,
                            }],
                            base: None,
                            additional: None,
                        },
                    );
                }
                _ => {}
            }
        }
        // A single base is inherited, unless the child redeclares one of its fields: most languages
        // cannot narrow an inherited field, so the child then gets every field.
        let base = match refs.as_slice() {
            [one] if self.caps.inheritance != Inheritance::None => {
                let mut names = HashSet::new();
                property_names(self.model, *one, &mut HashSet::new(), &mut names);
                let redeclares = own.fields.iter().any(|f| names.contains(&f.wire_name));
                if redeclares {
                    None
                } else {
                    self.by_name.get(one).copied()
                }
            }
            _ => None,
        };
        let record = match base {
            Some(_) => Record { base, ..own },
            None => {
                let mut inherited = Record {
                    fields: Vec::new(),
                    base: None,
                    additional: None,
                };
                for r in &refs {
                    let mut seen = HashSet::new();
                    self.copy_fields(*r, cx, &mut inherited, &mut seen);
                }
                merge_fields(&mut inherited, own);
                inherited
            }
        };
        DeclKind::Record(record)
    }

    /// Copies the fields of a referenced declaration (and of what it combines) into `record`.
    fn copy_fields(
        &mut self,
        id: NameId,
        cx: &Cx,
        record: &mut Record,
        seen: &mut HashSet<NameId>,
    ) {
        if !seen.insert(id) {
            return;
        }
        let model = self.model;
        let Some(decl) = model.schemas.iter().find(|d| d.id == id) else {
            return;
        };
        let parts: Vec<Type> = match &decl.ty {
            Type::Intersection(ms) => ms.clone(),
            other => vec![other.clone()],
        };
        for p in parts {
            match p {
                Type::Object(o) => {
                    let r = self.record(&o, cx);
                    merge_fields(record, r);
                }
                Type::Named(n) => self.copy_fields(n.id, cx, record, seen),
                _ => {}
            }
        }
    }

    /// Passes 8 and 11.
    fn record(&mut self, o: &Object, cx: &Cx) -> Record {
        let mut fields = Vec::with_capacity(o.properties.len());
        for p in &o.properties {
            let (nullable, inner) = match &p.ty {
                Type::Union(ts) if ts.iter().any(|t| matches!(t, Type::Null)) => {
                    let others: Vec<Type> = ts
                        .iter()
                        .filter(|t| !matches!(t, Type::Null))
                        .cloned()
                        .collect();
                    let inner = match others.len() {
                        1 => others.into_iter().next().expect("one"),
                        _ => Type::Union(others),
                    };
                    (true, inner)
                }
                other => (false, other.clone()),
            };
            let pcx = cx.named(&tsurara_naming::pascal_case(&p.name));
            let mut ty = self.expr(&inner, &pcx);
            if nullable
                && !self.caps.nullable_primitives
                && matches!(ty, Expr::Scalar(_) | Expr::Bool)
            {
                self.warn(
                    &cx.loc,
                    format!(
                        "the nullable primitive `{}` is represented as any value",
                        p.name
                    ),
                );
                ty = Expr::Any;
            }
            let presence = match (p.optional, nullable) {
                (false, false) => Presence::Required,
                (true, false) => Presence::Optional,
                (false, true) => Presence::Nullable,
                (true, true) => Presence::OptionalNullable,
            };
            fields.push(Field {
                wire_name: p.name.clone(),
                ty,
                presence,
                boxed: false,
                doc: p.doc.clone(),
            });
        }
        let additional = o
            .index
            .as_deref()
            .map(|i| Box::new(self.expr(i, &cx.named("Value"))));
        Record {
            fields,
            base: None,
            additional,
        }
    }

    /// Pass 10: fields through which a record contains itself get indirection.
    fn mark_recursion(&mut self) {
        if self.caps.recursion_by_value {
            return;
        }
        let edges: Vec<Vec<usize>> = self
            .decls
            .iter()
            .map(|d| match &d.kind {
                DeclKind::Record(r) => r.fields.iter().filter_map(|f| by_value(&f.ty)).collect(),
                // Unions and aliases hold their members by value too, so cycles through them need
                // a boxed field on the way.
                DeclKind::TaggedUnion(t) => t
                    .variants
                    .iter()
                    .map(|v| &v.ty)
                    .chain(t.default.as_deref())
                    .filter_map(by_value)
                    .collect(),
                DeclKind::UntaggedUnion(u) => u.members.iter().filter_map(by_value).collect(),
                DeclKind::Alias(e) => by_value(e).into_iter().collect(),
                DeclKind::Enum(_) => Vec::new(),
            })
            .collect();
        let reaches = |from: usize, to: usize| -> bool {
            let mut seen = HashSet::new();
            let mut queue = VecDeque::from([from]);
            while let Some(n) = queue.pop_front() {
                if n == to {
                    return true;
                }
                if seen.insert(n) {
                    queue.extend(edges[n].iter().copied());
                }
            }
            false
        };
        let mut boxed = Vec::new();
        for (i, d) in self.decls.iter().enumerate() {
            if let DeclKind::Record(r) = &d.kind {
                for (j, f) in r.fields.iter().enumerate() {
                    if by_value(&f.ty).is_some_and(|t| reaches(t, i)) {
                        boxed.push((i, j));
                    }
                }
            }
        }
        for (i, j) in boxed {
            if let DeclKind::Record(r) = &mut self.decls[i].kind {
                r.fields[j].boxed = true;
            }
        }
    }
}

/// The declaration a field contains by value (not through a list or map).
fn by_value(e: &Expr) -> Option<usize> {
    match e {
        Expr::Named { decl, .. } => Some(decl.0),
        Expr::Nullable(i) => by_value(i),
        _ => None,
    }
}

/// Adds fields; a later field with the same wire name replaces an earlier one.
/// An object with no properties of its own, only `additionalProperties`.
fn is_map(o: &Object) -> bool {
    o.properties.is_empty() && o.index.is_some()
}

/// Whether a type lowers to a record without intersections: an object, a tag, or a reference to a
/// declaration that does (through `allOf`s).
fn record_like(model: &Model, ty: &Type, seen: &mut HashSet<NameId>) -> bool {
    match ty {
        Type::Object(_) | Type::Tag(_) => true,
        Type::Inline(i) => record_like(model, &i.ty, seen),
        Type::Intersection(ms) => ms.iter().all(|m| record_like(model, m, seen)),
        Type::Named(n) => {
            if !seen.insert(n.id) {
                return true;
            }
            model
                .schemas
                .iter()
                .find(|d| d.id == n.id)
                .is_some_and(|d| record_like(model, &d.ty, seen))
        }
        _ => false,
    }
}

/// The property names of a declaration and of what it combines, as `copy_fields` would copy them.
fn property_names(
    model: &Model,
    id: NameId,
    seen: &mut HashSet<NameId>,
    out: &mut HashSet<String>,
) {
    if !seen.insert(id) {
        return;
    }
    let Some(decl) = model.schemas.iter().find(|d| d.id == id) else {
        return;
    };
    let parts: Vec<&Type> = match &decl.ty {
        Type::Intersection(ms) => ms.iter().collect(),
        other => vec![other],
    };
    for p in parts {
        match p {
            Type::Object(o) => out.extend(o.properties.iter().map(|p| p.name.clone())),
            Type::Named(n) => property_names(model, n.id, seen, out),
            _ => {}
        }
    }
}

fn merge_fields(into: &mut Record, from: Record) {
    for f in from.fields {
        match into.fields.iter_mut().find(|g| g.wire_name == f.wire_name) {
            Some(g) => *g = f,
            None => into.fields.push(f),
        }
    }
    if from.additional.is_some() {
        into.additional = from.additional;
    }
}

/// Which declarations of the model are sent and which are received (design.md 18.5.3, pass 5).
/// Declarations that no operation reaches count as both.
fn sites(model: &Model, by_name: &HashMap<NameId, DeclId>) -> Vec<Sites> {
    let n = model.schemas.len();
    let refs: Vec<Vec<usize>> = model
        .schemas
        .iter()
        .map(|d| {
            let mut out = Vec::new();
            named(&d.ty, &mut out);
            out.iter()
                .filter_map(|id| by_name.get(id))
                .map(|d| d.0)
                .collect()
        })
        .collect();
    let mut sent_seeds = Vec::new();
    let mut received_seeds = Vec::new();
    for op in &model.operations {
        seeds(op, false, &mut sent_seeds, &mut received_seeds);
    }
    for op in &model.webhooks {
        seeds(op, true, &mut sent_seeds, &mut received_seeds);
    }
    let reach = |seeds: Vec<NameId>| -> Vec<bool> {
        let mut seen = vec![false; n];
        let mut queue: VecDeque<usize> = seeds
            .iter()
            .filter_map(|id| by_name.get(id))
            .map(|d| d.0)
            .collect();
        while let Some(i) = queue.pop_front() {
            if !seen[i] {
                seen[i] = true;
                queue.extend(refs[i].iter().copied());
            }
        }
        seen
    };
    let sent = reach(sent_seeds);
    let recv = reach(received_seeds);
    (0..n)
        .map(|i| match (sent[i], recv[i]) {
            (false, false) => Sites {
                sent: true,
                received: true,
            },
            (s, r) => Sites {
                sent: s,
                received: r,
            },
        })
        .collect()
}

/// Seeds the directions from an operation. `incoming`: the other side sends the request (webhooks
/// and callbacks), so the body is received and the responses are sent.
fn seeds(op: &Operation, incoming: bool, sent: &mut Vec<NameId>, received: &mut Vec<NameId>) {
    let (req, resp) = if incoming {
        (&mut *received, &mut *sent)
    } else {
        (&mut *sent, &mut *received)
    };
    for p in &op.params {
        named(&p.ty, req);
    }
    if let Some(b) = &op.body {
        for c in &b.contents {
            named(&c.ty, req);
        }
    }
    for r in &op.responses {
        for c in &r.contents {
            named(&c.ty, resp);
        }
        for h in &r.headers {
            named(&h.ty, resp);
        }
    }
    for cb in &op.callbacks {
        for o in &cb.operations {
            seeds(o, !incoming, sent, received);
        }
    }
}

/// The name requests a type refers to.
fn named(ty: &Type, out: &mut Vec<NameId>) {
    match ty {
        Type::Named(n) => out.push(n.id),
        Type::Array(t) => named(t, out),
        Type::Tuple(t) => {
            t.items.iter().for_each(|i| named(i, out));
            if let Some(r) = &t.rest {
                named(r, out);
            }
        }
        Type::Union(ts) | Type::Intersection(ts) => ts.iter().for_each(|t| named(t, out)),
        Type::Choice(c) => {
            c.members.iter().for_each(|t| named(t, out));
            if let Some(d) = c.discriminator.as_ref().and_then(|d| d.default.as_deref()) {
                named(d, out);
            }
        }
        Type::Object(o) => {
            o.properties.iter().for_each(|p| named(&p.ty, out));
            if let Some(i) = &o.index {
                named(i, out);
            }
        }
        Type::Inline(i) => named(&i.ty, out),
        Type::Mapped(m) => named(&m.schema, out),
        _ => {}
    }
}
