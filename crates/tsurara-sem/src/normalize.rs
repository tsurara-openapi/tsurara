//! IR normalization. Brings types with the same meaning into the same shape (design.md 6.3).

use crate::ir::{Literal, Object, Property, ScalarKind, Type};
use crate::print::print_type;

/// Builds a union. Flattens nesting and removes duplicates. Becomes `unknown` if it contains `unknown`;
/// `never` is removed. `null` and `undefined` go last.
pub fn union(types: Vec<Type>) -> Type {
    let mut flat = Vec::with_capacity(types.len());
    flatten_union(types, &mut flat);
    if flat.iter().any(|t| matches!(t, Type::Unknown)) {
        return Type::Unknown;
    }
    let mut out: Vec<Type> = Vec::with_capacity(flat.len());
    for t in flat {
        if matches!(t, Type::Never) || out.contains(&t) {
            continue;
        }
        out.push(t);
    }
    let has = |out: &[Type], k: &Type| out.contains(k);
    let (has_string, has_number, has_boolean) = (
        out.iter().any(is_string),
        out.iter().any(is_number),
        has(&out, &Type::Boolean),
    );
    out.retain(|t| match t {
        Type::Literal(Literal::String(_)) => !has_string,
        Type::Literal(Literal::Number(_)) => !has_number,
        Type::Literal(Literal::Boolean(_)) => !has_boolean,
        _ => true,
    });
    let true_lit = Type::Literal(Literal::Boolean(true));
    let false_lit = Type::Literal(Literal::Boolean(false));
    if let (Some(i), Some(j)) = (
        out.iter().position(|t| *t == true_lit),
        out.iter().position(|t| *t == false_lit),
    ) {
        let first = i.min(j);
        out[first] = Type::Boolean;
        out.remove(i.max(j));
    }
    let tail: Vec<Type> = [Type::Null, Type::Undefined]
        .into_iter()
        .filter(|k| out.contains(k))
        .collect();
    out.retain(|t| !matches!(t, Type::Null | Type::Undefined));
    out.extend(tail);
    match out.len() {
        0 => Type::Never,
        1 => out.pop().expect("one element"),
        _ => Type::Union(out),
    }
}

fn flatten_union(types: Vec<Type>, out: &mut Vec<Type>) {
    for t in types {
        match t {
            Type::Union(inner) => flatten_union(inner, out),
            other => out.push(other),
        }
    }
}

/// Builds an intersection. Unnamed objects are merged into one and placed last. Named types are
/// sorted by name, so reordering allOf does not change the output.
pub fn intersection(types: Vec<Type>) -> Type {
    let mut flat = Vec::with_capacity(types.len());
    flatten_intersection(types, &mut flat);
    if flat.iter().any(|t| matches!(t, Type::Never)) {
        return Type::Never;
    }
    let mut refs = Vec::new();
    let mut others = Vec::new();
    let mut objects = Vec::new();
    for t in flat {
        match t {
            Type::Unknown => {}
            Type::Ref(_) => {
                if !refs.contains(&t) {
                    refs.push(t);
                }
            }
            Type::Object(o) => objects.push(o),
            t => {
                if !others.contains(&t) {
                    others.push(t);
                }
            }
        }
    }
    refs.sort_by(|a, b| match (a, b) {
        (Type::Ref(a), Type::Ref(b)) => a.cmp(b),
        _ => std::cmp::Ordering::Equal,
    });
    absorb_primitives(&mut others);
    let mut out = refs;
    out.extend(others);
    if !objects.is_empty() {
        out.push(Type::Object(merge_objects(objects)));
    }
    match out.len() {
        0 => Type::Unknown,
        1 => out.pop().expect("one element"),
        _ => Type::Intersection(out),
    }
}

fn flatten_intersection(types: Vec<Type>, out: &mut Vec<Type>) {
    for t in types {
        match t {
            Type::Intersection(inner) => flatten_intersection(inner, out),
            other => out.push(other),
        }
    }
}

/// Turns `string & 'cat'` into `'cat'`.
fn absorb_primitives(types: &mut Vec<Type>) {
    let has_lit = |pred: fn(&Literal) -> bool, ts: &[Type]| {
        ts.iter().any(|t| matches!(t, Type::Literal(l) if pred(l)))
    };
    let s = has_lit(|l| matches!(l, Literal::String(_)), types);
    let n = has_lit(|l| matches!(l, Literal::Number(_)), types);
    let b = has_lit(|l| matches!(l, Literal::Boolean(_)), types);
    types.retain(|t| match t {
        _ if is_string(t) => !s,
        _ if is_number(t) => !n,
        Type::Boolean => !b,
        _ => true,
    });
}

/// `string`, or a string scalar with any format.
fn is_string(t: &Type) -> bool {
    matches!(t, Type::String) || matches!(t, Type::Scalar(s) if s.kind == ScalarKind::String)
}

/// `number`, or a number scalar with any format. Integer scalars do not absorb number literals, which
/// may not be integers; the TypeScript lowering normalizes them once they are `number`.
fn is_number(t: &Type) -> bool {
    matches!(t, Type::Number) || matches!(t, Type::Scalar(s) if s.kind == ScalarKind::Number)
}

/// Merges object members into one. To not depend on member order, they are sorted by their printed
/// form first. Only a property that appears with different types keeps an intersection.
pub fn merge_objects(mut objects: Vec<Object>) -> Object {
    if objects.len() == 1 {
        return objects.pop().expect("one element");
    }
    objects.sort_by_cached_key(|o| print_type(&Type::Object(o.clone())));
    let mut props: Vec<Property> = Vec::new();
    let mut index: Option<Type> = None;
    for o in objects {
        for p in o.properties {
            match props.iter_mut().find(|q| q.name == p.name) {
                Some(q) => {
                    if q.ty != p.ty {
                        q.ty = intersection(vec![q.ty.clone(), p.ty]);
                    }
                    q.optional &= p.optional;
                    q.readonly |= p.readonly;
                    if q.doc.is_none() {
                        q.doc = p.doc;
                    }
                }
                None => props.push(p),
            }
        }
        if let Some(i) = o.index {
            index = Some(match index {
                None => *i,
                Some(prev) => intersection(vec![prev, *i]),
            });
        }
    }
    Object {
        properties: props,
        index: index.map(Box::new),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lit(s: &str) -> Type {
        Type::string_literal(s)
    }

    #[test]
    fn unions() {
        assert_eq!(
            union(vec![
                Type::Null,
                Type::String,
                Type::Union(vec![Type::Number, Type::String])
            ]),
            Type::Union(vec![Type::String, Type::Number, Type::Null])
        );
        assert_eq!(union(vec![Type::String, Type::Unknown]), Type::Unknown);
        assert_eq!(union(vec![Type::Never, Type::String]), Type::String);
        assert_eq!(union(vec![]), Type::Never);
        assert_eq!(union(vec![lit("a"), Type::String]), Type::String);
        assert_eq!(
            union(vec![
                Type::Literal(Literal::Boolean(true)),
                Type::Literal(Literal::Boolean(false))
            ]),
            Type::Boolean
        );
    }

    #[test]
    fn intersections() {
        let obj = |name: &str, ty: Type| {
            Type::Object(Object {
                properties: vec![Property::new(name, ty)],
                index: None,
            })
        };
        let a = intersection(vec![
            obj("x", Type::String),
            Type::Ref("B".into()),
            obj("y", Type::Number),
            Type::Ref("A".into()),
        ]);
        let b = intersection(vec![
            Type::Ref("A".into()),
            obj("y", Type::Number),
            Type::Ref("B".into()),
            obj("x", Type::String),
        ]);
        assert_eq!(a, b);
        assert_eq!(print_type(&a), "A & B & { x: string; y: number }");
        assert_eq!(
            intersection(vec![obj("k", Type::String), obj("k", lit("cat"))]),
            obj("k", lit("cat"))
        );
        assert_eq!(
            intersection(vec![Type::Unknown, Type::String]),
            Type::String
        );
        assert_eq!(intersection(vec![Type::Never, Type::String]), Type::Never);
    }
}
