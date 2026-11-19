//! `bodilessResponses: read`: error responses the definition gives no content are read when they
//! arrive with a Content-Type, so that a message the definition forgot is not lost.

use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_lang_typescript::options::BodilessResponses;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes};

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /orders:
    post:
      operationId: createOrder
      responses:
        "201": { description: created }
        "204": { description: nothing }
        "400": { description: bad }
        "409":
          description: conflict
          content:
            application/json: { schema: { type: object, properties: { code: { type: string } } } }
        "5XX": { description: failed }
        default: { description: other }
    head:
      operationId: checkOrders
      responses:
        "200": { description: ok }
        "404": { description: none }
"##;

fn files(bodiless: BodilessResponses) -> Vec<(String, String)> {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let ts = TypeScriptOptions {
        bodiless_responses: bodiless,
        ..TypeScriptOptions::default()
    };
    generate(&set, &Options::default(), &ts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (format!("gen/{}", f.path), f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files.iter().find(|(p, _)| p == path).unwrap().1
}

#[test]
fn error_responses_without_content_are_read_when_asked() {
    let files = files(BodilessResponses::Read);
    let create = file(&files, "gen/operations/createOrder.ts");
    for line in [
        "201: { parseAs: 'none' },",
        "204: { parseAs: 'none' },",
        "400: { parseAs: 'none', readWithContentType: true },",
        "409: { parseAs: 'json' },",
        "'5XX': { parseAs: 'none', readWithContentType: true },",
        "default: { parseAs: 'none', readWithContentType: true },",
    ] {
        assert!(create.contains(line), "missing `{line}` in:\n{create}");
    }
    assert!(create.contains("status: 400; error: unknown;"), "{create}");
    assert!(create.contains("status: 201; data: undefined;"), "{create}");
    let head = file(&files, "gen/operations/checkOrders.ts");
    assert!(head.contains("404: { parseAs: 'none' },"), "{head}");
    assert!(head.contains("status: 404; error: undefined;"), "{head}");
    assert_tsc_passes(&files, &TsOptions::default());
}

#[test]
fn by_default_they_are_discarded() {
    let files = files(BodilessResponses::Discard);
    let create = file(&files, "gen/operations/createOrder.ts");
    assert!(create.contains("400: { parseAs: 'none' },"), "{create}");
    assert!(
        create.contains("status: 400; error: undefined;"),
        "{create}"
    );
    assert!(!create.contains("readWithContentType"), "{create}");
}
