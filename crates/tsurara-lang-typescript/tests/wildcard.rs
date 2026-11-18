//! Wildcard media types (design.md 8.5).

use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::model::Model;
use tsurara_sem::operation::{BodyKind, MediaContent};
use tsurara_sem::{Options, WarningKind, build_model};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /file:
    get:
      operationId: download
      responses:
        "200":
          description: ok
          content: { "*/*": { schema: { type: string, format: binary } } }
    post:
      operationId: upload
      requestBody:
        content: { "*/*": { schema: { type: string, format: binary } } }
      responses: { "204": { description: ok } }
  /thing:
    get:
      operationId: getThing
      responses:
        "200":
          description: ok
          content: { "*/*": { schema: { $ref: "#/components/schemas/Thing" } } }
    post:
      operationId: sendThing
      requestBody:
        content: { "application/*": { schema: { $ref: "#/components/schemas/Thing" } } }
      responses: { "204": { description: ok } }
  /json:
    get:
      operationId: getJsonString
      responses:
        "200":
          description: ok
          content:
            application/json: { schema: { type: string, contentMediaType: text/html } }
components:
  schemas:
    Thing: { type: object, properties: { a: { type: string } } }
"##;

fn model(opts: &Options) -> Model {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    // Body types are what TypeScript makes of the body kind, so the lowered model is checked.
    tsurara_lang_typescript::lower::lower_model(
        &build_model(&set, opts).unwrap(),
        opts,
        &TypeScriptOptions::default(),
    )
    .0
}

fn response(m: &Model, op: &str) -> MediaContent {
    let op = m.operations.iter().find(|o| o.name == op).unwrap();
    op.responses[0].contents[0].clone()
}

fn request(m: &Model, op: &str) -> MediaContent {
    let op = m.operations.iter().find(|o| o.name == op).unwrap();
    op.body.as_ref().unwrap().contents[0].clone()
}

fn wildcard_warnings(m: &Model) -> Vec<String> {
    m.warnings
        .iter()
        .filter(|w| w.kind == WarningKind::WildcardMediaType)
        .map(|w| w.loc.to_string())
        .collect()
}

#[test]
fn a_wildcard_response_with_a_binary_schema_is_a_blob() {
    let m = model(&Options::default());
    let c = response(&m, "download");
    assert_eq!(c.body_kind, BodyKind::Blob);
    assert_eq!(print_type(&c.ty), "Blob");
}

#[test]
fn a_wildcard_request_with_a_binary_schema_is_sent_as_is() {
    let m = model(&Options::default());
    let c = request(&m, "upload");
    assert_eq!(
        print_type(&c.ty),
        "Blob | ArrayBuffer | Uint8Array | ReadableStream<Uint8Array>"
    );
}

#[test]
fn a_wildcard_with_a_non_binary_schema_is_auto_and_warns() {
    let m = model(&Options::default());
    let c = response(&m, "getThing");
    assert_eq!(c.body_kind, BodyKind::Auto);
    assert_eq!(print_type(&c.ty), "Thing | string | Blob");
    assert_eq!(print_type(&request(&m, "sendThing").ty), "Thing");
    let warnings = wildcard_warnings(&m);
    assert!(
        warnings.iter().any(|w| w.contains("~1thing/get")),
        "{warnings:?}"
    );
    assert!(
        warnings.iter().any(|w| w.contains("~1thing/post")),
        "{warnings:?}"
    );
    assert!(
        !warnings.iter().any(|w| w.contains("~1file/")),
        "{warnings:?}"
    );
}

#[test]
fn wildcards_can_be_treated_as_json() {
    let opts = Options {
        wildcard_as_json: true,
        ..Options::default()
    };
    let m = model(&opts);
    let c = response(&m, "getThing");
    assert_eq!(c.body_kind, BodyKind::Json);
    assert_eq!(print_type(&c.ty), "Thing");
    assert!(wildcard_warnings(&m).is_empty());
}

#[test]
fn json_media_types_keep_strings_as_strings() {
    let m = model(&Options::default());
    let c = response(&m, "getJsonString");
    assert_eq!(c.body_kind, BodyKind::Json);
    assert_eq!(print_type(&c.ty), "string");
}

/// Prints a model type as TypeScript prints it, after the TypeScript lowering.
fn print_type(ty: &tsurara_sem::ir::Type) -> String {
    tsurara_sem::print::print_type(&tsurara_lang_typescript::lower::lower(ty))
}
