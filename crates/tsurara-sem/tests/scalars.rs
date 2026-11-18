//! The model keeps integer versus number and `format` (design.md 18.4.1).

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::ir::{Scalar, ScalarKind, Type};
use tsurara_sem::{Options, build_model};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Id: { type: integer, format: int64 }
    Count: { type: integer }
    Price: { type: number, format: double }
    When: { type: string, format: date-time }
    Name: { type: string }
    Either: { type: [integer, string] }
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

fn scalar(kind: ScalarKind, format: Option<&str>) -> Type {
    Type::Scalar(Scalar {
        kind,
        format: format.map(str::to_owned),
    })
}

#[test]
fn integer_and_number_are_distinct_and_keep_their_format() {
    assert_eq!(decl("Id"), scalar(ScalarKind::Integer, Some("int64")));
    assert_eq!(decl("Count"), scalar(ScalarKind::Integer, None));
    assert_eq!(decl("Price"), scalar(ScalarKind::Number, Some("double")));
}

#[test]
fn strings_keep_their_format() {
    assert_eq!(decl("When"), scalar(ScalarKind::String, Some("date-time")));
    assert_eq!(decl("Name"), scalar(ScalarKind::String, None));
}

#[test]
fn type_arrays_keep_each_scalar() {
    assert_eq!(
        decl("Either"),
        Type::Union(vec![
            scalar(ScalarKind::Integer, None),
            scalar(ScalarKind::String, None)
        ])
    );
}
