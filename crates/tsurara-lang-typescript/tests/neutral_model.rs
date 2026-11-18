//! The model holds no TypeScript type names, and external types are per language.

use std::path::Path;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, parse_document};
use tsurara_sem::ir::Type;
use tsurara_sem::operation::BodyKind;
use tsurara_sem::{Options, WarningKind, build_model};

fn fixture(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    DocumentSet::load(&url, &f as &dyn Fetcher).unwrap()
}

#[test]
fn the_model_has_no_typescript_type_names() {
    for name in ["kitchen-sink.yaml", "petstore3.json"] {
        let model = build_model(&fixture(name), &Options::default()).unwrap();
        let debug = format!("{model:?}");
        assert!(!debug.contains("Builtin"), "{name}");
    }
}

#[test]
fn binary_values_and_body_kinds_are_neutral() {
    let model = build_model(&fixture("kitchen-sink.yaml"), &Options::default()).unwrap();
    let upload = model
        .operations
        .iter()
        .find(|o| {
            o.body
                .as_ref()
                .is_some_and(|b| b.contents[0].media_type == "multipart/form-data")
        })
        .unwrap();
    // The inline body object is a hoisting candidate.
    let Type::Inline(body) = &upload.body.as_ref().unwrap().contents[0].ty else {
        panic!()
    };
    let Type::Object(o) = &body.ty else { panic!() };
    assert_eq!(o.properties[0].ty, Type::Binary);
    let petstore = build_model(&fixture("petstore3.json"), &Options::default()).unwrap();
    let image = petstore
        .operations
        .iter()
        .find(|o| o.id == "uploadFile")
        .unwrap();
    let c = &image.body.as_ref().unwrap().contents[0];
    assert_eq!(c.body_kind, BodyKind::Blob);
}

#[test]
fn languages_without_an_entry_use_the_schema_and_warn() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Id: { type: string, x-tsurara-type: { rust: uuid::Uuid } }
"##;
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    let Type::Mapped(m) = &model.schemas[0].ty else {
        panic!("{:?}", model.schemas[0].ty)
    };
    assert_eq!(m.entries.keys().collect::<Vec<_>>(), ["rust"]);
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    let id = out
        .files
        .iter()
        .find(|f| f.path == "schemas/Id.ts")
        .unwrap();
    assert!(
        id.contents.contains("export type Id = string;"),
        "{}",
        id.contents
    );
    let w = out
        .warnings
        .iter()
        .find(|w| w.kind == WarningKind::UnmappedType)
        .expect("a warning for the missing typescript entry");
    assert_eq!(w.loc.pointer, "/components/schemas/Id/x-tsurara-type");
}

#[test]
fn backend_warnings_name_the_language() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Id: { type: string, x-tsurara-type: { rust: uuid::Uuid } }
    Error: { type: string, not: { const: "" } }
"##;
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    for w in &out.warnings {
        match w.kind {
            WarningKind::UnmappedType | WarningKind::Renamed => {
                assert_eq!(w.language.as_deref(), Some("typescript"), "{w}")
            }
            _ => assert_eq!(w.language, None, "{w}"),
        }
    }
    assert_eq!(out.warnings.len(), 3, "{:?}", out.warnings);
}
