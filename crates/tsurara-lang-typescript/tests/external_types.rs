//! External types written in definitions (`x-tsurara-type`) are printed only when they are type
//! expressions of a restricted grammar, so that a definition cannot put code into the output.

use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, WarningKind};

fn doc(entry: &str) -> String {
    format!(
        r##"
openapi: 3.1.0
info: {{ title: t, version: "1" }}
paths: {{}}
components:
  schemas:
    T:
      type: object
      properties:
        t: {{ type: string, x-tsurara-type: {{ typescript: {entry} }} }}
"##
    )
}

fn emit(entry: &str) -> (String, Vec<String>) {
    let set = DocumentSet::from_value(parse_document(doc(entry).as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    let all = out.files.iter().map(|f| f.contents.as_str()).collect();
    let invalid = out
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidDefinition)
        .map(|w| w.message.clone())
        .collect();
    (all, invalid)
}

#[test]
fn injected_code_is_not_printed() {
    for entry in [
        r#"{ type: "string }; console.log('INJ'); type Z = { a: 1", import: { from: "node:child_process", name: exec } }"#,
        r#"{ type: "string /* x */" }"#,
        r#"{ type: "string // x" }"#,
        r#"{ type: "string\nconsole.log(1)" }"#,
        r#"{ type: "`${string}; console.log(1)" }"#,
        r#"{ type: "`${ { a: console.log(1) } }`" }"#,
        r#"{ type: Brand, import: { from: "./a'; console.log(1); '", name: Brand } }"#,
        r#"{ type: Brand, import: { from: ./brand.js, name: "Brand }" } }"#,
        r#"{ type: "Record<string, number" }"#,
    ] {
        let (all, invalid) = emit(entry);
        assert!(
            !all.contains("INJ") && !all.contains("console") && !all.contains("child_process"),
            "{entry}:\n{all}"
        );
        assert!(!all.contains("Brand"), "{entry}:\n{all}");
        assert!(all.contains("t?: string"), "{entry}:\n{all}");
        assert_eq!(invalid.len(), 1, "{entry}: {invalid:?}");
    }
}

#[test]
fn valid_entries_still_work() {
    let (all, invalid) =
        emit(r#"{ type: "Brand<'Uuid'>", import: { from: ./brand.js, name: Brand } }"#);
    assert!(invalid.is_empty(), "{invalid:?}");
    assert!(all.contains("t?: Brand<'Uuid'>"), "{all}");
    assert!(all.contains("import type { Brand } from"), "{all}");
    let (all, invalid) = emit(r#"{ type: "`${number}px`" }"#);
    assert!(invalid.is_empty(), "{invalid:?}");
    assert!(all.contains("t?: `${number}px`"), "{all}");
    let (all, invalid) = emit(r#"{ type: "Array<A.B | 'x' | -1>[] & C" }"#);
    assert!(invalid.is_empty(), "{invalid:?}");
    assert!(all.contains("Array<A.B | 'x' | -1>[] & C"), "{all}");
}
