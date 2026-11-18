//! Checks that a lowered module uses only what the capabilities allow (design.md 18.5). A violation is
//! a bug in a pass.

use crate::caps::{Capabilities, Enums, Inheritance};
use crate::tir::*;

/// Every construct the language cannot express, as messages naming the declaration.
pub fn validate(module: &Module, caps: &Capabilities) -> Vec<String> {
    let mut out = Vec::new();
    for (i, d) in module.decls.iter().enumerate() {
        let mut err = |m: String| out.push(format!("{} (#{i}): {m}", d.preferred_name));
        match &d.kind {
            DeclKind::Record(r) => record(r, caps, module, &mut err),
            DeclKind::Enum(e) => {
                let native = e.repr == EnumRepr::Native;
                let integers = e.values.iter().all(
                    |v| matches!(v, tsurara_sem::ir::Literal::Number(n) if n.is_i64() || n.is_u64()),
                );
                if native
                    && (caps.enums == Enums::None
                        || (caps.enums == Enums::IntegerOnly && !integers))
                {
                    err("a native enum the language does not have".into());
                }
            }
            DeclKind::TaggedUnion(t) => {
                if !caps.tagged_unions {
                    err("a tagged union".into());
                }
                for v in &t.variants {
                    expr(&v.ty, caps, module, &mut err);
                }
                if let Some(d) = &t.default {
                    expr(d, caps, module, &mut err);
                }
            }
            DeclKind::UntaggedUnion(u) => u
                .members
                .iter()
                .for_each(|m| expr(m, caps, module, &mut err)),
            DeclKind::Alias(e) => expr(e, caps, module, &mut err),
        }
    }
    out
}

fn record(r: &Record, caps: &Capabilities, module: &Module, err: &mut impl FnMut(String)) {
    if r.base.is_some() && caps.inheritance == Inheritance::None {
        err("a base type without inheritance".into());
    }
    for f in &r.fields {
        expr(&f.ty, caps, module, err);
        if f.boxed && caps.recursion_by_value {
            // Harmless, but no pass should do it.
            err(format!(
                "`{}` is boxed although recursion by value is allowed",
                f.wire_name
            ));
        }
    }
    if let Some(a) = &r.additional {
        expr(a, caps, module, err);
    }
}

fn expr(e: &Expr, caps: &Capabilities, module: &Module, err: &mut impl FnMut(String)) {
    match e {
        Expr::List(t) | Expr::Map(t) => expr(t, caps, module, err),
        Expr::Tuple { items, rest, .. } => {
            if !caps.tuples {
                err("a tuple".into());
            }
            items.iter().for_each(|i| expr(i, caps, module, err));
            if let Some(r) = rest {
                expr(r, caps, module, err);
            }
        }
        Expr::Named { decl, .. } => {
            if decl.0 >= module.decls.len() {
                err(format!("a reference to a missing declaration #{}", decl.0));
            }
        }
        Expr::External { schema, .. } => expr(schema, caps, module, err),
        Expr::Nullable(t) => {
            if !caps.nullable_primitives && matches!(**t, Expr::Scalar(_) | Expr::Bool) {
                err("a nullable primitive".into());
            }
            expr(t, caps, module, err);
        }
        Expr::InlineRecord(r) => {
            if !caps.inline_records {
                err("an inline record".into());
            }
            record(r, caps, module, err);
        }
        Expr::Union(ts) => {
            if !caps.structural_unions {
                err("a structural union".into());
            }
            ts.iter().for_each(|t| expr(t, caps, module, err));
        }
        Expr::Intersection(ts) => {
            if !caps.intersections {
                err("an intersection".into());
            }
            ts.iter().for_each(|t| expr(t, caps, module, err));
        }
        Expr::Literal(_) => {
            if !caps.literal_types {
                err("a literal type".into());
            }
        }
        Expr::Any | Expr::Never | Expr::Null | Expr::Bool | Expr::Scalar(_) | Expr::Bytes => {}
    }
}
