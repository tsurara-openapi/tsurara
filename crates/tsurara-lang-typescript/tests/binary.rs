//! Binary values by context (design.md 6.4): JSON values are strings, while the HTTP body
//! itself and multipart part values are binary.

use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, build_model};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /page:
    post:
      operationId: putPage
      requestBody:
        content:
          application/json:
            schema:
              type: object
              properties:
                html: { type: string, contentMediaType: text/html }
      responses: { "204": { description: ok } }
  /inline:
    post:
      operationId: uploadInline
      requestBody:
        content:
          multipart/form-data:
            schema:
              type: object
              properties:
                file: { type: string, format: binary }
                files: { type: array, items: { type: string, format: binary } }
                avatar: { $ref: "#/components/schemas/BinaryFile" }
                meta: { type: object, properties: { a: { type: string } } }
      responses: { "204": { description: ok } }
  /named:
    post:
      operationId: uploadNamed
      requestBody:
        content:
          multipart/form-data:
            schema: { $ref: "#/components/schemas/UploadForm" }
      responses: { "204": { description: ok } }
  /mixed:
    post:
      operationId: uploadMixed
      requestBody:
        content:
          multipart/form-data:
            schema: { $ref: "#/components/schemas/Mixed" }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { $ref: "#/components/schemas/Mixed" }
  /shared:
    post:
      operationId: uploadShared
      requestBody: { $ref: "#/components/requestBodies/Shared" }
      responses: { "204": { description: ok } }
  /plain:
    post:
      operationId: uploadPlain
      requestBody:
        content:
          multipart/form-data:
            schema: { $ref: "#/components/schemas/Plain" }
      responses: { "204": { description: ok } }
components:
  requestBodies:
    Shared:
      content:
        multipart/form-data:
          schema: { $ref: "#/components/schemas/SharedForm" }
  schemas:
    BinaryFile: { type: string, format: binary }
    UploadForm:
      type: object
      required: [file]
      properties:
        file: { type: string, format: binary }
        note: { type: string }
    Mixed:
      type: object
      properties:
        file: { type: string, format: binary }
    SharedForm:
      type: object
      properties:
        file: { type: string, format: binary }
    Plain:
      type: object
      properties:
        note: { type: string }
"##;

/// The printed type without JSDoc, on one line.
fn flat(printed: String) -> String {
    let mut out = String::new();
    let mut rest = printed.as_str();
    while let Some(start) = rest.find("/**") {
        out.push_str(&rest[..start]);
        let end = rest[start..].find("*/").expect("closed") + start + 2;
        rest = &rest[end..];
    }
    out.push_str(rest);
    out.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace("; }", " }")
}

fn body_doc(name: &str) -> String {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let model = named(&set);
    let op = model.operations.iter().find(|o| o.name == name).unwrap();
    print_type(&op.body.as_ref().unwrap().contents[0].ty)
}

/// The model as TypeScript names and lowers it.
fn named(set: &DocumentSet) -> tsurara_sem::Model {
    let opts = Options::default();
    tsurara_lang_typescript::lower::lower_model(
        &build_model(set, &opts).unwrap(),
        &opts,
        &TypeScriptOptions::default(),
    )
    .0
}

fn body(name: &str) -> String {
    flat(body_doc(name))
}

fn decl(name: &str) -> String {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let model = named(&set);
    flat(print_type(
        &model.schemas.iter().find(|d| d.name == name).unwrap().ty,
    ))
}

#[test]
fn content_media_type_inside_json_is_a_string() {
    assert_eq!(body("putPage"), "{ html?: string }");
    assert!(body_doc("putPage").contains("@contentMediaType text/html"));
}

#[test]
fn multipart_part_values_are_binary() {
    assert_eq!(
        body("uploadInline"),
        "{ file?: Blob | File; files?: (Blob | File)[]; avatar?: Blob | File; meta?: { a?: string } }"
    );
}

#[test]
fn named_schemas_used_only_by_multipart_bodies_have_part_values() {
    assert_eq!(decl("UploadForm"), "{ file: Blob | File; note?: string }");
    assert_eq!(body("uploadNamed"), "UploadForm");
    assert_eq!(decl("SharedForm"), "{ file?: Blob | File }");
    assert_eq!(body("uploadShared"), "SharedForm");
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    let warned: Vec<&str> = model
        .warnings
        .iter()
        .map(|w| w.loc.pointer.as_str())
        .collect();
    assert!(
        !warned
            .iter()
            .any(|p| p.contains("UploadForm") || p.contains("SharedForm")),
        "{warned:?}"
    );
}

#[test]
fn named_schemas_also_used_in_json_stay_true_for_json_and_multipart_expands_them() {
    assert_eq!(decl("Mixed"), "{ file?: string }");
    assert_eq!(body("uploadMixed"), "{ file?: Blob | File }");
    // A binary string schema referenced from a part is still a JSON string on its own.
    assert_eq!(decl("BinaryFile"), "string");
}

#[test]
fn named_multipart_schemas_without_binary_parts_stay_references() {
    assert_eq!(body("uploadPlain"), "Plain");
}

/// Prints a model type as TypeScript prints it, after the TypeScript lowering.
fn print_type(ty: &tsurara_sem::ir::Type) -> String {
    tsurara_sem::print::print_type(&tsurara_lang_typescript::lower::lower(ty))
}
