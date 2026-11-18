//! Semantic warnings describe the definition, not a language (design.md 18.4.5).

use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, build_model};

#[test]
fn semantic_warnings_do_not_mention_a_language() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    A: { not: { type: string } }
    B: { type: string, not: { const: "" } }
    C: { type: object, if: { required: [a] }, then: { required: [b] } }
    D: { type: object, dependentSchemas: { a: { required: [b] } } }
"##;
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let model = build_model(&set, &Options::default()).unwrap();
    assert_eq!(model.warnings.len(), 4, "{:?}", model.warnings);
    for w in &model.warnings {
        assert!(!w.message.contains("TypeScript"), "{w}");
        assert!(!w.message.contains("unknown"), "{w}");
        assert_eq!(w.language, None, "{w}");
    }
}
