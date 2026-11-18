//! `enum` is an explicit node in the model (design.md 18.4.1).

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::ir::{Enum, Literal, Type};
use tsurara_sem::{Options, build_model};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Status: { type: string, enum: [available, sold] }
    Level: { type: integer, enum: [1, 2] }
    Mixed: { enum: [1, x, true, null] }
    Cat: { const: cat }
"##;

fn decl(name: &str) -> Type {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    model
        .schemas
        .into_iter()
        .find(|d| d.preferred_name == name && !d.writable)
        .unwrap()
        .ty
}

fn s(v: &str) -> Literal {
    Literal::String(v.into())
}

fn n(v: i64) -> Literal {
    Literal::Number(v.into())
}

#[test]
fn string_integer_and_mixed_enums_are_enum_nodes() {
    assert_eq!(
        decl("Status"),
        Type::Enum(Enum {
            values: vec![s("available"), s("sold")],
            nullable: false
        })
    );
    assert_eq!(
        decl("Level"),
        Type::Enum(Enum {
            values: vec![n(1), n(2)],
            nullable: false
        })
    );
    assert_eq!(
        decl("Mixed"),
        Type::Enum(Enum {
            values: vec![n(1), s("x"), Literal::Boolean(true)],
            nullable: true
        })
    );
}

#[test]
fn const_is_a_literal_not_an_enum() {
    assert_eq!(decl("Cat"), Type::Literal(s("cat")));
}
