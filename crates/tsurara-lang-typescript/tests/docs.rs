use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::print::print_type_alias;
use tsurara_sem::{Options, build_model};

#[test]
fn jsdoc_snapshot() {
    let doc = parse_document(
        br#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Pet:
      title: Pet
      description: "A pet. Never ends with */ in a comment."
      deprecated: true
      type: object
      required: [id]
      properties:
        id:
          type: integer
          format: int64
          minimum: 1
          description: Unique id
          examples: [42]
        name:
          type: string
          maxLength: 64
          pattern: "^[a-z]+$"
          default: rex
        tags:
          type: array
          items: { type: string }
          minItems: 1
          uniqueItems: true
        status:
          type: string
          enum: [available, sold]
          deprecated: true
          example: sold
"#,
    )
    .unwrap();
    let set = DocumentSet::from_value(doc);
    let opts = Options::default();
    let model = tsurara_lang_typescript::lower::lower_model(
        &build_model(&set, &opts).unwrap(),
        &opts,
        &TypeScriptOptions::default(),
    )
    .0;
    let out: String = model
        .schemas
        .iter()
        .map(|d| print_type_alias(&d.name, &d.ty, d.doc.as_ref()))
        .collect();
    insta::assert_snapshot!(out);
}

/// Tuple length constraints approximated with the rest element type stay in JSDoc (design.md 6.4).
#[test]
fn approximated_tuple_lengths_are_documented() {
    let doc = parse_document(
        br#"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    AtLeastThree:
      type: array
      prefixItems: [{ type: string }, { type: number }]
      items: { type: boolean }
      minItems: 3
    AtMostThree:
      type: array
      prefixItems: [{ type: string }]
      items: { type: boolean }
      maxItems: 3
"#,
    )
    .unwrap();
    let set = DocumentSet::from_value(doc);
    let opts = Options::default();
    let model = tsurara_lang_typescript::lower::lower_model(
        &build_model(&set, &opts).unwrap(),
        &opts,
        &TypeScriptOptions::default(),
    )
    .0;
    for (name, tag) in [
        ("AtLeastThree", "@minItems 3"),
        ("AtMostThree", "@maxItems 3"),
    ] {
        let d = model.schemas.iter().find(|d| d.name == name).unwrap();
        let out = print_type_alias(&d.name, &d.ty, d.doc.as_ref());
        assert!(out.contains(tag), "{name} should document `{tag}`:\n{out}");
    }
}
