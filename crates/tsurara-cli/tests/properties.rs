//! Property-based tests (design.md chapter 13): definitions built from random schemas
//! (compositions, nullable values, references with cycles, unusual names, and text that ends comments
//! and literals in paths, media types, channel names and descriptions) generate code that passes
//! `tsc` (with the strictest tsconfig) and pyright.
//!
//! All cases are checked in one run of each checker. A failing case is shrunk with proptest, one
//! checker run per step, and the smallest definition is saved in `target/proptest-failures` to be
//! added to `corpus/edge-cases`. `TSURARA_PROPTEST_CASES` sets the number of cases (default 48) and
//! `TSURARA_PROPTEST_SEED` the seed (default fixed, so CI is deterministic).

use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::{Config, RngAlgorithm, TestRng, TestRunner};
use serde_json::{Map, Value, json};
use tsurara_loader::DocumentSet;
use tsurara_sem::Options;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, repo_root,
};

/// Names that stress naming: keywords of both languages, globals, punctuation, digits, non-ASCII.
const NAMES: &[&str] = &[
    "id",
    "name",
    "class",
    "type",
    "default",
    "constructor",
    "__proto__",
    "None",
    "self",
    "import",
    "a-b",
    "a b",
    "1st",
    "$ref",
    "日本語",
    "camelCase",
    "snake_case",
    "Pet",
    "pet",
    "Error",
    "Date",
    "Promise",
    "Any",
    "File",
    "x",
];

fn name() -> impl Strategy<Value = String> {
    prop_oneof![
        6 => proptest::sample::select(NAMES).prop_map(str::to_owned),
        1 => text(FRAGMENTS),
    ]
}

/// Pieces of text that end comments, docstrings, strings or template literals in some generated
/// language, with every line break of the target languages.
const FRAGMENTS: &[&str] = &[
    "a", " ", "*/", "/*", "\"\"\"", "'", "\"", "`", "\\", "${", "}", "#", "\r", "\n", "\r\n",
    "\u{2028}", "\u{2029}", "\u{85}", "\u{c}", "\t",
];

/// The fragments that can be part of a path segment without adding a template variable.
const PATH_FRAGMENTS: &[&str] = &[
    "a", " ", "*/", "/*", "\"\"\"", "'", "\"", "`", "\\", "$", "#", "\r", "\n", "\u{2028}",
];

/// Text made of the given fragments.
fn text(fragments: &'static [&'static str]) -> impl Strategy<Value = String> {
    proptest::collection::vec(proptest::sample::select(fragments), 1..6).prop_map(|v| v.concat())
}

/// A schema of the given depth that may refer to the components `S0`..`S{refs}`.
fn schema(depth: u32, refs: usize) -> BoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(json!({ "type": "string" })),
        Just(json!({ "type": "string", "format": "date-time" })),
        Just(json!({ "type": "integer", "format": "int64" })),
        Just(json!({ "type": "number" })),
        Just(json!({ "type": "boolean" })),
        Just(json!({ "type": "null" })),
        Just(json!({})),
        proptest::collection::vec(name(), 1..4)
            .prop_map(|v| json!({ "type": "string", "enum": v })),
        (0i64..3).prop_map(|n| json!({ "const": n })),
        (0..refs.max(1)).prop_map(|i| json!({ "$ref": format!("#/components/schemas/S{i}") })),
        Just(json!({ "type": ["string", "null"] })),
        Just(json!({ "type": "string", "nullable": true })),
        Just(json!({ "type": "integer", "readOnly": true })),
        Just(json!({ "type": "string", "writeOnly": true })),
    ];
    if depth == 0 {
        return leaf.boxed();
    }
    let inner = schema(depth - 1, refs);
    prop_oneof![
        3 => leaf,
        2 => (proptest::collection::btree_map(name(), inner.clone(), 0..4), any::<u8>(), any::<bool>())
            .prop_map(|(props, required, extra)| {
                let keys: Vec<String> = props.keys().cloned().collect();
                let required: Vec<&String> =
                    keys.iter().enumerate().filter(|(i, _)| required & (1 << i) != 0).map(|(_, k)| k).collect();
                let mut o = json!({ "type": "object", "properties": props, "required": required });
                if extra {
                    o["additionalProperties"] = json!({ "type": "string" });
                }
                o
            }),
        1 => inner.clone().prop_map(|i| json!({ "type": "array", "items": i })),
        1 => proptest::collection::vec(inner.clone(), 1..4).prop_map(|v| json!({ "oneOf": v })),
        1 => proptest::collection::vec(inner.clone(), 1..4).prop_map(|v| json!({ "anyOf": v })),
        1 => proptest::collection::vec(inner.clone(), 1..3).prop_map(|v| json!({ "allOf": v })),
        1 => inner.clone().prop_map(|i| json!({ "anyOf": [i, { "type": "null" }] })),
        1 => (name(), proptest::collection::vec(name(), 1..3)).prop_map(|(prop, tags)| {
            // A discriminated union whose members carry the tag.
            let members: Vec<Value> = tags
                .iter()
                .map(|t| json!({ "type": "object", "required": [prop.clone()], "properties": { prop.clone(): { "const": t } } }))
                .collect();
            json!({ "oneOf": members, "discriminator": { "propertyName": prop } })
        }),
    ]
    .boxed()
}

/// A definition with `n` components, used by an operation's body, response and parameter.
fn document() -> impl Strategy<Value = Value> {
    (1usize..5).prop_flat_map(|n| {
        (
            proptest::collection::vec(schema(3, n), n),
            schema(1, n),
            any::<bool>(),
            (
                text(FRAGMENTS),
                text(PATH_FRAGMENTS),
                text(PATH_FRAGMENTS),
                text(FRAGMENTS),
            ),
        )
            .prop_map(move |(components, param, v30, (description, path, media, channel))| {
                let mut schemas = Map::new();
                for (i, mut s) in components.into_iter().enumerate() {
                    if i == 0 {
                        if let Value::Object(o) = &mut s {
                            o.insert("description".into(), json!(description));
                        }
                    }
                    schemas.insert(format!("S{i}"), s);
                }
                let mut doc = json!({
                    "openapi": "3.1.0",
                    "info": { "title": "t", "version": "1" },
                    "paths": { format!("/items/{{id}}/{path}"): { "post": {
                        "operationId": "postItem",
                        "description": description,
                        "parameters": [
                            { "name": "id", "in": "path", "required": true, "schema": { "type": "string" } },
                            { "name": "q", "in": "query", "schema": param }
                        ],
                        "requestBody": { "content": {
                            "application/json": { "schema": { "$ref": "#/components/schemas/S0" } },
                            format!("text/x-{media}"): { "schema": { "type": "string" } }
                        } },
                        "responses": {
                            "200": { "description": "ok", "content": { "application/json": { "schema": { "$ref": format!("#/components/schemas/S{}", n - 1) } } } },
                            "default": { "description": "error", "content": { "application/json": { "schema": { "type": "object", "properties": { "message": { "type": "string" } } } } } }
                        }
                    } } },
                    "x-event-channels": {
                        channel: { "content": { "application/json": { "schema": { "$ref": "#/components/schemas/S0" } } } }
                    },
                    "components": { "schemas": schemas }
                });
                if v30 {
                    // OpenAPI 3.0 has no `type: null` and no `const`; the generator approximates them.
                    doc["openapi"] = json!("3.0.3");
                }
                doc
            })
    })
}

/// Generated files as paths and contents.
type Files = Vec<(String, String)>;

fn files(doc: &Value) -> Result<(Files, Files, Files, Files), String> {
    let set = DocumentSet::from_value(doc.clone());
    let ts = tsurara_lang_typescript::generate(&set, &Options::default(), &Default::default())
        .map_err(|e| format!("typescript: {e}"))?;
    let py = tsurara_lang_python::generate(&set, &Options::default(), &Default::default())
        .map_err(|e| format!("python: {e}"))?;
    let rs = tsurara_lang_rust::generate(&set, &Options::default(), &Default::default())
        .map_err(|e| format!("rust: {e}"))?;
    let go = tsurara_lang_go::generate(&set, &Options::default(), &Default::default())
        .map_err(|e| format!("go: {e}"))?;
    let pairs =
        |o: tsurara_backend::Output| o.files.into_iter().map(|f| (f.path, f.contents)).collect();
    Ok((pairs(ts), pairs(py), pairs(rs), pairs(go)))
}

/// Checks cases together: each in its own directory, one checker run per language. Returns the
/// indices of the cases that fail, with the reasons.
fn check(docs: &[Value]) -> Vec<(usize, String)> {
    let mut failures = Vec::new();
    let mut ts_all = Vec::new();
    let mut py_all = Vec::new();
    let mut rs_all = Vec::new();
    let mut rs_modules = String::new();
    let mut go_all = Vec::new();
    for (i, doc) in docs.iter().enumerate() {
        match std::panic::catch_unwind(|| files(doc)) {
            Ok(Ok((ts, py, rs, go))) => {
                go_all.extend(go.into_iter().map(|(p, c)| (format!("c{i}/{p}"), c)));
                ts_all.extend(ts.into_iter().map(|(p, c)| (format!("c{i}/{p}"), c)));
                py_all.extend(py.into_iter().map(|(p, c)| (format!("c{i}/{p}"), c)));
                rs_all.extend(rs.into_iter().map(|(p, c)| (format!("c{i}/{p}"), c)));
                rs_modules.push_str(&format!("pub mod c{i};\n"));
            }
            Ok(Err(e)) => failures.push((i, e)),
            Err(_) => failures.push((i, "the generator panicked".into())),
        }
    }
    let blame = |output: &str, failures: &mut Vec<(usize, String)>| {
        for (i, _) in docs.iter().enumerate() {
            let marker = format!("c{i}/");
            let lines: Vec<&str> = output.lines().filter(|l| l.contains(&marker)).collect();
            if !lines.is_empty() && !failures.iter().any(|(j, _)| *j == i) {
                failures.push((i, lines.join("\n")));
            }
        }
    };
    if let Err(e) = (TypeScriptChecker {
        options: TsOptions::default(),
    })
    .check(&ts_all)
    {
        blame(&e, &mut failures);
    }
    if let Err(e) = (PythonChecker {
        package: "pkg".into(),
    })
    .check(&py_all)
    {
        blame(&e, &mut failures);
    }
    rs_all.push(("mod.rs".into(), rs_modules));
    if let Err(e) = RustChecker::default().check(&rs_all) {
        blame(&e, &mut failures);
    }
    if let Err(e) = GoChecker::default().check(&go_all) {
        blame(&e, &mut failures);
    }
    failures
}

#[test]
fn random_definitions_generate_code_that_type_checks() {
    let cases: usize = std::env::var("TSURARA_PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48);
    let seed: u64 = std::env::var("TSURARA_PROPTEST_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2026);
    let mut seed_bytes = [0u8; 32];
    seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut runner = TestRunner::new_with_rng(
        Config::default(),
        TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes),
    );
    let strategy = document();
    let mut trees: Vec<_> = (0..cases)
        .map(|_| strategy.new_tree(&mut runner).expect("a case"))
        .collect();
    let docs: Vec<Value> = trees.iter().map(|t| t.current()).collect();
    let failures = check(&docs);
    let Some((first, reason)) = failures.first().cloned() else {
        return;
    };
    // Shrink the first failure: simplify while it still fails, complicate when it stops failing.
    let tree = &mut trees[first];
    let mut last = (tree.current(), reason);
    for _ in 0..256 {
        if !tree.simplify() {
            break;
        }
        loop {
            match check(&[tree.current()]).into_iter().next() {
                Some((_, why)) => {
                    last = (tree.current(), why);
                    break;
                }
                None => {
                    if !tree.complicate() {
                        break;
                    }
                }
            }
        }
    }
    let dir = repo_root().join("target/proptest-failures");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("seed-{seed}-case-{first}.json"));
    std::fs::write(&path, serde_json::to_string_pretty(&last.0).unwrap()).unwrap();
    panic!(
        "{} of {cases} cases fail; the smallest found is saved in {}:\n{}\n\n{}",
        failures.len(),
        path.display(),
        serde_json::to_string_pretty(&last.0).unwrap(),
        last.1
    );
}

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

/// Samples of random OpenAPI 3.1 definitions without `allOf` validate against their
/// schemas, except where the samples report why not (`sample-cycle`, `sample-pattern`).
#[test]
fn random_definitions_have_valid_samples() {
    let cases: u32 = std::env::var("TSURARA_PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(48)
        .max(256);
    let seed: u64 = std::env::var("TSURARA_PROPTEST_SEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(2026);
    let mut seed_bytes = [0u8; 32];
    seed_bytes[..8].copy_from_slice(&seed.to_le_bytes());
    let mut runner = TestRunner::new_with_rng(
        Config {
            cases,
            failure_persistence: None,
            ..Config::default()
        },
        TestRng::from_seed(RngAlgorithm::ChaCha, &seed_bytes),
    );
    runner
        .run(&document(), |doc| {
            // Random `allOf` compositions are mostly contradictory (members that give a property
            // different types); samples do not solve them (design.md 18.7.1). The mapping table and the
            // corpus cover `allOf`.
            if doc["openapi"] != json!("3.1.0") || doc.to_string().contains("\"allOf\"") {
                return Ok(());
            }
            let set = DocumentSet::from_value(doc.clone());
            let model = tsurara_sem::build_model(&set, &Options::default()).unwrap();
            let (samples, warnings) = tsurara_sample::samples(&model);
            let mut root = doc.clone();
            discriminated_any_of(&mut root);
            root["$schema"] = json!("https://json-schema.org/draft/2020-12/schema");
            // A required cycle has no finite value; its samples stop with `{}` and a warning.
            if !warnings.is_empty() {
                return Ok(());
            }
            for (name, _) in doc["components"]["schemas"].as_object().unwrap() {
                let pointer = format!("/components/schemas/{name}");
                let Some(decl) = model
                    .schemas
                    .iter()
                    .find(|d| d.preferred_name == *name && !d.writable)
                else {
                    continue;
                };
                let Some((_, value)) = samples.types.iter().find(|(id, _)| *id == decl.id) else {
                    continue;
                };
                root["$ref"] = json!(format!("#{pointer}"));
                let validator = jsonschema::validator_for(&root).unwrap();
                let errors: Vec<String> = validator
                    .iter_errors(value)
                    .map(|e| e.to_string())
                    .collect();
                prop_assert!(errors.is_empty(), "{name}: {value} is invalid: {errors:?}");
            }
            Ok(())
        })
        .unwrap();
}
