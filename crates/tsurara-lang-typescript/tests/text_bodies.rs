//! Bodies of text media types: strings, unless their schema says the body is binary, which keeps
//! it in its original bytes (another charset, or a file to pass on).

use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /orders/export:
    get:
      operationId: exportOrders
      responses:
        "200":
          description: ok
          content:
            text/csv: { schema: { type: string, format: binary } }
  /notes:
    get:
      operationId: getNotes
      responses:
        "200":
          description: ok
          content:
            text/plain: { schema: { type: string } }
    put:
      operationId: putNotes
      requestBody:
        required: true
        content:
          text/csv: { schema: { $ref: "#/components/schemas/Csv" } }
      responses: { "204": { description: ok } }
components:
  schemas:
    Csv: { type: string, format: binary }
"##;

fn files() -> Vec<(String, String)> {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    out.files
        .into_iter()
        .map(|f| (format!("gen/{}", f.path), f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files.iter().find(|(p, _)| p == path).unwrap().1
}

#[test]
fn a_binary_schema_makes_a_text_body_a_blob() {
    let files = files();
    let export = file(&files, "gen/operations/exportOrders.ts");
    assert!(export.contains("200: { parseAs: 'blob' }"), "{export}");
    assert!(export.contains("data: Blob;"), "{export}");
    let put = file(&files, "gen/operations/putNotes.ts");
    assert!(put.contains("body: Blob"), "{put}");
    let notes = file(&files, "gen/operations/getNotes.ts");
    assert!(notes.contains("200: { parseAs: 'text' }"), "{notes}");
    assert!(notes.contains("data: string;"), "{notes}");
    assert_tsc_passes(&files, &TsOptions::default());
}
