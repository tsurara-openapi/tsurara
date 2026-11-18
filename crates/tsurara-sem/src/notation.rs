//! A compact, language-neutral notation for model types, used by semantic case tests
//! (design.md 18.12). It shows what the model keeps, not what any language prints.

use crate::ir::{Literal, Type};

pub fn notation(ty: &Type) -> String {
    match ty {
        Type::Unknown => "any".into(),
        Type::Never => "never".into(),
        Type::Null => "null".into(),
        Type::Undefined => "undefined".into(),
        Type::String => "string".into(),
        Type::Number => "number".into(),
        Type::Boolean => "boolean".into(),
        Type::Scalar(s) => {
            let kind = match s.kind {
                crate::ir::ScalarKind::Integer => "integer",
                crate::ir::ScalarKind::Number => "number",
                crate::ir::ScalarKind::String => "string",
            };
            match &s.format {
                Some(f) => format!("{kind}({f})"),
                None => kind.into(),
            }
        }
        Type::Literal(l) => literal(l),
        Type::Enum(e) => {
            let mut values: Vec<String> = e.values.iter().map(literal).collect();
            if e.nullable {
                values.push("null".into());
            }
            format!("enum({})", values.join(", "))
        }
        Type::Ref(name) => name.clone(),
        Type::Named(n) => n.hint.clone(),
        Type::Array(t) => format!("array({})", notation(t)),
        Type::Tuple(t) => {
            let mut items: Vec<String> = t.items.iter().map(notation).collect();
            for item in items.iter_mut().skip(t.required) {
                item.push('?');
            }
            if let Some(r) = &t.rest {
                items.push(format!("...{}", notation(r)));
            }
            format!("tuple({})", items.join(", "))
        }
        Type::Union(ts) => format!("union({})", list(ts)),
        Type::Intersection(ts) => format!("allOf({})", list(ts)),
        Type::Choice(c) => {
            let kind = if c.exclusive { "oneOf" } else { "anyOf" };
            match &c.discriminator {
                None => format!("{kind}({})", list(&c.members)),
                Some(d) => {
                    let members: Vec<String> = c
                        .members
                        .iter()
                        .zip(&d.values)
                        .map(|(m, v)| {
                            if v.is_empty() {
                                notation(m)
                            } else {
                                format!("{} = {}", v.join(" | "), notation(m))
                            }
                        })
                        .collect();
                    let default = d
                        .default
                        .as_deref()
                        .map(|t| format!(", default = {}", notation(t)))
                        .unwrap_or_default();
                    format!("{kind}[{}]({}{default})", d.property, members.join(", "))
                }
            }
        }
        Type::Tag(t) => format!("tag({} = {})", t.property, t.values.join(" | ")),
        Type::Object(o) => {
            let mut parts: Vec<String> = o
                .properties
                .iter()
                .map(|p| {
                    let q = if p.optional { "?" } else { "" };
                    format!("{}{q}: {}", p.name, notation(&p.ty))
                })
                .collect();
            if let Some(i) = &o.index {
                parts.push(format!("[string]: {}", notation(i)));
            }
            if parts.is_empty() {
                "{}".into()
            } else {
                format!("{{ {} }}", parts.join(", "))
            }
        }
        Type::External(e) => format!("external({})", e.code),
        Type::Mapped(m) => {
            let entries: Vec<String> = m.entries.keys().cloned().collect();
            format!("mapped[{}]({})", entries.join(", "), notation(&m.schema))
        }
        Type::Inline(i) => format!("{}:{}", i.preferred_name, notation(&i.ty)),
        Type::Binary => "binary".into(),
        Type::Builtin(n) => n.to_string(),
    }
}

fn list(ts: &[Type]) -> String {
    ts.iter().map(notation).collect::<Vec<_>>().join(", ")
}

fn literal(l: &Literal) -> String {
    match l {
        Literal::String(s) => serde_json::to_string(s).expect("strings serialize"),
        Literal::Number(n) => n.to_string(),
        Literal::Boolean(b) => b.to_string(),
    }
}
