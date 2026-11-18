//! External types written in definitions (`x-tsurara-type`) are printed only when they are annotations
//! of a restricted grammar, and their imports are for type checkers only.

use tsurara_lang_python::{PythonOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, WarningKind};

fn emit(entry: &str) -> (String, usize) {
    let doc = format!(
        "openapi: 3.1.0\ninfo: {{ title: t, version: \"1\" }}\npaths: {{}}\ncomponents:\n  schemas:\n    T:\n      type: object\n      properties:\n        t: {{ type: string, x-tsurara-type: {{ python: {entry} }} }}\n"
    );
    let set = DocumentSet::from_value(parse_document(doc.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &PythonOptions::default()).unwrap();
    let t = out
        .files
        .iter()
        .find(|f| f.path == "t.py")
        .unwrap()
        .contents
        .clone();
    let invalid = out
        .warnings
        .iter()
        .filter(|w| w.kind == WarningKind::InvalidDefinition)
        .count();
    (t, invalid)
}

#[test]
fn injected_code_is_not_printed() {
    for entry in [
        r#"{ type: "str\nimport os; os.system('x')" }"#,
        r#"{ type: "str = __import__('os')" }"#,
        r#"{ type: "str  # comment" }"#,
        r#"{ type: Thing, import: { from: "os; import antigravity", name: Thing } }"#,
        r#"{ type: Thing, import: { from: uuid, name: "UUID\nimport os" } }"#,
    ] {
        let (t, invalid) = emit(entry);
        assert!(
            !t.contains("os") && !t.contains("antigravity") && !t.contains("Thing"),
            "{entry}:\n{t}"
        );
        assert!(t.contains("t: str | None = None"), "{entry}:\n{t}");
        assert_eq!(invalid, 1, "{entry}");
    }
}

#[test]
fn valid_entries_are_imported_for_type_checkers_only() {
    let (t, invalid) =
        emit(r#"{ type: "UUID | list[Literal['x']]", import: { from: uuid, name: UUID } }"#);
    assert_eq!(invalid, 0);
    assert!(
        t.contains("t: UUID | list[Literal['x']] | None = None"),
        "{t}"
    );
    // Under TYPE_CHECKING, so importing the module runs nothing of the imported one.
    let checking = t.split("if TYPE_CHECKING:").nth(1).unwrap_or_default();
    assert!(checking.contains("from uuid import UUID"), "{t}");
}
