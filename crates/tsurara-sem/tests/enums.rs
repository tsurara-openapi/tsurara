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
    Color: { type: string, enum: [r, g, r, null], x-enum-varnames: [Red, Green, Again, None] }
    Size: { type: integer, enum: [1, 2], x-enumNames: [Small, Large] }
    Short: { type: string, enum: [a, b], x-enum-varnames: [A] }
    Twice: { type: string, enum: [a, b], x-enum-varnames: [Same, Same] }
"##;

fn model() -> tsurara_sem::Model {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    build_model(&set, &Options::default()).unwrap()
}

fn decl(name: &str) -> Type {
    model()
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
            nullable: false,
            names: vec![],
        })
    );
    assert_eq!(
        decl("Level"),
        Type::Enum(Enum {
            values: vec![n(1), n(2)],
            nullable: false,
            names: vec![],
        })
    );
    assert_eq!(
        decl("Mixed"),
        Type::Enum(Enum {
            values: vec![n(1), s("x"), Literal::Boolean(true)],
            nullable: true,
            names: vec![],
        })
    );
}

#[test]
fn const_is_a_literal_not_an_enum() {
    assert_eq!(decl("Cat"), Type::Literal(s("cat")));
}

#[test]
fn member_names_come_from_x_enum_varnames_or_x_enum_names() {
    // The names of duplicated values and of `null` are dropped with them.
    assert_eq!(
        decl("Color"),
        Type::Enum(Enum {
            values: vec![s("r"), s("g")],
            nullable: true,
            names: vec!["Red".into(), "Green".into()],
        })
    );
    assert_eq!(
        decl("Size"),
        Type::Enum(Enum {
            values: vec![n(1), n(2)],
            nullable: false,
            names: vec!["Small".into(), "Large".into()],
        })
    );
}

#[test]
fn member_names_that_do_not_fit_the_values_are_ignored_with_a_warning() {
    let model = model();
    for name in ["Short", "Twice"] {
        let ty = &model
            .schemas
            .iter()
            .find(|d| d.preferred_name == name)
            .unwrap()
            .ty;
        assert!(
            matches!(ty, Type::Enum(e) if e.names.is_empty()),
            "{name}: {ty:?}"
        );
        let pointer = format!("/components/schemas/{name}/x-enum-varnames");
        assert!(
            model
                .warnings
                .iter()
                .any(|w| w.kind == tsurara_sem::WarningKind::InvalidDefinition
                    && w.loc.pointer == pointer),
            "{name}: {:?}",
            model.warnings
        );
    }
}
