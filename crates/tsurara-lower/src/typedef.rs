//! TypeDefs (design.md 18.7, `spec/type-def.schema.json`): type metadata built from `tir`, for runtimes
//! that convert between JSON and typed values generically.

use crate::tir::*;

#[path = "typedef_reference.rs"]
pub mod reference;
use serde_json::{Map, Value, json};
use tsurara_sem::ir::{Literal, ScalarKind};

/// Version of the TypeDef format.
pub const TYPE_DEF_VERSION: u32 = 1;

/// Builds the TypeDefs of `module`. `names[i]` is the backend's name of declaration `i`, `member`
/// gives the backend's member name of a field (by declaration and wire name), and `language` selects
/// external type entries.
pub fn type_defs(
    module: &Module,
    names: &[String],
    member: &dyn Fn(DeclId, &str) -> String,
    language: &str,
) -> Value {
    let b = Builder {
        names,
        member,
        language,
    };
    let types: Map<String, Value> = module
        .decls
        .iter()
        .enumerate()
        .map(|(i, d)| (names[i].clone(), b.decl(DeclId(i), &d.kind)))
        .collect();
    json!({ "v": TYPE_DEF_VERSION, "types": types })
}

/// The TypeDef type of an expression (`{"ref": "Pet"}`, `{"list": ...}`), for references from
/// OperationDefs (`type` of parameters and media types, `types` of responses).
pub fn type_ref(e: &Expr, names: &[String], language: &str) -> Value {
    let member = |_: DeclId, wire: &str| wire.to_owned();
    Builder {
        names,
        member: &member,
        language,
    }
    .expr(e)
}

struct Builder<'a> {
    names: &'a [String],
    member: &'a dyn Fn(DeclId, &str) -> String,
    language: &'a str,
}

impl Builder<'_> {
    fn decl(&self, id: DeclId, kind: &DeclKind) -> Value {
        match kind {
            DeclKind::Record(r) => {
                let mut v = self.record(id, r);
                v["kind"] = json!("record");
                v
            }
            DeclKind::Enum(e) => json!({
                "kind": "enum",
                "values": e.values.iter().map(literal).collect::<Vec<_>>(),
                "open": e.open,
            }),
            DeclKind::TaggedUnion(t) => {
                let mut v = json!({
                    "kind": "taggedUnion",
                    "property": t.property,
                    "variants": t.variants.iter().map(|v| json!({ "values": v.values, "type": self.expr(&v.ty) })).collect::<Vec<_>>(),
                    "open": t.open,
                });
                if let Some(d) = &t.default {
                    v["default"] = self.expr(d);
                }
                v
            }
            DeclKind::UntaggedUnion(u) => {
                let mut v = json!({
                    "kind": "untaggedUnion",
                    "members": u.members.iter().map(|m| self.expr(m)).collect::<Vec<_>>(),
                });
                if let Some(d) = &u.discriminator {
                    v["discriminator"] = json!(d);
                }
                v
            }
            DeclKind::Alias(e) => json!({ "kind": "alias", "type": self.expr(e) }),
        }
    }

    fn record(&self, id: DeclId, r: &Record) -> Value {
        let fields: Vec<Value> = r
            .fields
            .iter()
            .map(|f| {
                json!({
                    "wire": f.wire_name,
                    "member": (self.member)(id, &f.wire_name),
                    "type": self.expr(&f.ty),
                    "presence": match f.presence {
                        Presence::Required => "required",
                        Presence::Optional => "optional",
                        Presence::Nullable => "nullable",
                        Presence::OptionalNullable => "optionalNullable",
                    },
                })
            })
            .collect();
        let mut v = json!({ "fields": fields });
        if let Some(b) = r.base {
            v["base"] = json!(self.names[b.0]);
        }
        if let Some(a) = &r.additional {
            v["additional"] = self.expr(a);
        }
        v
    }

    fn expr(&self, e: &Expr) -> Value {
        match e {
            Expr::Any => json!("any"),
            Expr::Never => json!("never"),
            Expr::Null => json!("null"),
            Expr::Bool => json!("bool"),
            Expr::Bytes => json!("bytes"),
            Expr::Scalar(s) => {
                let kind = match s.kind {
                    ScalarKind::Integer => "integer",
                    ScalarKind::Number => "number",
                    ScalarKind::String => "string",
                };
                match &s.format {
                    Some(f) => json!({ "scalar": kind, "format": f }),
                    None => json!({ "scalar": kind }),
                }
            }
            Expr::List(t) => json!({ "list": self.expr(t) }),
            Expr::Map(t) => json!({ "map": self.expr(t) }),
            Expr::Tuple {
                items,
                required,
                rest,
            } => {
                let mut v = json!({
                    "tuple": items.iter().map(|i| self.expr(i)).collect::<Vec<_>>(),
                    "required": required,
                });
                if let Some(r) = rest {
                    v["rest"] = self.expr(r);
                }
                v
            }
            Expr::Named { decl, .. } => json!({ "ref": self.names[decl.0] }),
            Expr::External { entries, schema } => match entries.get(self.language) {
                Some(ext) => json!({ "external": ext.code, "schema": self.expr(schema) }),
                None => self.expr(schema),
            },
            Expr::Nullable(t) => json!({ "nullable": self.expr(t) }),
            // TypeDef languages lower without inline records and intersections (their capabilities
            // exclude them), so these only appear when a caller lowers with other capabilities.
            Expr::InlineRecord(_) | Expr::Intersection(_) => json!("any"),
            Expr::Union(ts) => {
                json!({ "union": ts.iter().map(|t| self.expr(t)).collect::<Vec<_>>() })
            }
            Expr::Literal(l) => json!({ "literal": literal(l) }),
        }
    }
}

fn literal(l: &Literal) -> Value {
    match l {
        Literal::String(s) => json!(s),
        Literal::Number(n) => Value::Number(n.clone()),
        Literal::Boolean(b) => json!(b),
    }
}
