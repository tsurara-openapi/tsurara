//! Prints TypeScript code from the IR.
//!
//! The same IR always yields the same string. Indentation is fixed to two spaces and newlines to `\n`.

use crate::ir::{Doc, Literal, Object, Type};
use std::fmt::Write;
use tsurara_pretty::{Doc as Pretty, Fit, render};

/// Maximum length of an object written inline.
const INLINE_OBJECT_WIDTH: usize = 60;
/// Maximum length of an object on each line of a top-level union.
const UNION_LINE_OBJECT_WIDTH: usize = 110;
/// Maximum length for writing a top-level union on one line.
const INLINE_UNION_WIDTH: usize = 80;

const PREC_UNION: u8 = 1;
const PREC_INTERSECTION: u8 = 2;
const PREC_POSTFIX: u8 = 3;

/// Prints a type as a single expression (indent 0).
pub fn print_type(ty: &Type) -> String {
    render(&type_doc(ty, 0, INLINE_OBJECT_WIDTH), usize::MAX)
}

/// Prints `export type Name = ...;`.
pub fn print_type_alias(name: &str, ty: &Type, doc: Option<&Doc>) -> String {
    let mut s = String::new();
    if let Some(doc) = doc {
        s.push_str(&print_doc(doc, 0));
    }
    write!(s, "export type {name} =").unwrap();
    let body = match ty {
        Type::Union(members) => {
            let has_object = members.iter().any(|m| matches!(m, Type::Object(_)));
            // One line when it is short enough next to the name, otherwise one member per line.
            let fit = if has_object {
                Fit::Never
            } else {
                Fit::Within((INLINE_UNION_WIDTH - 15).saturating_sub(name.len()))
            };
            let member = |m: &Type| type_doc(m, PREC_UNION, UNION_LINE_OBJECT_WIDTH);
            let flat = Pretty::concat([Pretty::text(" "), join(members.iter().map(member), " | ")]);
            Pretty::group(fit, Pretty::flat_or(flat, union_lines(members)))
        }
        Type::Object(o) if !o.properties.is_empty() => {
            Pretty::concat([Pretty::text(" "), object_doc(o, true, 0)])
        }
        // `{ ...common } & (| A | B)`, such as a `Data` type with one member per body media type.
        Type::Intersection(members) if matches!(members.last(), Some(Type::Union(_))) => {
            let (Some(Type::Union(union)), rest) = (members.last(), &members[..members.len() - 1])
            else {
                unreachable!("checked by the guard");
            };
            let mut parts = vec![Pretty::text(" ")];
            for m in rest {
                parts.push(match m {
                    Type::Object(o) if !o.properties.is_empty() => object_doc(o, true, 0),
                    _ => type_doc(m, PREC_INTERSECTION + 1, INLINE_OBJECT_WIDTH),
                });
                parts.push(Pretty::text(" & "));
            }
            parts.push(Pretty::text("("));
            parts.push(union_lines(union));
            parts.push(Pretty::HardLine);
            parts.push(Pretty::text(")"));
            Pretty::concat(parts)
        }
        _ => Pretty::concat([Pretty::text(" "), type_doc(ty, 0, INLINE_OBJECT_WIDTH)]),
    };
    s.push_str(&render(&body, usize::MAX));
    s.push_str(";\n");
    s
}

/// The members of a top-level union, one per line: `\n  | A`.
fn union_lines(members: &[Type]) -> Pretty {
    Pretty::nest(
        2,
        Pretty::concat(members.iter().flat_map(|m| {
            [
                Pretty::HardLine,
                Pretty::text("| "),
                type_doc(m, PREC_UNION, UNION_LINE_OBJECT_WIDTH),
            ]
        })),
    )
}

fn join(docs: impl Iterator<Item = Pretty>, sep: &'static str) -> Pretty {
    let mut out = Vec::new();
    for (i, d) in docs.enumerate() {
        if i > 0 {
            out.push(Pretty::text(sep));
        }
        out.push(d);
    }
    Pretty::concat(out)
}

/// A type as a document. `width` is the largest flat width of objects inside it, other than
/// property types, which always use `INLINE_OBJECT_WIDTH`.
fn type_doc(ty: &Type, prec: u8, width: usize) -> Pretty {
    match ty {
        Type::Unknown => "unknown".into(),
        Type::Never => "never".into(),
        Type::Null => "null".into(),
        Type::Undefined => "undefined".into(),
        Type::String => "string".into(),
        Type::Number => "number".into(),
        Type::Boolean => "boolean".into(),
        Type::Scalar(sc) => type_doc(&sc.base(), prec, width),
        Type::Enum(e) => type_doc(&e.literal_union(), prec, width),
        Type::Binary => type_doc(&binary_union(), prec, width),
        // Printed as TypeScript prints it; other languages do not use this printer.
        Type::Mapped(m) => match m.entries.get("typescript") {
            Some(e) => type_doc(&Type::External(e.clone()), prec, width),
            None => type_doc(&m.schema, prec, width),
        },
        Type::Choice(c) => type_doc(&c.union(), prec, width),
        Type::Tag(t) => type_doc(&t.object(), prec, width),
        Type::Literal(l) => literal_text(l, prec).into(),
        Type::Ref(name) => name.clone().into(),
        Type::Named(n) => n.hint.clone().into(),
        Type::Inline(i) => type_doc(&i.ty, prec, width),
        Type::Builtin(name) => name.to_string().into(),
        Type::External(e) => {
            if prec > 0 && needs_parens(&e.code) {
                format!("({})", e.code).into()
            } else {
                e.code.clone().into()
            }
        }
        Type::Array(inner) => Pretty::concat([type_doc(inner, PREC_POSTFIX, width), "[]".into()]),
        Type::Tuple(t) => {
            let mut parts: Vec<Pretty> = t
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    if i < t.required {
                        type_doc(item, 0, width)
                    } else {
                        Pretty::concat([type_doc(item, PREC_POSTFIX, width), "?".into()])
                    }
                })
                .collect();
            if let Some(rest) = &t.rest {
                parts.push(Pretty::concat([
                    "...".into(),
                    type_doc(rest, PREC_POSTFIX, width),
                    "[]".into(),
                ]));
            }
            Pretty::concat(["[".into(), join(parts.into_iter(), ", "), "]".into()])
        }
        Type::Union(members) => parenthesized(
            prec > PREC_UNION,
            join(
                members.iter().map(|m| type_doc(m, PREC_UNION + 1, width)),
                " | ",
            ),
        ),
        Type::Intersection(members) => parenthesized(
            prec > PREC_INTERSECTION,
            join(
                members
                    .iter()
                    .map(|m| type_doc(m, PREC_INTERSECTION + 1, width)),
                " & ",
            ),
        ),
        Type::Object(o) => object_doc(o, false, width),
    }
}

fn parenthesized(paren: bool, doc: Pretty) -> Pretty {
    if paren {
        Pretty::concat(["(".into(), doc, ")".into()])
    } else {
        doc
    }
}

/// Roughly checks whether an external type's code contains operators. If so, it is parenthesized.
fn needs_parens(code: &str) -> bool {
    let mut depth = 0i32;
    for c in code.chars() {
        match c {
            '(' | '<' | '{' | '[' => depth += 1,
            ')' | '>' | '}' | ']' => depth -= 1,
            '|' | '&' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

fn literal_text(l: &Literal, prec: u8) -> String {
    match l {
        Literal::String(v) => string_literal(v),
        Literal::Number(n) => {
            let text = n.to_string();
            if text.starts_with('-') && prec >= PREC_POSTFIX {
                format!("({text})")
            } else {
                text
            }
        }
        Literal::Boolean(b) => if *b { "true" } else { "false" }.to_owned(),
    }
}

/// An object: `{ a: T; b?: U }` when it has no docs and its flat width is within `width`,
/// otherwise one member per line. `force_multiline` breaks it whenever it has properties.
fn object_doc(o: &Object, force_multiline: bool, width: usize) -> Pretty {
    if o.properties.is_empty() && o.index.is_none() {
        // The same type as `{}`, which lint rules reject as "any non-nullish value".
        return "Record<never, never>".into();
    }
    let mut members: Vec<(Option<&Doc>, Pretty)> = Vec::new();
    for p in &o.properties {
        let mut head = String::new();
        if p.readonly {
            head.push_str("readonly ");
        }
        head.push_str(&property_key(&p.name));
        if p.optional {
            head.push('?');
        }
        head.push_str(": ");
        members.push((
            p.doc.as_ref(),
            Pretty::concat([head.into(), type_doc(&p.ty, 0, INLINE_OBJECT_WIDTH)]),
        ));
    }
    if let Some(index) = &o.index {
        members.push((
            None,
            Pretty::concat([
                "[key: string]: ".into(),
                type_doc(index, 0, INLINE_OBJECT_WIDTH),
            ]),
        ));
    }
    let mut inner = Vec::new();
    for (i, (doc, member)) in members.into_iter().enumerate() {
        inner.push(if i == 0 {
            Pretty::line(" ")
        } else {
            Pretty::flat_or("; ".into(), Pretty::concat([";".into(), Pretty::HardLine]))
        });
        if let Some(doc) = doc {
            // JSDoc lines only exist in the broken layout; a doc breaks the object.
            let lines: Vec<Pretty> = print_doc(doc, 0)
                .lines()
                .flat_map(|l| [Pretty::text(l.to_owned()), Pretty::HardLine])
                .collect();
            inner.push(Pretty::concat(lines));
        }
        inner.push(member);
    }
    inner.push(Pretty::flat_or(Pretty::Nil, ";".into()));
    let fit = if force_multiline && !o.properties.is_empty() {
        Fit::Never
    } else {
        Fit::Within(width)
    };
    Pretty::group(
        fit,
        Pretty::concat([
            "{".into(),
            Pretty::nest(2, Pretty::concat(inner)),
            Pretty::line(" "),
            "}".into(),
        ]),
    )
}

/// Prints a JSDoc. `*/` is escaped as `*\/`.
pub fn print_doc(doc: &Doc, indent: usize) -> String {
    let pad = " ".repeat(indent);
    let mut lines: Vec<String> = Vec::new();
    if let Some(d) = &doc.description {
        let d = d.trim_end();
        if !d.trim().is_empty() {
            lines.extend(
                crate::text::lines(d)
                    .into_iter()
                    .map(|l| l.trim_end().to_owned()),
            );
        }
    }
    for (tag, value) in &doc.tags {
        let value = Doc::tag_text(tag, value);
        let value_lines = crate::text::lines(&value);
        let mut value_lines = value_lines.iter().map(String::as_str);
        let first = value_lines.next().unwrap_or("");
        if first.is_empty() {
            lines.push(format!("@{tag}"));
        } else {
            lines.push(format!("@{tag} {first}"));
        }
        lines.extend(value_lines.map(|l| l.trim_end().to_owned()));
    }
    if lines.is_empty() {
        return String::new();
    }
    let lines: Vec<String> = lines.iter().map(|l| escape_doc(l)).collect();
    if lines.len() == 1 {
        return format!("{pad}/** {} */\n", lines[0]);
    }
    let mut s = format!("{pad}/**\n");
    for l in lines {
        if l.is_empty() {
            writeln!(s, "{pad} *").unwrap();
        } else {
            writeln!(s, "{pad} * {l}").unwrap();
        }
    }
    writeln!(s, "{pad} */").unwrap();
    s
}

fn escape_doc(line: &str) -> String {
    line.replace("*/", "*\\/")
}

/// Text of the definition for a place inside a JSDoc line: on one line, with `*/` escaped.
pub fn doc_inline(text: &str) -> String {
    escape_doc(&crate::text::single_line(text))
}

/// Property key. Names usable as identifiers and canonical non-negative integers are written as is,
/// and everything else is quoted.
pub fn property_key(name: &str) -> String {
    if is_identifier_name(name) || is_canonical_index(name) {
        name.to_owned()
    } else {
        string_literal(name)
    }
}

fn is_canonical_index(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_digit())
        && (name == "0" || !name.starts_with('0'))
        && name.len() <= 15
}

/// ECMAScript IdentifierName (ASCII only). Reserved words are allowed as property names.
pub fn is_identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Single-quoted string literal.
pub fn string_literal(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                write!(out, "\\x{:02x}", c as u32).unwrap();
            }
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// `Blob | File`, the TypeScript type of a binary part value.
pub fn binary_union() -> Type {
    Type::Union(vec![Type::builtin("Blob"), Type::builtin("File")])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::{External, Import, Property, Tuple};
    use serde_json::Number;

    fn obj(props: Vec<Property>) -> Type {
        Type::Object(Object {
            properties: props,
            index: None,
        })
    }

    #[test]
    fn precedence() {
        let u = Type::Union(vec![Type::String, Type::Number]);
        assert_eq!(print_type(&u.clone().array()), "(string | number)[]");
        assert_eq!(
            print_type(&Type::Intersection(vec![Type::Ref("A".into()), u.clone()])),
            "A & (string | number)"
        );
        assert_eq!(
            print_type(&Type::Union(vec![
                Type::Intersection(vec![Type::Ref("A".into()), Type::Ref("B".into())]),
                Type::Null
            ])),
            "A & B | null"
        );
        assert_eq!(print_type(&Type::String.array().array()), "string[][]");
        assert_eq!(
            print_type(&Type::Literal(Literal::Number(Number::from(-1))).array()),
            "(-1)[]"
        );
        assert_eq!(
            print_type(
                &Type::External(External {
                    code: "string & { __brand: 'Uuid' }".into(),
                    import: None
                })
                .array()
            ),
            "(string & { __brand: 'Uuid' })[]"
        );
    }

    #[test]
    fn tuples() {
        let t = Type::Tuple(Tuple {
            items: vec![Type::String, Type::Number],
            required: 2,
            rest: Some(Box::new(Type::Union(vec![Type::Boolean, Type::Null]))),
        });
        assert_eq!(print_type(&t), "[string, number, ...(boolean | null)[]]");
        let t = Type::Tuple(Tuple {
            items: vec![],
            required: 0,
            rest: None,
        });
        assert_eq!(print_type(&t), "[]");
    }

    #[test]
    fn optional_tuple_elements() {
        let t = Type::Tuple(Tuple {
            items: vec![
                Type::String,
                Type::Union(vec![Type::Number, Type::Null]),
                Type::Boolean.array(),
            ],
            required: 1,
            rest: Some(Box::new(Type::Unknown)),
        });
        assert_eq!(
            print_type(&t),
            "[string, (number | null)?, boolean[]?, ...unknown[]]"
        );
    }

    #[test]
    fn property_keys() {
        assert_eq!(property_key("name"), "name");
        assert_eq!(property_key("@type"), "'@type'");
        assert_eq!(property_key("content-type"), "'content-type'");
        assert_eq!(property_key("class"), "class");
        assert_eq!(property_key("200"), "200");
        assert_eq!(property_key("007"), "'007'");
        assert_eq!(property_key("2XX"), "'2XX'");
        assert_eq!(property_key("it's"), "'it\\'s'");
        assert_eq!(property_key("日本"), "'日本'");
        assert_eq!(property_key(""), "''");
    }

    #[test]
    fn string_literals() {
        assert_eq!(string_literal("a'b\\c\nd\u{1}"), "'a\\'b\\\\c\\nd\\x01'");
    }

    #[test]
    fn docs() {
        let doc = Doc {
            description: Some("Line 1\n\nEnds */ here".into()),
            tags: vec![
                ("deprecated".into(), "".into()),
                ("default".into(), 1.into()),
            ],
        };
        assert_eq!(
            print_doc(&doc, 2),
            "  /**\n   * Line 1\n   *\n   * Ends *\\/ here\n   * @deprecated\n   * @default 1\n   */\n"
        );
        let doc = Doc {
            description: Some("Short".into()),
            tags: vec![],
        };
        assert_eq!(print_doc(&doc, 0), "/** Short */\n");
    }

    #[test]
    fn aliases() {
        let pet = obj(vec![
            Property::new("id", Type::Number),
            Property::new("name", Type::String)
                .optional(true)
                .with_doc(Some(Doc {
                    description: Some("The name".into()),
                    tags: vec![],
                })),
            Property::new("@type", Type::string_literal("Pet")),
            Property::new("tags", obj(vec![Property::new("a", Type::String)]).array()),
        ]);
        assert_eq!(
            print_type_alias("Pet", &pet, None),
            "export type Pet = {\n  id: number;\n  /** The name */\n  name?: string;\n  '@type': 'Pet';\n  tags: { a: string }[];\n};\n"
        );
        let map = Type::Object(Object {
            properties: vec![],
            index: Some(Box::new(Type::Unknown)),
        });
        assert_eq!(
            print_type_alias("Map", &map, None),
            "export type Map = { [key: string]: unknown };\n"
        );
        let small = Type::Union(vec![Type::string_literal("a"), Type::string_literal("b")]);
        assert_eq!(
            print_type_alias("S", &small, None),
            "export type S = 'a' | 'b';\n"
        );
        let big = Type::Union(vec![
            obj(vec![
                Property::new("ok", Type::Literal(Literal::Boolean(true))),
                Property::new("data", Type::Ref("Pet".into())),
            ]),
            Type::Ref("TransportFailure".into()),
        ]);
        assert_eq!(
            print_type_alias("GetPetResult", &big, None),
            "export type GetPetResult =\n  | { ok: true; data: Pet }\n  | TransportFailure;\n"
        );
        let nested = obj(vec![Property::new(
            "inner",
            obj(vec![
                Property::new("aaaaaaaaaaaaaaaa", Type::String),
                Property::new("bbbbbbbbbbbbbbbbbbbb", Type::String),
                Property::new("cccccccccccc", Type::String),
            ]),
        )]);
        assert_eq!(
            print_type_alias("N", &nested, None),
            "export type N = {\n  inner: {\n    aaaaaaaaaaaaaaaa: string;\n    bbbbbbbbbbbbbbbbbbbb: string;\n    cccccccccccc: string;\n  };\n};\n"
        );
    }

    #[test]
    fn external_types() {
        let t = Type::External(External {
            code: "Uuid".into(),
            import: Some(Import {
                from: "./brand.js".into(),
                name: "Uuid".into(),
            }),
        });
        assert_eq!(print_type(&t), "Uuid");
    }

    #[test]
    fn snapshot_of_a_complex_alias() {
        let doc = Doc {
            description: Some("A pet in the store.".into()),
            tags: vec![("see".into(), "https://example.com".into())],
        };
        let ty = Type::Intersection(vec![
            Type::Ref("Base".into()),
            obj(vec![
                Property::new("id", Type::Number).with_doc(Some(Doc {
                    description: Some("Unique id".into()),
                    tags: vec![("format".into(), "int64".into())],
                })),
                Property::new(
                    "status",
                    Type::Union(vec![
                        Type::string_literal("available"),
                        Type::string_literal("sold"),
                        Type::Null,
                    ]),
                )
                .optional(true),
                Property::new(
                    "position",
                    Type::Tuple(Tuple {
                        items: vec![Type::Number, Type::Number],
                        required: 2,
                        rest: None,
                    }),
                ),
                Property::new(
                    "extra",
                    Type::Object(Object {
                        properties: vec![Property::new("x-y", Type::String)],
                        index: Some(Box::new(Type::Union(vec![Type::String, Type::Number]))),
                    }),
                ),
            ]),
        ]);
        insta::assert_snapshot!(print_type_alias("Pet", &ty, Some(&doc)));
    }
}
