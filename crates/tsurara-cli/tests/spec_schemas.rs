//! The JSON Schemas of the model, the manifest and the protocol in `spec/`: they match
//! the Rust types, and the models and manifests of the edge-case corpus validate against them.

use serde_json::Value;
use tsurara_backend::json_schemas;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, build_model};
use tsurara_testkit::repo_root;

#[test]
fn spec_schemas_are_up_to_date() {
    let mut stale = Vec::new();
    let mut schemas = json_schemas();
    schemas.push(("cli-output.schema.json", tsurara_cli::output::json_schema()));
    for (name, want) in schemas {
        let path = repo_root().join("spec").join(name);
        if std::env::var_os("TSURARA_UPDATE_SCHEMA").is_some() {
            std::fs::write(&path, &want).unwrap();
            continue;
        }
        if std::fs::read_to_string(&path).unwrap_or_default() != want {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "spec/{stale:?} out of date; run `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test spec_schemas`"
    );
}

fn validator(name: &str) -> jsonschema::Validator {
    let schema = json_schemas()
        .into_iter()
        .find(|(n, _)| *n == name)
        .unwrap()
        .1;
    jsonschema::validator_for(&serde_json::from_str(&schema).unwrap()).unwrap()
}

fn assert_valid(validator: &jsonschema::Validator, value: &Value, what: &str) {
    let errors: Vec<String> = validator
        .iter_errors(value)
        .take(5)
        .map(|e| format!("{} at {}", e, e.instance_path()))
        .collect();
    assert!(errors.is_empty(), "{what}: {errors:#?}");
}

#[test]
fn corpus_models_and_manifests_validate() {
    let model_schema = validator("sem-model.schema.json");
    let manifest_schema = validator("manifest.schema.json");
    let protocol = validator("plugin-protocol.schema.json");
    let dir = repo_root().join("corpus/edge-cases");
    let mut paths: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    paths.sort();
    for path in paths {
        let what = path.file_name().unwrap().to_string_lossy().into_owned();
        let mut set =
            DocumentSet::from_value(parse_document(&std::fs::read(&path).unwrap()).unwrap());
        tsurara_loader::normalize_swagger2(&mut set).unwrap();
        let opts = Options::default();
        let model = build_model(&set, &opts).unwrap();
        assert_valid(&model_schema, &serde_json::to_value(&model).unwrap(), &what);
        let backends: [&dyn tsurara_backend::Backend; 4] = [
            &tsurara_lang_typescript::TypeScript::default(),
            &tsurara_lang_python::Python::default(),
            &tsurara_lang_rust::Rust::default(),
            &tsurara_lang_go::Go::default(),
        ];
        for backend in backends {
            let out = tsurara_backend::generate_from_model(backend, &model, &opts).unwrap();
            let manifest = out.manifest.unwrap();
            let what = format!("{what} ({})", backend.id());
            assert_valid(
                &manifest_schema,
                &serde_json::to_value(&manifest).unwrap(),
                &what,
            );
            let input = tsurara_backend::plugin::ExtendInput {
                language: backend.id(),
                model: &model,
                manifest: Some(&manifest),
                common: &opts,
                options: &backend.options_json(),
                marker: "// marker",
            };
            assert_valid(&protocol, &serde_json::to_value(input).unwrap(), &what);
        }
    }
}

#[test]
fn the_model_schema_rejects_a_broken_model() {
    let set = DocumentSet::from_value(
        parse_document(b"openapi: 3.1.0\ninfo: { title: t, version: '1' }\npaths: {}\n").unwrap(),
    );
    let mut model = serde_json::to_value(build_model(&set, &Options::default()).unwrap()).unwrap();
    let schema = validator("sem-model.schema.json");
    assert!(schema.is_valid(&model));
    model.as_object_mut().unwrap().remove("operations");
    assert!(
        !schema.is_valid(&model),
        "a model without operations is invalid"
    );
    model["operations"] = serde_json::json!([{ "id": 1 }]);
    assert!(
        !schema.is_valid(&model),
        "an operation with a numeric id is invalid"
    );
}

/// FNV-1a, 64 bits: a stable fingerprint of a schema file for `spec/CHANGELOG.md`.
fn fingerprint(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Every schema version is recorded in the changelog (design.md 14): a change to a schema
/// needs an entry that names the schema with the fingerprint of its new contents.
#[test]
fn every_schema_change_is_in_the_changelog() {
    let changelog = std::fs::read_to_string(repo_root().join("spec/CHANGELOG.md")).unwrap();
    let missing: Vec<String> = json_schemas()
        .into_iter()
        .map(|(name, text)| format!("`{name}` {}", fingerprint(&text)))
        .filter(|line| !changelog.contains(line.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "spec/CHANGELOG.md has no entry for the current schemas; add an entry with:\n{}",
        missing.join("\n")
    );
}
