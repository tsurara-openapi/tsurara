//! Every OperationDef the TypeScript backend emits validates against `spec/operation-def.schema.json`
//! (design.md 18.9). The generated files are imported with Node type stripping to get the
//! OperationDefs as JSON; skipped without Node 22.18 or later unless `TSURARA_REQUIRE_NODE=1`.

use std::path::Path;
use std::process::Command;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_lang_typescript::options::ImportExtension;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::repo_root;

const DUMP: &str = r#"import * as all from './gen/index.ts';
const defs = Object.entries(all)
  .filter(([name, value]) => name.endsWith('Def') && typeof value === 'object')
  .map(([name, value]) => ({ name, value }));
console.log(JSON.stringify(defs));
"#;

/// The OperationDef schema, with `type-def.schema.json` (which it refers to) registered locally.
fn validator(schema: &serde_json::Value) -> jsonschema::Validator {
    let type_defs: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/type-def.schema.json")).unwrap(),
    )
    .unwrap();
    let registry = jsonschema::Registry::new()
        .add("https://raw.githubusercontent.com/tsurara-openapi/tsurara/main/spec/type-def.schema.json", type_defs)
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(schema)
        .unwrap()
}

fn node_available() -> bool {
    let ok = Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| {
            let v = String::from_utf8_lossy(&o.stdout)
                .trim()
                .trim_start_matches('v')
                .to_owned();
            let mut p = v.split('.');
            Some((
                p.next()?.parse::<u32>().ok()?,
                p.next()?.parse::<u32>().ok()?,
            ))
        })
        .is_some_and(|v| v >= (22, 18));
    if !ok && std::env::var_os("TSURARA_REQUIRE_NODE").is_some() {
        panic!("Node 22.18 or later is required");
    }
    ok
}

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    tsurara_loader::normalize_swagger2(&mut set).unwrap();
    set
}

#[test]
fn emitted_operation_defs_validate_against_the_schema() {
    if !node_available() {
        return;
    }
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/operation-def.schema.json")).unwrap(),
    )
    .unwrap();
    let validator = validator(&schema);
    let opts = TypeScriptOptions {
        import_extension: ImportExtension::Ts,
        ..TypeScriptOptions::default()
    };
    let mut checked = 0;
    for name in [
        "kitchen-sink.yaml",
        "petstore3.json",
        "v3.0-petstore-expanded.yaml",
        "v3.0-petstore.yaml",
    ] {
        let dir = tempfile::tempdir().unwrap();
        for f in generate(&load(name), &Options::default(), &opts)
            .unwrap()
            .files
        {
            let path = dir.path().join("gen").join(&f.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, f.contents).unwrap();
        }
        std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
        std::fs::write(dir.path().join("dump.ts"), DUMP).unwrap();
        let out = Command::new("node")
            .arg("dump.ts")
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let defs: Vec<serde_json::Value> = serde_json::from_slice(&out.stdout).unwrap();
        assert!(!defs.is_empty(), "{name}");
        // The shared builder that other backends print (`tsurara_backend::operation_def`) agrees with
        // the TypeScript literals.
        let model = tsurara_sem::build_model(&load(name), &Options::default()).unwrap();
        let (named, _) =
            tsurara_lang_typescript::lower::lower_model(&model, &Options::default(), &opts);
        for op in &named.operations {
            let def_name = format!("{}Def", op.name);
            let emitted = defs
                .iter()
                .find(|d| d["name"] == def_name.as_str())
                .unwrap_or_else(|| panic!("{name}: no {def_name}"));
            assert_eq!(
                emitted["value"],
                tsurara_backend::operation_def::operation_def(op, None),
                "{name} {def_name}"
            );
        }
        for def in defs {
            let errors: Vec<String> = validator
                .iter_errors(&def["value"])
                .map(|e| format!("{} at {}", e, e.instance_path()))
                .collect();
            assert!(errors.is_empty(), "{name} {}: {errors:?}", def["name"]);
            checked += 1;
        }
    }
    assert!(checked > 10);
}

#[test]
fn the_schema_rejects_an_invalid_operation_def() {
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/operation-def.schema.json")).unwrap(),
    )
    .unwrap();
    let validator = validator(&schema);
    let bad = serde_json::json!({
        "v": 1, "id": "x", "method": "GET", "path": "/x",
        "responses": { "200": { "parseAs": "xml" } }
    });
    assert!(!validator.is_valid(&bad));
}
