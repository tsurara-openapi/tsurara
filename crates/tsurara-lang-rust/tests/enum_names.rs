//! Enum members are named by `x-enum-varnames` (or `x-enumNames`) when the definition has them.

use tsurara_lang_rust::{RustOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Color: { type: string, enum: [r, g], x-enum-varnames: [Red, Green] }
    Size: { type: integer, enum: [1, 2], x-enumNames: [Small, Large] }
"##;

fn all_files() -> String {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    files(&set)
        .into_iter()
        .map(|(_, c)| c)
        .collect::<Vec<_>>()
        .join("\n")
}

fn files(set: &DocumentSet) -> Vec<(String, String)> {
    generate(set, &Options::default(), &RustOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

#[test]
fn members_are_named_by_x_enum_varnames() {
    let out = all_files();
    for line in [
        "    #[serde(rename = \"r\")]\n    Red,\n",
        "    #[serde(rename = \"g\")]\n    Green,\n",
        "pub const SIZE_SMALL: Size = 1;\n",
        "pub const SIZE_LARGE: Size = 2;\n",
    ] {
        assert!(out.contains(line), "missing `{line}` in:\n{out}");
    }
}
