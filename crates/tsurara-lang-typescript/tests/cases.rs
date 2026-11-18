//! Runs pairs of small definitions and expected outputs listed in `tests/cases/*.yaml`.
//!
//! Each file is an array of cases, and a case has the following keys.
//!
//! - `name`: name of the case
//! - `openapi`: definition version (default `3.1.0`)
//! - `schema`: a single schema, placed in `components.schemas` as `T`
//! - `schemas`: several schemas (instead of `schema`)
//! - `expect`: the expected TypeScript type. A string for `schema`, a map from name to type for
//!   `schemas`. It can also be a map from language to that (`expect: { typescript: ... }`)
//! - `semantic`: the expected model type in `tsurara_sem::notation` form, before any language's lowering
//!   (a string for `schema`, a map from preferred name to notation for `schemas`). It is checked
//!   once for all languages (design.md 18.12)
//! - `warnings`: the expected warning kinds and JSON Pointers (`unsupported #/components/schemas/T/not`).
//!   When present, the list must match exactly; `[]` checks that there are no warnings
//! - `options`: `splitVisibility` (boolean)
//! - `accept`: values the definition allows; each must be assignable to the generated type
//! - `reject`: values the definition does not allow; each must be a type error
//! - `approx`: values the definition does not allow but the type accepts, because the constraint is
//!   approximated (design.md 6.4.1); each must be assignable
//!
//! For `schema`, the three are lists of JSON values for `T`. For `schemas`, they are maps from type
//! name to a list. The values are emitted as `export const` assignments and checked by `tsc`, with
//! `// @ts-expect-error` before rejected values, so a type that accepts a rejected value fails with an
//! unused-directive error.

use serde_json::{Value, json};
use std::fmt::Write;
use std::path::Path;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::lower::lower_model;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::notation::notation;
use tsurara_sem::print::{print_type, print_type_alias};
use tsurara_sem::{Options, build_model};

#[test]
fn cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    entries.sort();
    let mut failures = String::new();
    let mut count = 0;
    let mut ts_files = vec![(
        "brand.ts".to_owned(),
        "export type Uuid = string & { readonly __brand: 'Uuid' };\n".to_owned(),
    )];
    for path in entries {
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        let cases = parse_document(&std::fs::read(&path).unwrap()).unwrap();
        for case in cases.as_array().expect("a file is an array of cases") {
            count += 1;
            let name = case["name"].as_str().unwrap_or("?");
            match run_case(case) {
                Ok(ts) => ts_files.push((
                    format!("case{count}.ts"),
                    format!("// {file}: {name}\n{ts}"),
                )),
                Err(msg) => writeln!(failures, "--- {file}: {name}\n{msg}").unwrap(),
            }
        }
    }
    assert!(failures.is_empty(), "case failures:\n{failures}");
    assert!(count > 0);
    // Checks that all generated types pass tsc (no circular reference errors).
    tsurara_testkit::assert_tsc_passes(&ts_files, &tsurara_testkit::TsOptions::default());
}

/// Languages whose output a case can expect.
const LANGUAGES: &[&str] = &["typescript"];

/// Runs a case and returns the generated types as the contents of a TypeScript file.
fn run_case(case: &Value) -> Result<String, String> {
    let version = case
        .get("openapi")
        .and_then(Value::as_str)
        .unwrap_or("3.1.0");
    // `expect` is the TypeScript output, or a map from language to output (`{ typescript: ... }`).
    let expect = match &case["expect"] {
        Value::Object(m) if m.keys().all(|k| LANGUAGES.contains(&k.as_str())) && !m.is_empty() => {
            m.get("typescript").cloned().unwrap_or(Value::Null)
        }
        other => other.clone(),
    };
    let (schemas, expect, semantic) = match (case.get("schema"), case.get("schemas")) {
        (Some(s), None) => (
            json!({ "T": s }),
            json!({ "T": expect }),
            case.get("semantic").map(|v| json!({ "T": v })),
        ),
        (None, Some(s)) => (s.clone(), expect, case.get("semantic").cloned()),
        _ => return Err("a case needs either `schema` or `schemas`".into()),
    };
    let doc = json!({
        "openapi": version,
        "info": { "title": "cases", "version": "1" },
        "paths": {},
        "components": { "schemas": schemas }
    });
    let mut opts = Options::default();
    if let Some(o) = case.get("options") {
        if let Some(b) = o.get("splitVisibility").and_then(Value::as_bool) {
            opts.split_visibility = b;
        }
    }
    let set = DocumentSet::from_value(doc);
    // Printed as TypeScript prints it: after TypeScript's naming and lowering, like `emit`.
    let model = build_model(&set, &opts).map_err(|e| e.to_string())?;
    let mut msg = String::new();
    // The semantic expectation is checked on the model, before any language: once for all languages.
    if let Some(Value::Object(semantic)) = &semantic {
        for (name, want) in semantic {
            let want = want.as_str().ok_or("semantic types must be strings")?;
            match model
                .schemas
                .iter()
                .find(|d| &d.preferred_name == name && !d.writable)
            {
                Some(d) => {
                    let got = notation(&d.ty);
                    if got != want {
                        writeln!(
                            msg,
                            "{name} (semantic):\n  expected: {want}\n  actual:   {got}"
                        )
                        .unwrap();
                    }
                }
                None => writeln!(msg, "{name} (semantic): not in the model").unwrap(),
            }
        }
    }
    let (mut model, backend_warnings) = lower_model(&model, &opts, &TypeScriptOptions::default());
    model.warnings.extend(backend_warnings);
    let expect = expect.as_object().ok_or("`expect` must be a map")?;
    for (name, want) in expect {
        let want = want
            .as_str()
            .ok_or("expected types must be strings")?
            .trim_end();
        match model.schemas.iter().find(|d| &d.name == name) {
            Some(decl) => {
                let got = print_type(&decl.ty);
                if got != want {
                    writeln!(msg, "{name}:\n  expected: {want}\n  actual:   {got}").unwrap();
                }
            }
            None => {
                let names: Vec<_> = model.schemas.iter().map(|d| d.name.as_str()).collect();
                writeln!(msg, "{name}: not generated (generated: {names:?})").unwrap();
            }
        }
    }
    if let Some(Value::Array(want)) = case.get("warnings") {
        let got: Vec<String> = model
            .warnings
            .iter()
            .map(|w| format!("{} #{}", w.kind.as_str(), w.loc.pointer))
            .collect();
        let want: Vec<&str> = want.iter().filter_map(Value::as_str).collect();
        for w in &want {
            if !got.iter().any(|g| g == w) {
                writeln!(msg, "missing warning `{w}` (got {got:?})").unwrap();
            }
        }
        for g in &got {
            if !want.contains(&g.as_str()) {
                writeln!(msg, "unexpected warning `{g}`").unwrap();
            }
        }
    }
    if !msg.is_empty() {
        return Err(msg);
    }
    let mut ts = String::from("import type { Uuid } from './brand.ts';\nexport type { Uuid };\n");
    for decl in &model.schemas {
        ts.push_str(&print_type_alias(&decl.name, &decl.ty, decl.doc.as_ref()));
    }
    ts.push_str(&assignments(case)?);
    Ok(ts)
}

/// The `accept`, `approx` and `reject` values of a case as TypeScript assignments.
fn assignments(case: &Value) -> Result<String, String> {
    let single = case.get("schema").is_some();
    let mut out = String::new();
    let mut n = 0;
    for (key, directive) in [
        ("accept", None),
        (
            "approx",
            Some("// Accepted as an approximation (design.md 6.4.1)."),
        ),
        ("reject", Some("// @ts-expect-error")),
    ] {
        let Some(v) = case.get(key) else { continue };
        let lists: Vec<(String, &Value)> = if single {
            vec![("T".to_owned(), v)]
        } else {
            v.as_object()
                .ok_or(format!("`{key}` must be a map from type name to values"))?
                .iter()
                .map(|(k, v)| (k.clone(), v))
                .collect()
        };
        for (ty, values) in lists {
            let values = values
                .as_array()
                .ok_or(format!("`{key}` values must be a list"))?;
            for value in values {
                n += 1;
                if let Some(d) = directive {
                    writeln!(out, "{d}").unwrap();
                }
                writeln!(out, "export const {key}{n}: {ty} = {value};").unwrap();
            }
        }
    }
    Ok(out)
}
