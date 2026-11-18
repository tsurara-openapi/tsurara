//! Naming hoisting candidates (design.md 18.4.2) for backends that turn inline objects, enums and
//! unions into named types. TypeScript writes them inline and does not use this.

use std::collections::{BTreeMap, HashSet};
use tsurara_loader::Loc;
use tsurara_naming::{Context, Names, NamingPolicy};
use tsurara_sem::Model;
use tsurara_sem::ir::{Inline, Object, Type};
use tsurara_sem::operation::Operation;

/// The candidates of the model in model order: declarations, then operations, then webhooks; outer
/// candidates before the ones nested in them. A schema converted more than once (for reading and
/// writing) is listed once.
pub fn candidates(model: &Model) -> Vec<&Inline> {
    let mut out = Vec::new();
    for d in &model.schemas {
        collect(&d.ty, &mut out);
    }
    for op in model.operations.iter().chain(&model.webhooks) {
        collect_operation(op, &mut out);
    }
    let mut seen = HashSet::new();
    out.retain(|c| seen.insert(c.loc.clone()));
    out
}

fn collect_operation<'a>(op: &'a Operation, out: &mut Vec<&'a Inline>) {
    for p in &op.params {
        collect(&p.ty, out);
    }
    if let Some(b) = &op.body {
        for c in &b.contents {
            collect(&c.ty, out);
        }
    }
    for r in &op.responses {
        for c in &r.contents {
            collect(&c.ty, out);
        }
        for h in &r.headers {
            collect(&h.ty, out);
        }
    }
    for cb in &op.callbacks {
        for o in &cb.operations {
            collect_operation(o, out);
        }
    }
}

fn collect<'a>(ty: &'a Type, out: &mut Vec<&'a Inline>) {
    match ty {
        Type::Inline(i) => {
            out.push(i);
            collect(&i.ty, out);
        }
        Type::Array(t) => collect(t, out),
        Type::Tuple(t) => {
            t.items.iter().for_each(|i| collect(i, out));
            if let Some(r) = &t.rest {
                collect(r, out);
            }
        }
        Type::Union(ts) | Type::Intersection(ts) => ts.iter().for_each(|t| collect(t, out)),
        Type::Choice(c) => c.members.iter().for_each(|t| collect(t, out)),
        Type::Object(Object { properties, index }) => {
            properties.iter().for_each(|p| collect(&p.ty, out));
            if let Some(i) = index {
                collect(i, out);
            }
        }
        Type::Mapped(m) => collect(&m.schema, out),
        _ => {}
    }
}

/// Claims a name for every candidate, after the backend has claimed its declarations and operations,
/// so that adding an inline schema never renames a declaration. `overrides` (by JSON Pointer) are used
/// as they are.
pub fn claim_candidates(
    model: &Model,
    names: &mut Names,
    policy: &dyn NamingPolicy,
    scope: &str,
    overrides: &BTreeMap<String, String>,
) -> Vec<(Loc, String)> {
    candidates(model)
        .into_iter()
        .map(|c| {
            let pointer = format!("#{}", c.loc.pointer);
            let name = match overrides.get(&pointer) {
                Some(name) => {
                    names.reserve(scope, name, &pointer);
                    name.clone()
                }
                None => {
                    let ident = policy.convert(Context::Type, &c.preferred_name, false);
                    names
                        .claim(policy, Context::Type, scope, &ident, &pointer)
                        .name
                }
            };
            (c.loc.clone(), name)
        })
        .collect()
}
