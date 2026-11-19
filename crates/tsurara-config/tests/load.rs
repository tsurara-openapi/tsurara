//! Loads configuration files from a temporary directory.

use serde_json::json;
use std::path::{Path, PathBuf};
use tsurara_config::{Command, Error, load_with_env};
use tsurara_config::{Language, Target};
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::options::{ImportExtension, Layout};
use tsurara_loader::DocumentSet;

fn write(dir: &Path, path: &str, contents: &str) -> PathBuf {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

fn ts(t: &Target) -> &TypeScriptOptions {
    match &t.language {
        Language::TypeScript(o) => o,
        _ => panic!("not a TypeScript target"),
    }
}

fn no_env(_: &str) -> Option<String> {
    None
}

/// The temporary directory with symlinks resolved (`/var` is a symlink to `/private/var` on macOS).
fn tempdir() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    (tmp, root)
}

#[test]
fn paths_in_extended_files_are_relative_to_that_file() {
    let (_tmp, root) = tempdir();
    write(
        &root,
        "tsurara.base.yaml",
        "input: specs/openapi.yaml\nhooks:\n  afterWrite: [biome, format, --write, \"{output}\"]\n",
    );
    let config = write(
        &root,
        "packages/openapi/tsurara.config.yaml",
        "extends: ../../tsurara.base.yaml\noutput: src/generated\n",
    );
    let config = load_with_env(&config, &no_env).unwrap();
    let [target] = config.targets.as_slice() else {
        panic!("expected one target");
    };
    assert_eq!(target.name, "default");
    assert_eq!(
        target.input.url.to_file_path().unwrap(),
        root.join("specs/openapi.yaml")
    );
    assert_eq!(target.output, root.join("packages/openapi/src/generated"));
    assert_eq!(
        target.hooks.after_write,
        vec![Command {
            args: vec![
                "biome".into(),
                "format".into(),
                "--write".into(),
                "{output}".into()
            ],
            dir: root.clone(),
        }]
    );
}

#[test]
fn relative_type_mapping_imports_are_relative_to_generated_files() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "packages/openapi/tsurara.config.yaml",
        r#"
input: openapi.yaml
output: src/generated
typescript:
  typeMappings:
    uuid: { type: Uuid, import: { from: ./src/brand.ts } }
    email: { type: Email, import: { from: "@acme/brand", name: EmailAddress } }
"#,
    );
    let config = load_with_env(&config, &no_env).unwrap();
    let opts = &config.targets[0].options;
    let uuid = opts.type_mappings["uuid"]["typescript"]
        .import
        .as_ref()
        .unwrap();
    assert_eq!(uuid.from, "../brand.ts");
    assert_eq!(uuid.name, "Uuid");
    let email = opts.type_mappings["email"]["typescript"]
        .import
        .as_ref()
        .unwrap();
    assert_eq!(email.from, "@acme/brand");
    assert_eq!(email.name, "EmailAddress");

    let set = DocumentSet::from_value(json!({
        "openapi": "3.1.0",
        "info": { "title": "t", "version": "1" },
        "paths": {},
        "components": { "schemas": { "Id": { "type": "string", "format": "uuid" } } }
    }));
    let out = tsurara_lang_typescript::generate(&set, opts, ts(&config.targets[0])).unwrap();
    let file = out
        .files
        .iter()
        .find(|f| f.contents.contains("export type Id"))
        .expect("the schema file is generated");
    let depth = file.path.matches('/').count();
    assert!(
        depth > 0,
        "schema files are in a subdirectory: {}",
        file.path
    );
    let want = format!(
        "import type {{ Uuid }} from '{}../brand.ts';",
        "../".repeat(depth)
    );
    assert!(
        file.contents.contains(&want),
        "{}:\n{}",
        file.path,
        file.contents
    );
}

#[test]
fn x_tsurara_type_imports_are_relative_to_the_config_file() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "pkg/tsurara.config.yaml",
        "input: openapi.yaml\noutput: src/generated\n",
    );
    let config = load_with_env(&config, &no_env).unwrap();
    assert_eq!(config.targets[0].options.external_import_base, "../..");
}

#[test]
fn top_level_entries_are_target_defaults() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.json",
        r#"{
          "input": "openapi.yaml",
          "typescript": { "layout": "tags" },
          "filter": { "tags": ["pet"], "excludeTags": ["internal"], "excludeOperationIds": ["debug"] },
          "targets": [
            { "name": "a", "output": "out/a" },
            { "name": "b", "output": "out/b", "typescript": { "layout": "single" }, "filter": { "operationIds": ["getPet"] } }
          ]
        }"#,
    );
    let config = load_with_env(&config, &no_env).unwrap();
    let a = config.target("a").unwrap();
    let b = config.target("b").unwrap();
    assert_eq!(ts(a).layout, Layout::Tags);
    assert_eq!(ts(b).layout, Layout::Single);
    assert_eq!(a.options.filter.tags, ["pet"]);
    assert_eq!(a.options.filter.exclude_tags, ["internal"]);
    assert_eq!(a.options.filter.exclude_operation_ids, ["debug"]);
    // Objects are deep-merged.
    assert_eq!(b.options.filter.tags, ["pet"]);
    assert_eq!(b.options.filter.operation_ids, ["getPet"]);
    assert_eq!(a.input, b.input);
}

#[test]
fn toml_and_input_objects_with_environment_variables() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.toml",
        r#"
output = "out"

[input]
url = "https://api.example.com/openapi.json"
headers = { Authorization = "Bearer ${API_TOKEN}" }
"#,
    );
    let env = |n: &str| (n == "API_TOKEN").then(|| "secret".to_owned());
    let config = load_with_env(&config, &env).unwrap();
    let input = &config.targets[0].input;
    assert_eq!(input.url.as_str(), "https://api.example.com/openapi.json");
    assert_eq!(
        input.headers,
        [("Authorization".to_owned(), "Bearer secret".to_owned())]
    );

    let err = load_with_env(&root.join("tsurara.config.toml"), &no_env).unwrap_err();
    assert!(
        matches!(&err, Error::Env { name, .. } if name == "API_TOKEN"),
        "{err}"
    );
}

#[test]
fn unknown_keys_are_errors_in_the_file_they_are_written_in() {
    let (_tmp, root) = tempdir();
    let base = write(&root, "base.yaml", "input: a.yaml\nouptut: out\n");
    let config = write(
        &root,
        "sub/tsurara.config.yaml",
        "extends: ../base.yaml\noutput: out\n",
    );
    let err = load_with_env(&config, &no_env).unwrap_err();
    match &err {
        Error::Invalid { path, message } => {
            assert_eq!(path, &base);
            assert!(message.contains("ouptut"), "{message}");
        }
        other => panic!("unexpected error: {other}"),
    }

    let config = write(
        &root,
        "targets.yaml",
        "targets:\n  - { input: a.yaml, output: out, nmae: x }\n",
    );
    let err = load_with_env(&config, &no_env).unwrap_err().to_string();
    assert!(err.contains("targets[0]") && err.contains("nmae"), "{err}");
}

#[test]
fn type_mappings_that_contradict_runtime_values_are_errors() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\ntypescript:\n  typeMappings:\n    date-time: Date\n",
    );
    let err = load_with_env(&config, &no_env).unwrap_err();
    assert!(
        matches!(&err, Error::TypeMapping { key, ty, .. } if key == "date-time" && ty == "Date"),
        "{err}"
    );
}

#[test]
fn extends_cycles_and_missing_fields_are_errors() {
    let (_tmp, root) = tempdir();
    write(&root, "a.yaml", "extends: b.yaml\n");
    let b = write(&root, "b.yaml", "extends: [a.yaml]\n");
    let err = load_with_env(&root.join("a.yaml"), &no_env).unwrap_err();
    assert!(
        matches!(&err, Error::ExtendsCycle { path } if path == &root.join("a.yaml") || path == &b),
        "{err}"
    );

    let config = write(&root, "c.yaml", "input: a.yaml\n");
    let err = load_with_env(&config, &no_env).unwrap_err();
    assert!(matches!(
        err,
        Error::Missing {
            field: "output",
            ..
        }
    ));

    let config = write(
        &root,
        "d.yaml",
        "input: a.yaml\ntargets: [{ output: x }, { output: y }]\n",
    );
    let err = load_with_env(&config, &no_env).unwrap_err();
    assert!(matches!(err, Error::UnnamedTarget { index: 0 }));
}

#[test]
fn json_schema_is_up_to_date() {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schema/tsurara.config.schema.json");
    let want = tsurara_config::json_schema();
    if std::env::var_os("TSURARA_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let got = std::fs::read_to_string(&path).unwrap_or_default();
    assert!(
        got == want,
        "schema/tsurara.config.schema.json is out of date; run `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-config`"
    );
}

#[test]
fn the_design_example_loads() {
    let (_tmp, root) = tempdir();
    write(
        &root,
        "tsurara.base.yaml",
        "typescript:\n  importExtension: ts\n",
    );
    let config = write(
        &root,
        "packages/openapi/tsurara.config.yaml",
        r##"
extends: ../../tsurara.base.yaml
targets:
  - name: public-api
    input:
      url: https://api.example.com/openapi.json
      headers:
        Authorization: "Bearer ${API_TOKEN}"
    output: src/generated/public
    filter:
      tags: [pet, store]
    typescript:
      typeMappings:
        uuid: { type: Uuid, import: { from: ../brand.js, name: Uuid } }
    naming:
      overrides:
        "#/components/schemas/Error": ApiErrorBody
    hooks:
      transformDocument: [node, scripts/patch-spec.mjs]
      afterWrite:
        - [biome, format, --write, "{output}"]
"##,
    );
    let env = |_: &str| Some("t".to_owned());
    let config = load_with_env(&config, &env).unwrap();
    let t = config.target("public-api").unwrap();
    let dir = root.join("packages/openapi");
    assert_eq!(t.output, dir.join("src/generated/public"));
    assert_eq!(ts(t).import_extension, ImportExtension::Ts);
    assert_eq!(
        t.options.type_mappings["uuid"]["typescript"]
            .import
            .as_ref()
            .unwrap()
            .from,
        "../../../../brand.js"
    );
    assert_eq!(
        t.options.name_overrides["#/components/schemas/Error"],
        "ApiErrorBody"
    );
    assert_eq!(
        t.hooks.transform_document[0].args,
        ["node", "scripts/patch-spec.mjs"]
    );
    assert_eq!(t.hooks.transform_document[0].dir, dir);
    assert_eq!(t.hooks.after_write.len(), 1);
}

#[test]
fn wildcard_as_json_is_read() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: openapi.yaml\noutput: out\nwildcardAsJson: true\n",
    );
    let config = load_with_env(&config, &no_env).unwrap();
    assert!(config.targets[0].options.wildcard_as_json);
}

#[test]
fn two_targets_share_one_input_with_their_own_language_sections() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        r#"
input: openapi.yaml
targets:
  - name: web
    output: web/generated
    language: typescript
    typescript: { importExtension: ts }
  - name: node
    output: node/generated
    typescript: { importExtension: js, layout: single }
"#,
    );
    let config = load_with_env(&config, &no_env).unwrap();
    let web = config.target("web").unwrap();
    let node = config.target("node").unwrap();
    assert_eq!(web.input, node.input);
    assert_eq!(web.language.id(), "typescript");
    assert_eq!(node.language.id(), "typescript");
    assert_eq!(ts(web).import_extension, ImportExtension::Ts);
    assert_eq!(ts(node).layout, Layout::Single);
}

#[test]
fn unknown_languages_and_removed_top_level_keys_are_errors() {
    let (_tmp, root) = tempdir();
    let bad = write(
        &root,
        "a.yaml",
        "input: x.yaml\noutput: out\nlanguage: cobol\n",
    );
    let msg = load_with_env(&bad, &no_env).unwrap_err().to_string();
    assert!(msg.contains("unknown language `cobol`"), "{msg}");
    let bad = write(
        &root,
        "b.yaml",
        "input: x.yaml\noutput: out\nimportExtension: ts\n",
    );
    let msg = load_with_env(&bad, &no_env).unwrap_err().to_string();
    assert!(msg.contains("unknown field"), "{msg}");
    let bad = write(
        &root,
        "c.yaml",
        "input: x.yaml\noutput: out\npython: { strictEnums: true }\n",
    );
    let msg = load_with_env(&bad, &no_env).unwrap_err().to_string();
    assert!(
        msg.contains("generates `typescript`, not `python`"),
        "{msg}"
    );
}

#[test]
fn two_targets_share_one_input_with_different_languages() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        r#"
input: openapi.yaml
targets:
  - name: web
    output: web/generated
    typescript: { importExtension: ts }
  - name: api
    output: api/generated
    language: python
    python: { strictEnums: true }
"#,
    );
    let config = load_with_env(&config, &no_env).unwrap();
    let web = config.target("web").unwrap();
    let api = config.target("api").unwrap();
    assert_eq!(web.input, api.input);
    assert_eq!(web.language.id(), "typescript");
    assert_eq!(api.language.id(), "python");
    let Language::Python(py) = &api.language else {
        panic!()
    };
    assert!(py.strict_enums);
}

#[test]
fn type_mappings_that_are_not_type_expressions_are_errors() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\ntypescript:\n  overrides:\n    \"#/components/schemas/Id\": \"string }; console.log(1); type Z = {\"\n",
    );
    let err = load_with_env(&config, &no_env).unwrap_err();
    assert!(
        matches!(&err, Error::InvalidType { key, .. } if key == "#/components/schemas/Id"),
        "{err}"
    );
}

/// Every key of the configuration schema is described in the user guide.
#[test]
fn the_guide_describes_every_key() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join("schema/tsurara.config.schema.json")).unwrap(),
    )
    .unwrap();
    let guide = std::fs::read_to_string(root.join("docs/guide/configuration.md")).unwrap();
    let mut missing = Vec::new();
    let mut keys = Vec::new();
    for def in schema["$defs"]
        .as_object()
        .into_iter()
        .flat_map(|d| d.values())
        .chain([&schema])
    {
        for key in def["properties"]
            .as_object()
            .into_iter()
            .flat_map(|p| p.keys())
        {
            keys.push(key.clone());
        }
    }
    for key in keys {
        if key != "$schema"
            && !guide.contains(&format!("`{key}`"))
            && !guide.contains(&format!("{key}:"))
        {
            missing.push(key);
        }
    }
    missing.sort();
    missing.dedup();
    assert!(
        missing.is_empty(),
        "docs/guide/configuration.md does not describe {missing:?}"
    );
}

#[test]
fn validators_select_the_zod_schemas() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\ntypescript:\n  validators: zod\n",
    );
    let loaded = load_with_env(&config, &no_env).unwrap();
    assert_eq!(
        ts(&loaded.targets[0]).validators,
        Some(tsurara_lang_typescript::options::Validators::Zod)
    );
}

#[test]
fn rust_targets_take_the_rust_section() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\nlanguage: rust\nrust:\n  runtimeCrate: my_client\n",
    );
    let loaded = load_with_env(&config, &no_env).unwrap();
    match &loaded.targets[0].language {
        tsurara_config::Language::Rust(o) => assert_eq!(o.runtime_crate, "my_client"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn go_targets_take_the_go_section_and_default_the_package_to_the_output_directory() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "targets:\n  - name: a\n    input: a.yaml\n    output: gen/pet-store\n    language: go\n  - name: b\n    input: a.yaml\n    output: out\n    language: go\n    go:\n      package: petstore\n      runtimeModule: example.com/rt\n",
    );
    let loaded = load_with_env(&config, &no_env).unwrap();
    match &loaded.targets[0].language {
        tsurara_config::Language::Go(o) => {
            assert_eq!(o.package, "petstore");
            assert_eq!(
                o.runtime_module,
                "github.com/tsurara-openapi/tsurara/runtimes/go"
            );
        }
        other => panic!("{other:?}"),
    }
    match &loaded.targets[1].language {
        tsurara_config::Language::Go(o) => {
            assert_eq!(o.package, "petstore");
            assert_eq!(o.runtime_module, "example.com/rt");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn go_and_rust_targets_take_type_mappings_and_overrides() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "targets:\n  - name: go\n    input: a.yaml\n    output: out\n    language: go\n    go:\n      typeMappings:\n        date-time: { type: time.Time, import: { from: time } }\n        uuid: { type: uuid.UUID, import: { from: github.com/google/uuid } }\n      overrides:\n        \"#/components/schemas/Id\": int64\n  - name: rust\n    input: a.yaml\n    output: out2\n    language: rust\n    rust:\n      typeMappings:\n        date-time: chrono::DateTime<chrono::Utc>\n",
    );
    let loaded = load_with_env(&config, &no_env).unwrap();
    let go = &loaded.targets[0].options;
    let dt = &go.type_mappings["date-time"]["go"];
    assert_eq!(dt.code, "time.Time");
    assert_eq!(dt.import.as_ref().unwrap().from, "time");
    assert_eq!(
        go.type_mappings["uuid"]["go"].import.as_ref().unwrap().from,
        "github.com/google/uuid"
    );
    assert_eq!(
        go.type_overrides["#/components/schemas/Id"]["go"].code,
        "int64"
    );
    let rust = &loaded.targets[1].options;
    assert_eq!(
        rust.type_mappings["date-time"]["rust"].code,
        "chrono::DateTime<chrono::Utc>"
    );
}

#[test]
fn invalid_go_types_and_unknown_section_keys_are_reported_where_they_are() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\nlanguage: go\ngo:\n  typeMapping: {}\n",
    );
    let msg = load_with_env(&config, &no_env).unwrap_err().to_string();
    assert!(msg.contains("go: unknown field `typeMapping`"), "{msg}");
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: a.yaml\noutput: out\nlanguage: go\ngo:\n  typeMappings:\n    date-time: \"time.Time; panic()\"\n",
    );
    let msg = load_with_env(&config, &no_env).unwrap_err().to_string();
    assert!(msg.contains("date-time"), "{msg}");
}

#[test]
fn function_case_is_read() {
    let (_tmp, root) = tempdir();
    let config = write(
        &root,
        "tsurara.config.yaml",
        "input: openapi.yaml\noutput: out\ntypescript:\n  naming:\n    functionCase: camel\n",
    );
    let config = load_with_env(&config, &no_env).unwrap();
    assert_eq!(
        ts(&config.targets[0]).function_name_case,
        tsurara_lang_typescript::options::FunctionCase::Camel
    );
}
