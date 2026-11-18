//! Discriminators, `oneOf`/`anyOf` and `allOf` keep their structure in the model (design.md 18.4.1).

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::ir::{Choice, Discriminator, Scalar, ScalarKind, Tag, Type};
use tsurara_sem::{Options, build_model};

fn model(schemas: &str) -> tsurara_sem::Model {
    let doc = format!(
        "openapi: 3.2.0\ninfo: {{ title: t, version: \"1\" }}\npaths: {{}}\ncomponents:\n  schemas:\n{schemas}"
    );
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    build_model(&set, &Options::default()).unwrap()
}

/// The declaration's type, with references shown by their preferred names.
fn decl(m: &tsurara_sem::Model, name: &str) -> Type {
    let d = m
        .schemas
        .iter()
        .find(|d| d.preferred_name == name && !d.writable)
        .unwrap();
    d.ty.clone().map(&mut |t| match t {
        Type::Named(n) => Type::Ref(n.hint),
        Type::Inline(i) => i.ty,
        Type::Choice(mut c) => {
            if let Some(d) = &mut c.discriminator {
                if let Some(Type::Named(n)) = d.default.as_deref() {
                    d.default = Some(Box::new(Type::Ref(n.hint.clone())));
                }
            }
            Type::Choice(c)
        }
        other => other,
    })
}

fn string() -> Type {
    Type::Scalar(Scalar::new(ScalarKind::String, None))
}

fn number() -> Type {
    Type::Scalar(Scalar::new(ScalarKind::Number, None))
}

const PETS: &str = r##"
    Pet:
      oneOf: [{ $ref: "#/components/schemas/Cat" }, { $ref: "#/components/schemas/Dog" }, { $ref: "#/components/schemas/Bird" }]
      discriminator:
        propertyName: petType
        mapping: { kitty: "#/components/schemas/Cat", cat: "#/components/schemas/Cat" }
        defaultMapping: "#/components/schemas/Other"
    Cat: { type: object, properties: { meow: { type: string } } }
    Dog: { type: object, properties: { bark: { type: string } } }
    Bird: { type: object }
    Other: { type: object }
"##;

#[test]
fn discriminators_keep_the_property_and_the_values_per_member() {
    let m = model(PETS);
    let Type::Choice(Choice {
        members,
        exclusive,
        discriminator: Some(d),
    }) = decl(&m, "Pet")
    else {
        panic!("{:?}", decl(&m, "Pet"));
    };
    assert!(exclusive);
    assert_eq!(
        members,
        [
            Type::Ref("Cat".into()),
            Type::Ref("Dog".into()),
            Type::Ref("Bird".into())
        ]
    );
    assert_eq!(
        d,
        Discriminator {
            property: "petType".into(),
            // `mapping` values, then the implicit values (the schema name).
            values: vec![
                vec!["kitty".into(), "cat".into()],
                vec!["Dog".into()],
                vec!["Bird".into()]
            ],
            default: Some(Box::new(Type::Ref("Other".into()))),
        }
    );
}

#[test]
fn inherited_discriminators_are_tags_in_the_child() {
    let m = model(
        r##"
    Pet:
      type: object
      required: [petType]
      properties: { petType: { type: string } }
      discriminator: { propertyName: petType }
    Cat:
      allOf: [{ $ref: "#/components/schemas/Pet" }, { type: object, properties: { meow: { type: string } } }]
"##,
    );
    let Type::Intersection(members) = decl(&m, "Cat") else {
        panic!("{:?}", decl(&m, "Cat"));
    };
    assert!(
        members.contains(&Type::Tag(Tag {
            property: "petType".into(),
            values: vec!["Cat".into()]
        })),
        "{members:?}"
    );
}

#[test]
fn the_parent_child_cycle_keeps_the_children_in_the_parent() {
    let m = model(
        r##"
    Pet:
      type: object
      required: [petType]
      properties: { petType: { type: string } }
      oneOf: [{ $ref: "#/components/schemas/Cat" }]
      discriminator: { propertyName: petType }
    Cat:
      allOf: [{ $ref: "#/components/schemas/Pet" }, { type: object, properties: { meow: { type: string } } }]
"##,
    );
    let pet = format!("{:?}", decl(&m, "Pet"));
    assert!(
        pet.contains("Choice") && pet.contains("Ref(\"Cat\")"),
        "{pet}"
    );
    let cat = format!("{:?}", decl(&m, "Cat"));
    assert!(
        !cat.contains("Ref(\"Pet\")"),
        "the child expands the parent without its oneOf: {cat}"
    );
}

#[test]
fn one_of_keeps_duplicates_and_nesting() {
    let m = model(
        r##"
    Twice: { oneOf: [{ type: string }, { type: string }] }
    Nested: { oneOf: [{ oneOf: [{ type: string }, { type: number }] }, { type: number }] }
    Any: { anyOf: [{ anyOf: [{ type: string }, { type: number }] }, { type: number }] }
"##,
    );
    assert_eq!(
        decl(&m, "Twice"),
        Type::Choice(Choice {
            members: vec![string(), string()],
            exclusive: true,
            discriminator: None
        })
    );
    assert_eq!(
        decl(&m, "Nested"),
        Type::Choice(Choice {
            members: vec![
                Type::Choice(Choice {
                    members: vec![string(), number()],
                    exclusive: true,
                    discriminator: None
                }),
                number()
            ],
            exclusive: true,
            discriminator: None
        })
    );
    let Type::Choice(any) = decl(&m, "Any") else {
        panic!()
    };
    assert!(!any.exclusive);
}

#[test]
fn all_of_keeps_members_that_are_not_objects() {
    let m = model(
        r##"
    Code: { allOf: [{ type: string }, { anyOf: [{ minLength: 1 }, { const: x }] }] }
"##,
    );
    let Type::Intersection(members) = decl(&m, "Code") else {
        panic!("{:?}", decl(&m, "Code"));
    };
    assert!(members.contains(&string()), "{members:?}");
    assert!(
        members.iter().any(|t| matches!(t, Type::Choice(_))),
        "{members:?}"
    );
}

#[test]
fn inline_discriminated_members_take_the_values_of_their_tag_property() {
    let m = model(
        r##"
    Event:
      oneOf:
        - { type: object, required: [type], properties: { type: { const: created } } }
        - { type: object, required: [type], properties: { type: { type: string, enum: [deleted, removed] } } }
        - { type: object, properties: { type: { type: string } } }
      discriminator: { propertyName: type }
"##,
    );
    let Type::Choice(Choice {
        discriminator: Some(d),
        ..
    }) = decl(&m, "Event")
    else {
        panic!("{:?}", decl(&m, "Event"));
    };
    assert_eq!(
        d.values,
        vec![
            vec!["created".to_owned()],
            vec!["deleted".into(), "removed".into()],
            vec![]
        ]
    );
}
