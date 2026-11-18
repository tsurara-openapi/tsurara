//! Sample values (design.md 18.7.1).
//!
//! - `tests/cases/*.yaml`: the rules, as schemas with the expected sample (format in the file).
//! - Every row of the mapping table (`tsurara-lang-typescript/tests/cases/table.yaml`) and every
//!   declaration of the edge-case corpus written for OpenAPI 3.1 gets a sample that validates against
//!   its schema, unless a warning says why not.
//! - Two runs give identical samples.

use serde_json::{Value, json};
use std::path::Path;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sample::{Samples, samples};
use tsurara_sem::{Model, Options, build_model};
use tsurara_testkit::repo_root;

fn definition(schemas: Value, openapi: &str) -> Value {
    json!({
        "openapi": openapi,
        "info": { "title": "t", "version": "1" },
        "paths": {},
        "components": { "schemas": schemas },
    })
}

fn model_of(doc: &Value) -> Model {
    let set = DocumentSet::from_value(doc.clone());
    build_model(&set, &Options::default()).unwrap()
}

/// The sample of the (readable) declaration whose preferred name is `name`.
fn sample_of<'a>(model: &Model, s: &'a Samples, name: &str) -> Option<&'a Value> {
    let decl = model
        .schemas
        .iter()
        .find(|d| d.preferred_name == name && !d.writable)?;
    s.types
        .iter()
        .find(|(id, _)| *id == decl.id)
        .map(|(_, v)| v)
}

#[test]
fn cases() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cases");
    let mut failures = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let cases = parse_document(&std::fs::read(&path).unwrap()).unwrap();
        for case in cases.as_array().unwrap() {
            let name = case["name"].as_str().unwrap();
            let schemas = match (case.get("schema"), case.get("schemas")) {
                (Some(s), _) => json!({ "T": s }),
                (_, Some(s)) => s.clone(),
                _ => panic!("{name}: no schema"),
            };
            let model = model_of(&definition(schemas, "3.1.0"));
            let (s, warnings) = samples(&model);
            let expect = match case.get("schema") {
                Some(_) => json!({ "T": case["expect"] }),
                None => case["expect"].clone(),
            };
            for (type_name, want) in expect.as_object().unwrap() {
                let got = sample_of(&model, &s, type_name);
                if got != Some(want) {
                    failures.push_str(&format!(
                        "{name}: {type_name}: expected {want}, got {}\n",
                        got.map_or("nothing".to_owned(), Value::to_string)
                    ));
                }
            }
            let mut codes: Vec<&str> = warnings.iter().map(|w| w.kind.as_str()).collect();
            codes.sort();
            codes.dedup();
            let want: Vec<&str> = case
                .get("warnings")
                .and_then(Value::as_array)
                .map(|w| w.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            if codes != want {
                failures.push_str(&format!("{name}: warnings {codes:?}, expected {want:?}\n"));
            }
        }
    }
    assert!(failures.is_empty(), "\n{failures}");
}

/// Validates `value` against `#/components/schemas/{name}` of `doc` (OpenAPI 3.1 schemas are JSON
/// Schema 2020-12).
/// OpenAPI semantics the samples follow and JSON Schema does not know (design.md 18.7.1): `oneOf` is not
/// kept exclusive (validated as `anyOf`), and response samples have no writeOnly properties.
fn discriminated_any_of(v: &mut Value) {
    let write_only: Vec<String> = v["components"]["schemas"]
        .as_object()
        .map(|m| {
            m.iter()
                .filter(|(_, s)| s.get("writeOnly") == Some(&Value::Bool(true)))
                .map(|(k, _)| format!("#/components/schemas/{k}"))
                .collect()
        })
        .unwrap_or_default();
    openapi_semantics(v, &write_only);
}

fn openapi_semantics(v: &mut Value, write_only_refs: &[String]) {
    match v {
        Value::Object(o) => {
            if let Some(members) = o.remove("oneOf") {
                o.insert("anyOf".into(), members);
            }
            // Response samples leave writeOnly properties out; OpenAPI requires them in requests only.
            let write_only: Vec<String> = o
                .get("properties")
                .and_then(Value::as_object)
                .map(|ps| {
                    ps.iter()
                        .filter(|(_, p)| {
                            p.get("writeOnly") == Some(&Value::Bool(true))
                                || p.get("$ref")
                                    .and_then(Value::as_str)
                                    .is_some_and(|r| write_only_refs.iter().any(|w| w == r))
                        })
                        .map(|(k, _)| k.clone())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(Value::Array(required)) = o.get_mut("required") {
                required.retain(|r| {
                    !r.as_str()
                        .is_some_and(|r| write_only.iter().any(|w| w == r))
                });
            }
            o.values_mut()
                .for_each(|v| openapi_semantics(v, write_only_refs));
        }
        Value::Array(a) => a
            .iter_mut()
            .for_each(|v| openapi_semantics(v, write_only_refs)),
        _ => {}
    }
}

/// A JSON Pointer token as a URI fragment.
fn fragment(token: &str) -> String {
    let escaped = token.replace('~', "~0").replace('/', "~1");
    let mut out = String::new();
    for b in escaped.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn validate(doc: &Value, name: &str, value: &Value) -> Result<(), String> {
    let mut root = doc.clone();
    discriminated_any_of(&mut root);
    root["$ref"] = json!(format!("#/components/schemas/{}", fragment(name)));
    root["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
    let validator = jsonschema::validator_for(&root).map_err(|e| e.to_string())?;
    let errors: Vec<String> = validator
        .iter_errors(value)
        .map(|e| format!("{e} at {}", e.instance_path()))
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

/// Checks every top-level declaration of `doc` (not hoisted ones): its sample validates, unless the
/// samples have a warning about it.
fn check_all(doc: &Value, what: &str, failures: &mut String) {
    let model = model_of(doc);
    let (s, warnings) = samples(&model);
    let Some(components) = doc["components"]["schemas"].as_object() else {
        return;
    };
    for name in components.keys() {
        let pointer = format!(
            "/components/schemas/{}",
            name.replace('~', "~0").replace('/', "~1")
        );
        if warnings.iter().any(|w| w.loc.pointer.starts_with(&pointer)) {
            continue;
        }
        match sample_of(&model, &s, name) {
            None if doc["components"]["schemas"][name] == json!(false) => {}
            None => failures.push_str(&format!("{what}: {name}: no sample\n")),
            Some(v) => {
                if let Err(e) = validate(doc, name, v) {
                    failures.push_str(&format!("{what}: {name}: {v} is invalid: {e}\n"));
                }
            }
        }
    }
}

#[test]
fn every_row_of_the_mapping_table_has_a_valid_sample() {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/cases/table.yaml");
    let cases = parse_document(&std::fs::read(path).unwrap()).unwrap();
    let mut failures = String::new();
    let mut count = 0;
    for case in cases.as_array().unwrap() {
        let openapi = case
            .get("openapi")
            .and_then(Value::as_str)
            .unwrap_or("3.1.0");
        if !openapi.starts_with("3.1") && !openapi.starts_with("3.2") {
            continue;
        }
        let schemas = match (case.get("schema"), case.get("schemas")) {
            (Some(s), _) => json!({ "T": s }),
            (_, Some(s)) => s.clone(),
            _ => continue,
        };
        // Schemas that allow no value have no sample.
        if case.get("expect") == Some(&json!("never")) {
            continue;
        }
        count += 1;
        check_all(
            &definition(schemas, openapi),
            case["name"].as_str().unwrap(),
            &mut failures,
        );
    }
    assert!(count > 30, "only {count} rows");
    assert!(failures.is_empty(), "\n{failures}");
}

#[test]
fn corpus_samples_are_valid() {
    let dir = repo_root().join("corpus/edge-cases");
    let mut failures = String::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let doc = parse_document(&std::fs::read(&path).unwrap()).unwrap();
        let version = doc["openapi"].as_str().unwrap_or("");
        if version.starts_with("3.1") || version.starts_with("3.2") {
            check_all(
                &doc,
                &path.file_name().unwrap().to_string_lossy(),
                &mut failures,
            );
        }
    }
    assert!(failures.is_empty(), "\n{failures}");
}

#[test]
fn samples_are_deterministic() {
    let doc =
        parse_document(&std::fs::read(repo_root().join("corpus/edge-cases/names.yaml")).unwrap())
            .unwrap();
    let a = serde_json::to_string(&samples(&model_of(&doc)).0).unwrap();
    let b = serde_json::to_string(&samples(&model_of(&doc)).0).unwrap();
    assert_eq!(a, b);
}

#[test]
fn operations_get_parameter_body_and_response_samples() {
    let doc = parse_document(
        &std::fs::read(
            repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml"),
        )
        .unwrap(),
    )
    .unwrap();
    let model = model_of(&doc);
    let (s, _) = samples(&model);
    let show = s
        .operations
        .iter()
        .find(|o| o.operation_id == "showPetById")
        .unwrap();
    assert_eq!(show.params[0].name, "petId");
    assert_eq!(show.params[0].value, json!("petId"));
    let ok = show.responses.iter().find(|r| r.status == "200").unwrap();
    assert_eq!(ok.media_type, "application/json");
    assert_eq!(ok.value, json!({ "id": 1, "name": "name", "tag": "tag" }));
    let create = s
        .operations
        .iter()
        .find(|o| o.operation_id == "createPets")
        .unwrap();
    let body = create.body.as_ref().unwrap();
    assert_eq!(body.media_type, "application/json");
    assert_eq!(body.value, json!({ "id": 1, "name": "name", "tag": "tag" }));
}
