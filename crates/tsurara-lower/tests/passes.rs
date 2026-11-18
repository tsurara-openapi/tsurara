//! The passes of design.md 18.5.3, each with a small capability set. Every lowered module is also
//! validated against its capabilities.

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_lower::tir::*;
use tsurara_lower::{Capabilities, Enums, Inheritance, lower, validate};
use tsurara_sem::{Options, build_model};

fn lowered(doc: &str, caps: &Capabilities, opts: &Options) -> Module {
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, opts).unwrap();
    let (module, _) = lower(&model, caps);
    let errors = validate(&module, caps);
    assert!(errors.is_empty(), "{errors:#?}");
    module
}

fn schemas(s: &str) -> String {
    format!(
        "openapi: 3.1.0\ninfo: {{ title: t, version: \"1\" }}\npaths: {{}}\ncomponents:\n  schemas:\n{s}"
    )
}

fn decl<'a>(m: &'a Module, name: &str) -> &'a Decl {
    m.decls
        .iter()
        .find(|d| d.preferred_name == name)
        .unwrap_or_else(|| {
            panic!(
                "no {name} in {:#?}",
                m.decls
                    .iter()
                    .map(|d| &d.preferred_name)
                    .collect::<Vec<_>>()
            )
        })
}

fn record<'a>(m: &'a Module, name: &str) -> &'a Record {
    match &decl(m, name).kind {
        DeclKind::Record(r) => r,
        other => panic!("{name}: {other:?}"),
    }
}

fn field<'a>(r: &'a Record, name: &str) -> &'a Field {
    r.fields.iter().find(|f| f.wire_name == name).unwrap()
}

const NO_INLINE: Capabilities = Capabilities {
    inline_records: false,
    ..Capabilities::ALL
};

#[test]
fn pass_1_hoists_inline_records() {
    let doc = schemas(
        "    Pet: { type: object, properties: { owner: { type: object, properties: { name: { type: string } } } } }\n",
    );
    let m = lowered(&doc, &NO_INLINE, &Options::default());
    let owner = field(record(&m, "Pet"), "owner");
    let Expr::Named { decl: id, .. } = owner.ty else {
        panic!("{:?}", owner.ty)
    };
    assert_eq!(m.decl(id).preferred_name, "PetOwner");
    assert_eq!(m.decl(id).origin, Origin::Candidate);
    // With inline records, it stays inline.
    let m = lowered(&doc, &Capabilities::ALL, &Options::default());
    assert!(matches!(
        field(record(&m, "Pet"), "owner").ty,
        Expr::InlineRecord(_)
    ));
}

const ALL_OF: &str = "    Pet: { type: object, properties: { id: { type: string } } }\n    Cat:\n      allOf:\n        - { $ref: \"#/components/schemas/Pet\" }\n        - { type: object, properties: { meow: { type: boolean } } }\n";

#[test]
fn pass_2_all_of_uses_a_base_or_copies_fields() {
    let doc = schemas(ALL_OF);
    let single = Capabilities {
        intersections: false,
        inheritance: Inheritance::Single,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &single, &Options::default());
    let cat = record(&m, "Cat");
    assert_eq!(
        cat.base.map(|b| m.decl(b).preferred_name.clone()),
        Some("Pet".into())
    );
    assert_eq!(
        cat.fields
            .iter()
            .map(|f| f.wire_name.as_str())
            .collect::<Vec<_>>(),
        ["meow"]
    );

    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    let cat = record(&m, "Cat");
    assert_eq!(cat.base, None);
    assert_eq!(
        cat.fields
            .iter()
            .map(|f| f.wire_name.as_str())
            .collect::<Vec<_>>(),
        ["id", "meow"]
    );

    let m = lowered(&doc, &Capabilities::ALL, &Options::default());
    assert!(matches!(
        decl(&m, "Cat").kind,
        DeclKind::Alias(Expr::Intersection(_))
    ));
}

#[test]
fn pass_2_all_of_copies_fields_when_the_child_redeclares_one_of_the_base() {
    // A subclass cannot narrow an inherited field in most languages (mutable fields are invariant,
    // and a required field cannot follow a defaulted one), so the child gets every field instead.
    let doc = schemas(
        "    Label: { type: object, properties: { name: { type: string }, at: { type: [string, \"null\"] } } }\n    Archived:\n      allOf:\n        - { $ref: \"#/components/schemas/Label\" }\n        - { type: object, required: [at], properties: { at: { type: string }, by: { type: string } } }\n",
    );
    let single = Capabilities {
        intersections: false,
        inheritance: Inheritance::Single,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &single, &Options::default());
    let archived = record(&m, "Archived");
    assert_eq!(archived.base, None);
    assert_eq!(
        archived
            .fields
            .iter()
            .map(|f| (f.wire_name.as_str(), f.presence))
            .collect::<Vec<_>>(),
        [
            ("name", Presence::Optional),
            ("at", Presence::Required),
            ("by", Presence::Optional)
        ]
    );
}

#[test]
fn pass_2_all_of_with_a_value_that_is_not_an_object_keeps_one_member_and_warns() {
    let doc = schemas(
        "    Code: { allOf: [{ type: string }, { anyOf: [{ minLength: 1 }, { const: x }] }] }\n",
    );
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    let (m, warnings) = lower(&model, &Capabilities::MINIMAL);
    assert!(validate(&m, &Capabilities::MINIMAL).is_empty());
    assert!(matches!(
        decl(&m, "Code").kind,
        DeclKind::Alias(Expr::Scalar(_))
    ));
    assert_eq!(warnings.len(), 1, "{warnings:?}");
}

const PETS: &str = "    Pet:\n      oneOf: [{ $ref: \"#/components/schemas/Cat\" }, { $ref: \"#/components/schemas/Dog\" }]\n      discriminator: { propertyName: petType }\n    Cat: { type: object, properties: { meow: { type: boolean } } }\n    Dog: { type: object, properties: { bark: { type: boolean } } }\n";

#[test]
fn pass_3_and_6_discriminated_unions() {
    let doc = schemas(PETS);
    let tagged = Capabilities {
        structural_unions: false,
        strict_decoding: true,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &tagged, &Options::default());
    let DeclKind::TaggedUnion(t) = &decl(&m, "Pet").kind else {
        panic!()
    };
    assert_eq!(t.property, "petType");
    assert_eq!(t.variants[0].values, ["Cat"]);
    // Pet is in no operation, so it counts as received too: the fallback is on.
    assert!(t.open);

    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    let DeclKind::UntaggedUnion(u) = &decl(&m, "Pet").kind else {
        panic!()
    };
    assert_eq!(u.discriminator.as_deref(), Some("petType"));
    assert_eq!(u.members.len(), 2);
}

#[test]
fn pass_4_unions_without_a_discriminator() {
    let doc = schemas(
        "    Id: { oneOf: [{ type: string }, { type: integer }] }\n    Raw: { type: [string, integer] }\n",
    );
    let m = lowered(&doc, &Capabilities::ALL, &Options::default());
    assert!(matches!(&decl(&m, "Id").kind, DeclKind::Alias(Expr::Union(ts)) if ts.len() == 2));
    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    assert!(matches!(&decl(&m, "Id").kind, DeclKind::UntaggedUnion(u) if u.members.len() == 2));
    let DeclKind::Alias(Expr::Named { decl: id, .. }) = &decl(&m, "Raw").kind else {
        panic!("{:?}", decl(&m, "Raw").kind)
    };
    assert!(matches!(m.decl(*id).kind, DeclKind::UntaggedUnion(_)));
}

#[test]
fn pass_5_enums_by_capability() {
    let doc = schemas(
        "    Status: { type: string, enum: [a, b] }\n    Level: { type: integer, enum: [1, 2] }\n",
    );
    let integers = Capabilities {
        enums: Enums::IntegerOnly,
        ..Capabilities::MINIMAL
    };
    let m = lowered(&doc, &integers, &Options::default());
    let DeclKind::Enum(status) = &decl(&m, "Status").kind else {
        panic!()
    };
    let DeclKind::Enum(level) = &decl(&m, "Level").kind else {
        panic!()
    };
    assert_eq!(status.repr, EnumRepr::Constants);
    assert_eq!(level.repr, EnumRepr::Native);
}

#[test]
fn pass_5_enums_are_closed_where_sent_and_open_where_received() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    post:
      requestBody:
        content:
          application/json: { schema: { $ref: "#/components/schemas/Pet" } }
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { $ref: "#/components/schemas/Pet" } }
  /status:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { $ref: "#/components/schemas/Status" } }
components:
  schemas:
    Status: { type: string, enum: [a, b] }
    Pet:
      type: object
      properties:
        status: { $ref: "#/components/schemas/Status" }
"##;
    let opts = Options {
        split_visibility: false,
        ..Options::default()
    };
    let m = lowered(doc, &Capabilities::MINIMAL, &opts);
    let pet = decl(&m, "Pet");
    assert_eq!(
        pet.sites,
        Sites {
            sent: true,
            received: true
        }
    );
    let status = decl(&m, "Status");
    let DeclKind::Enum(e) = &status.kind else {
        panic!()
    };
    // Open where received; references say which side they are on.
    assert!(e.open);
    assert_eq!(
        status.sites,
        Sites {
            sent: true,
            received: true
        }
    );
    let Expr::Named { site, .. } = field(record(&m, "Pet"), "status").ty else {
        panic!()
    };
    assert_eq!(site, Site::Both);

    // The same enum only in a request body is closed.
    let request_only = doc.replace(
        "          application/json: { schema: { $ref: \"#/components/schemas/Status\" } }",
        "          application/json: { schema: { type: string } }",
    );
    let request_only = request_only.replace(
        "        \"200\":\n          description: ok\n          content:\n            application/json: { schema: { $ref: \"#/components/schemas/Pet\" } }",
        "        \"204\": { description: ok }",
    );
    let m = lowered(&request_only, &Capabilities::MINIMAL, &opts);
    let DeclKind::Enum(e) = &decl(&m, "Status").kind else {
        panic!()
    };
    assert!(!e.open, "{:?}", decl(&m, "Status").sites);
    assert_eq!(
        decl(&m, "Status").sites,
        Sites {
            sent: true,
            received: false
        }
    );

    // TypeScript's capabilities keep enums closed (design.md 18.5.3).
    let m = lowered(doc, &Capabilities::ALL, &opts);
    let DeclKind::Enum(e) = &decl(&m, "Status").kind else {
        panic!()
    };
    assert!(!e.open);
}

#[test]
fn pass_7_literals_become_their_scalar_without_literal_types() {
    let doc = schemas("    Cat: { const: cat }\n");
    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    assert!(matches!(
        decl(&m, "Cat").kind,
        DeclKind::Alias(Expr::Scalar(_))
    ));
    let m = lowered(&doc, &Capabilities::ALL, &Options::default());
    assert!(matches!(
        decl(&m, "Cat").kind,
        DeclKind::Alias(Expr::Literal(_))
    ));
}

#[test]
fn pass_8_presence() {
    let doc = schemas(
        "    P:\n      type: object\n      required: [a, c]\n      properties:\n        a: { type: string }\n        b: { type: string }\n        c: { type: [string, \"null\"] }\n        d: { type: [string, \"null\"] }\n",
    );
    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    let p = record(&m, "P");
    let presence: Vec<Presence> = p.fields.iter().map(|f| f.presence).collect();
    assert_eq!(
        presence,
        [
            Presence::Required,
            Presence::Optional,
            Presence::Nullable,
            Presence::OptionalNullable
        ]
    );
    // Without nullable primitives, the value cannot be a plain string.
    let caps = Capabilities {
        nullable_primitives: false,
        ..Capabilities::MINIMAL
    };
    let m = lowered(&doc, &caps, &Options::default());
    assert_eq!(field(record(&m, "P"), "c").ty, Expr::Any);
}

#[test]
fn pass_9_tuples_become_lists() {
    let doc = schemas(
        "    Pair: { type: array, prefixItems: [{ type: string }, { type: string }], items: false }\n",
    );
    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    assert!(
        matches!(&decl(&m, "Pair").kind, DeclKind::Alias(Expr::List(t)) if matches!(**t, Expr::Scalar(_)))
    );
    let m = lowered(&doc, &Capabilities::ALL, &Options::default());
    assert!(matches!(
        decl(&m, "Pair").kind,
        DeclKind::Alias(Expr::Tuple { .. })
    ));
}

#[test]
fn pass_10_recursion_gets_indirection() {
    let doc = schemas(
        "    Node:\n      type: object\n      properties:\n        next: { $ref: \"#/components/schemas/Node\" }\n        children: { type: array, items: { $ref: \"#/components/schemas/Node\" } }\n",
    );
    let caps = Capabilities {
        recursion_by_value: false,
        ..Capabilities::MINIMAL
    };
    let m = lowered(&doc, &caps, &Options::default());
    let node = record(&m, "Node");
    assert!(field(node, "next").boxed);
    assert!(!field(node, "children").boxed);
}

#[test]
fn pass_11_additional_properties_are_an_extra_map() {
    let doc = schemas(
        "    Tags: { type: object, properties: { a: { type: string } }, additionalProperties: { type: integer } }\n",
    );
    let m = lowered(&doc, &Capabilities::MINIMAL, &Options::default());
    let tags = record(&m, "Tags");
    assert!(
        matches!(tags.additional.as_deref(), Some(Expr::Scalar(s)) if s.kind == tsurara_sem::ir::ScalarKind::Integer)
    );
}

#[test]
fn the_validator_rejects_what_the_capabilities_do_not_allow() {
    let doc = schemas(PETS);
    // Lowered for TypeScript, checked against Go-like capabilities: tagged unions are not allowed.
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    let (m, _) = lower(&model, &Capabilities::ALL);
    let errors = validate(&m, &Capabilities::MINIMAL);
    assert!(
        errors.iter().any(|e| e.contains("tagged union")),
        "{errors:#?}"
    );
    let doc = schemas(ALL_OF);
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    let (m, _) = lower(&model, &Capabilities::ALL);
    let errors = validate(&m, &Capabilities::MINIMAL);
    assert!(
        errors.iter().any(|e| e.contains("intersection")),
        "{errors:#?}"
    );
}

#[test]
fn pass_2_only_records_are_bases() {
    // `$ref` with sibling keywords is an `allOf`; a union is not a base class.
    let doc = schemas(
        "    A: { type: object, properties: { a: { type: string } } }\n    B: { type: object, properties: { b: { type: string } } }\n    Item: { oneOf: [{ $ref: \"#/components/schemas/A\" }, { $ref: \"#/components/schemas/B\" }] }\n    Input:\n      oneOf:\n        - { type: string }\n        - { type: object, description: an item, $ref: \"#/components/schemas/Item\" }\n",
    );
    let single = Capabilities {
        intersections: false,
        inheritance: Inheritance::Single,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &single, &Options::default());
    for d in &m.decls {
        if let DeclKind::Record(r) = &d.kind {
            if let Some(b) = r.base {
                assert!(
                    matches!(m.decl(b).kind, DeclKind::Record(_)),
                    "{} extends {}",
                    d.preferred_name,
                    m.decl(b).preferred_name
                );
            }
        }
    }
}

#[test]
fn pass_4_a_union_with_properties_does_not_refer_to_itself() {
    let doc = schemas(
        "    A: { type: object, required: [type, role], properties: { type: { type: string }, role: { type: string, enum: [a] } } }\n    B: { type: object, required: [type, role], properties: { type: { type: string }, role: { type: string, enum: [b] } } }\n    C: { type: object, required: [type], properties: { type: { type: string, enum: [c] } } }\n    Param:\n      discriminator: { propertyName: type }\n      oneOf:\n        - discriminator: { propertyName: role }\n          oneOf: [{ $ref: \"#/components/schemas/A\" }, { $ref: \"#/components/schemas/B\" }]\n          properties: { type: { type: string, enum: [message] } }\n        - $ref: \"#/components/schemas/C\"\n",
    );
    // As Python has it.
    let caps = Capabilities {
        inline_records: false,
        intersections: false,
        inheritance: Inheritance::Single,
        strict_decoding: true,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &caps, &Options::default());
    for (i, d) in m.decls.iter().enumerate() {
        if let DeclKind::Alias(Expr::Named { decl, .. }) = &d.kind {
            assert_ne!(decl.0, i, "{} refers to itself", d.preferred_name);
        }
    }
}

#[test]
fn operations_are_lowered_with_their_inline_types_hoisted() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets/{id}:
    post:
      operationId: updatePet
      parameters:
        - { name: id, in: path, required: true, schema: { type: integer } }
        - { name: mode, in: query, schema: { type: string, enum: [fast, slow] } }
      requestBody:
        content:
          application/json:
            schema: { type: object, properties: { name: { type: string } } }
          application/octet-stream:
            schema: { type: string, format: binary }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { $ref: "#/components/schemas/Pet" }
            text/plain:
              schema: { type: string }
        "204": { description: none }
components:
  schemas:
    Pet: { type: object, properties: { id: { type: integer } } }
"##;
    let caps = Capabilities {
        inline_records: false,
        intersections: false,
        inheritance: Inheritance::Single,
        strict_decoding: true,
        ..Capabilities::ALL
    };
    let m = lowered(doc, &caps, &Options::default());
    assert_eq!(m.operations.len(), 1);
    let op = &m.operations[0];
    assert!(matches!(op.params[0], Expr::Scalar(_)));
    // The enum of a parameter is sent: closed.
    let Expr::Named { decl, site } = &op.params[1] else {
        panic!("{:?}", op.params[1])
    };
    assert_eq!(*site, Site::Sent);
    assert!(matches!(&m.decl(*decl).kind, DeclKind::Enum(e) if !e.open));
    // The inline body object becomes a record; the binary body is bytes.
    let Expr::Named { decl, .. } = &op.body[0] else {
        panic!("{:?}", op.body[0])
    };
    assert!(matches!(m.decl(*decl).kind, DeclKind::Record(_)));
    assert_eq!(op.body[1], Expr::Bytes);
    // Responses: the schema type for JSON, a string for text, nothing without content.
    assert!(matches!(
        &op.responses[0][0],
        Expr::Named {
            site: Site::Received,
            ..
        }
    ));
    assert!(matches!(&op.responses[0][1], Expr::Scalar(_)));
    assert!(op.responses[1].is_empty());
}

#[test]
fn pass_11_objects_with_only_additional_properties_are_maps() {
    let doc = schemas(
        "    Counts: { type: object, additionalProperties: { type: integer } }\n    Pet: { type: object, properties: { tags: { type: object, additionalProperties: { type: string } } } }\n",
    );
    let m = lowered(&doc, &NO_INLINE, &Options::default());
    assert!(matches!(
        &decl(&m, "Counts").kind,
        DeclKind::Alias(Expr::Map(t)) if matches!(**t, Expr::Scalar(_))
    ));
    assert!(matches!(field(record(&m, "Pet"), "tags").ty, Expr::Map(_)));
}

#[test]
fn pass_10_recursion_through_unions_gets_indirection() {
    // Node -> Tree (a union) -> Node, all by value: a field must be boxed.
    let doc = schemas(
        "    Node: { type: object, properties: { parent: { $ref: \"#/components/schemas/Tree\" } } }\n    Leaf: { type: object, properties: { v: { type: string } } }\n    Tree: { oneOf: [{ $ref: \"#/components/schemas/Node\" }, { $ref: \"#/components/schemas/Leaf\" }] }\n",
    );
    let caps = Capabilities {
        recursion_by_value: false,
        ..Capabilities::MINIMAL
    };
    let m = lowered(&doc, &caps, &Options::default());
    assert!(field(record(&m, "Node"), "parent").boxed);
    assert!(!field(record(&m, "Leaf"), "v").boxed);
}

#[test]
fn pass_3_discriminated_unions_with_a_member_no_value_selects_are_untagged() {
    let doc = schemas(
        "    Event:\n      oneOf:\n        - { type: object, properties: { type: { const: created } } }\n        - { type: object, properties: { type: { type: string } } }\n      discriminator: { propertyName: type }\n",
    );
    let tagged = Capabilities {
        structural_unions: false,
        strict_decoding: true,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &tagged, &Options::default());
    let DeclKind::UntaggedUnion(u) = &decl(&m, "Event").kind else {
        panic!("{:?}", decl(&m, "Event").kind)
    };
    assert_eq!(u.discriminator.as_deref(), Some("type"));
    assert_eq!(u.members.len(), 2);
}

#[test]
fn pass_2_an_all_of_member_of_a_union_that_keeps_one_member_is_not_the_union() {
    let doc = schemas(
        "    S1:\n      type: object\n      properties:\n        Error:\n          anyOf:\n            - { type: string }\n            - allOf: [{ type: number }, { $ref: \"#/components/schemas/S3\" }]\n    S3: { type: array, items: { type: string } }\n",
    );
    let caps = Capabilities {
        inline_records: false,
        structural_unions: false,
        intersections: false,
        ..Capabilities::ALL
    };
    let m = lowered(&doc, &caps, &Options::default());
    for (i, d) in m.decls.iter().enumerate() {
        if let DeclKind::UntaggedUnion(u) = &d.kind {
            for member in &u.members {
                assert!(
                    !matches!(member, Expr::Named { decl, .. } if decl.0 == i),
                    "{} refers to itself: {:#?}",
                    d.preferred_name,
                    m.decls
                );
            }
        }
    }
}

#[test]
fn multipart_bodies_are_lowered_from_their_schema() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /upload:
    post:
      operationId: upload
      requestBody:
        content:
          multipart/form-data:
            schema:
              type: object
              properties: { file: { type: string, format: binary }, note: { type: string } }
      responses: { "204": { description: ok } }
"##;
    let m = lowered(doc, &NO_INLINE, &Options::default());
    let Expr::Named { decl, .. } = &m.operations[0].body[0] else {
        panic!("{:?}", m.operations[0].body)
    };
    let DeclKind::Record(r) = &m.decl(*decl).kind else {
        panic!()
    };
    assert_eq!(r.fields[0].ty, Expr::Bytes);
}

const STREAMS: &str = r##"
openapi: 3.1.1
info: { title: t, version: "1" }
paths:
  /stream:
    get:
      operationId: stream
      responses:
        "200":
          description: ok
          content:
            text/event-stream:
              x-itemSchema:
                type: object
                required: [event, data]
                properties: { event: { type: string }, data: { type: string }, id: { type: string } }
                oneOf:
                  - properties:
                      event: { const: note }
                      data:
                        type: string
                        contentMediaType: application/json
                        contentSchema: { $ref: "#/components/schemas/Note" }
                  - properties:
                      event: { enum: [ping, pong] }
                  - properties:
                      event: { type: string }
  /lines:
    get:
      operationId: lines
      responses:
        "200":
          description: ok
          content:
            application/jsonl:
              x-itemSchema: { $ref: "#/components/schemas/Note" }
  /raw:
    get:
      operationId: raw
      responses:
        "200":
          description: ok
          content:
            text/event-stream: { schema: { type: string } }
components:
  schemas:
    Note: { type: object, required: [id], properties: { id: { type: string } } }
"##;

#[test]
fn sequences_are_read_item_by_item() {
    let tagged = Capabilities {
        structural_unions: false,
        inline_records: false,
        ..Capabilities::ALL
    };
    let m = lowered(STREAMS, &tagged, &Options::default());
    // stream, lines, raw.
    assert_eq!(
        m.operations
            .iter()
            .map(|o| o.sequences[0][0])
            .collect::<Vec<_>>(),
        [Some(Sequence::Events), Some(Sequence::JsonLines), None]
    );
    assert_eq!(m.operations[2].responses[0][0], Expr::Bytes);

    // Events are a union tagged by `event`; an event without a name is the default member.
    let Expr::Named { decl: union, .. } = &m.operations[0].responses[0][0] else {
        panic!("{:?}", m.operations[0].responses[0][0])
    };
    let union = m.decl(*union);
    assert_eq!(union.preferred_name, "StreamEvent");
    let DeclKind::TaggedUnion(t) = &union.kind else {
        panic!("{:?}", union.kind)
    };
    assert_eq!(t.property, "event");
    let values: Vec<&[String]> = t.variants.iter().map(|v| v.values.as_slice()).collect();
    assert_eq!(
        values,
        [&["note".to_owned()][..], &["ping".into(), "pong".into()]]
    );
    assert!(t.default.is_some());
    let note = record(&m, "StreamNoteEvent");
    assert!(
        matches!(&field(note, "data").ty, Expr::Named { decl, .. } if m.decl(*decl).preferred_name == "Note")
    );
    assert_eq!(field(note, "data").presence, Presence::Required);
    assert_eq!(field(note, "id").presence, Presence::Optional);
    record(&m, "StreamPingEvent");
    record(&m, "StreamOtherEvent");

    // The item of a JSON Lines response is the item schema's type.
    assert!(
        matches!(&m.operations[1].responses[0][0], Expr::Named { decl, .. } if m.decl(*decl).preferred_name == "Note")
    );

    // Without tagged unions, events are tried in order.
    let m = lowered(STREAMS, &Capabilities::MINIMAL, &Options::default());
    assert!(
        matches!(&decl(&m, "StreamEvent").kind, DeclKind::UntaggedUnion(u) if u.members.len() == 3)
    );
}
