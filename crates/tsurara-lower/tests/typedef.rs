//! TypeDefs (design.md 18.7): the format validates, the vectors in
//! `spec/conformance/typedefs.json` hold for a small reference converter, and they cover every
//! construct of the format.

use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2};
use tsurara_lower::typedef::reference::Converter;
use tsurara_lower::typedef::type_defs;
use tsurara_lower::{Capabilities, Inheritance, lower};
use tsurara_sem::{Options, build_model};

fn repo(path: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(path)
}

fn schema() -> jsonschema::Validator {
    let s: Value =
        serde_json::from_str(&std::fs::read_to_string(repo("spec/type-def.schema.json")).unwrap())
            .unwrap();
    jsonschema::validator_for(&s).unwrap()
}

fn vectors() -> Value {
    serde_json::from_str(&std::fs::read_to_string(repo("spec/conformance/typedefs.json")).unwrap())
        .unwrap()
}

/// Capabilities of a language that converts with TypeDefs, like Python with dataclasses.
const TYPEDEF_LANGUAGE: Capabilities = Capabilities {
    inline_records: false,
    intersections: false,
    tagged_unions: true,
    inheritance: Inheritance::Single,
    strict_decoding: true,
    ..Capabilities::ALL
};

#[test]
fn type_defs_of_the_fixtures_validate() {
    let validator = schema();
    let dir = repo("crates/tsurara-lang-typescript/tests/fixtures");
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
        let mut f = MemoryFetcher::new();
        f.insert(url.as_str(), std::fs::read(&path).unwrap());
        let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
        normalize_swagger2(&mut set).unwrap();
        let model = build_model(&set, &Options::default()).unwrap();
        for caps in [TYPEDEF_LANGUAGE, Capabilities::MINIMAL] {
            let (module, _) = lower(&model, &caps);
            let names: Vec<String> = (0..module.decls.len())
                .map(|i| format!("{}{i}", module.decls[i].preferred_name))
                .collect();
            let defs = type_defs(&module, &names, &|_, w: &str| w.to_owned(), "python");
            let errors: Vec<String> = validator
                .iter_errors(&defs)
                .map(|e| format!("{e} at {}", e.instance_path()))
                .collect();
            assert!(errors.is_empty(), "{name}: {errors:?}");
        }
    }
}

#[test]
fn the_vectors_hold_for_the_reference_converter() {
    let file = vectors();
    let errors: Vec<String> = schema()
        .iter_errors(&file["typeDefs"])
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let defs = Converter::new(&file["typeDefs"]);
    for v in file["vectors"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        for d in v["decode"].as_array().unwrap() {
            let got = defs.decode(&v["type"], &d["json"]);
            if d.get("error") == Some(&json!(true)) {
                assert!(got.is_err(), "{name}: {} decoded to {got:?}", d["json"]);
            } else {
                assert_eq!(got.as_ref().ok(), Some(&d["value"]), "{name}: {got:?}");
            }
        }
        for e in v["encode"].as_array().unwrap() {
            assert_eq!(defs.encode(&v["type"], &e["value"]), e["json"], "{name}");
        }
    }
}

#[test]
fn the_vectors_cover_every_construct() {
    let file = vectors();
    let mut seen = BTreeSet::new();
    fn walk(v: &Value, seen: &mut BTreeSet<String>) {
        match v {
            Value::String(s) => {
                seen.insert(s.clone());
            }
            Value::Object(o) => {
                if let Some(k) = o.get("kind").and_then(Value::as_str) {
                    seen.insert(k.to_owned());
                }
                for (k, x) in o {
                    seen.insert(k.clone());
                    walk(x, seen);
                }
            }
            Value::Array(a) => a.iter().for_each(|x| walk(x, seen)),
            _ => {}
        }
    }
    walk(&file["typeDefs"], &mut seen);
    for c in [
        "record",
        "enum",
        "taggedUnion",
        "untaggedUnion",
        "alias",
        "base",
        "additional",
        "default",
        "any",
        "never",
        "null",
        "bool",
        "scalar",
        "format",
        "list",
        "map",
        "tuple",
        "rest",
        "ref",
        "nullable",
        "external",
        "literal",
        "union",
        "required",
        "optional",
        "nullable",
        "optionalNullable",
    ] {
        assert!(seen.contains(c), "no vector uses `{c}`");
    }
}
