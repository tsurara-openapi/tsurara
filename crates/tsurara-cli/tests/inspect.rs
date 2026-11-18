//! Machine-readable output and `tsurara inspect` (design.md 12), through the binary.

use serde_json::{Value, json};
use std::path::Path;
use std::process::{Command, Output};
use tsurara_backend::{Backend, OutputFile, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, repo_root,
};

const PETSTORE: &str = "crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml";

fn tsurara(cwd: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn json_of(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!("{e}: {}", String::from_utf8_lossy(&out.stdout));
    })
}

/// A directory with petstore and one target per language.
fn project() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(repo_root().join(PETSTORE), tmp.path().join("openapi.yaml")).unwrap();
    std::fs::write(
        tmp.path().join("tsurara.config.yaml"),
        "input: openapi.yaml\ntargets:\n  - { name: ts, output: ts }\n  - { name: tsq, output: tsq, typescript: { query: tanstack, mocks: msw } }\n  - { name: py, output: py, language: python }\n  - { name: rs, output: rs, language: rust }\n  - { name: go, output: go, language: go, go: { package: api } }\n",
    )
    .unwrap();
    tmp
}

fn generated(backend: &dyn Backend) -> Vec<(String, String)> {
    let doc = parse_document(&std::fs::read(repo_root().join(PETSTORE)).unwrap()).unwrap();
    let out = generate(backend, &DocumentSet::from_value(doc), &Options::default()).unwrap();
    out.files
        .into_iter()
        .map(|OutputFile { path, contents }| (path, contents))
        .collect()
}

/// The example `inspect operation` prints for `operation` in `target`, as (file, code).
fn example(tmp: &Path, target: &str, operation: &str) -> (String, String) {
    let out = json_of(&tsurara(
        tmp,
        &[
            "inspect",
            "operation",
            operation,
            "--target",
            target,
            "--format",
            "json",
        ],
    ));
    let e = &out["example"];
    (
        e["file"].as_str().unwrap().to_owned(),
        e["code"].as_str().unwrap().to_owned(),
    )
}

const OPERATIONS: [&str; 3] = ["listPets", "createPets", "showPetById"];

#[test]
fn typescript_examples_compile() {
    let tmp = project();
    for op in OPERATIONS {
        let mut files = generated(&tsurara_lang_typescript::TypeScript::default());
        files.push(example(tmp.path(), "ts", op));
        if let Err(e) = (TypeScriptChecker {
            options: TsOptions::default(),
        })
        .check(&files)
        {
            panic!("{op}: {e}");
        }
    }
}

#[test]
fn python_examples_compile() {
    let tmp = project();
    for op in OPERATIONS {
        let mut files = generated(&tsurara_lang_python::Python::default());
        files.push(example(tmp.path(), "py", op));
        let checker = PythonChecker {
            package: "api".into(),
        };
        if let Err(e) = checker.check(&files) {
            panic!("{op}: {e}");
        }
    }
}

#[test]
fn rust_examples_compile() {
    let tmp = project();
    for op in OPERATIONS {
        let mut files = generated(&tsurara_lang_rust::Rust::default());
        files.push(example(tmp.path(), "rs", op));
        let root = files.iter_mut().find(|(p, _)| p == "mod.rs").unwrap();
        root.1.push_str("pub mod example;\n");
        if let Err(e) = RustChecker::default().check(&files) {
            panic!("{op}: {e}");
        }
    }
}

#[test]
fn go_examples_compile() {
    let tmp = project();
    for op in OPERATIONS {
        let mut files = generated(&tsurara_lang_go::Go::default());
        files.push(example(tmp.path(), "go", op));
        if let Err(e) = GoChecker::default().check(&files) {
            panic!("{op}: {e}");
        }
    }
}

#[test]
fn inspect_operations_lists_functions() {
    let tmp = project();
    let out = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operations",
            "--target",
            "py",
            "--format",
            "json",
        ],
    ));
    let ops = out["operations"].as_array().unwrap();
    assert_eq!(ops.len(), 3);
    let show = ops
        .iter()
        .find(|o| o["operationId"] == "showPetById")
        .unwrap();
    assert_eq!(show["method"], "GET");
    assert_eq!(show["path"], "/pets/{petId}");
    assert_eq!(show["function"]["identifier"], "show_pet_by_id");
    let filtered = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operations",
            "--target",
            "ts",
            "--search",
            "create",
            "--format",
            "json",
        ],
    ));
    assert_eq!(filtered["operations"].as_array().unwrap().len(), 1);
    let text = tsurara(tmp.path(), &["inspect", "operations", "--target", "ts"]);
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("GET /pets/{petId}"), "{text}");
}

#[test]
fn inspect_operation_by_method_and_path() {
    let tmp = project();
    let out = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operation",
            "GET /pets/{petId}",
            "--target",
            "ts",
            "--format",
            "json",
        ],
    ));
    assert_eq!(out["operationId"], "showPetById");
    assert_eq!(out["parameters"][0]["name"], "petId");
    assert_eq!(out["parameters"][0]["in"], "path");
    assert!(
        out["responses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r["status"] == "200")
    );
}

#[test]
fn inspect_schema_and_warning() {
    let tmp = project();
    let out = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect", "schema", "Error", "--target", "ts", "--format", "json",
        ],
    ));
    assert_eq!(out["identifier"], "ErrorModel");
    assert_eq!(out["file"], "schemas/ErrorModel.ts");
    assert!(
        out["declaration"]
            .as_str()
            .unwrap()
            .contains("export type ErrorModel")
    );
    let warning = json_of(&tsurara(
        tmp.path(),
        &["inspect", "warning", "renamed", "--format", "json"],
    ));
    assert_eq!(warning["code"], "renamed");
    assert!(
        warning["explanation"]
            .as_str()
            .unwrap()
            .contains("naming.overrides")
    );
    let unknown = tsurara(tmp.path(), &["inspect", "warning", "nope"]);
    assert!(!unknown.status.success());
}

/// `generate --format json` and `check --format json` validate against `spec/cli-output.schema.json`
/// on the edge-case corpus.
#[test]
fn json_output_validates_against_its_schema() {
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/cli-output.schema.json")).unwrap(),
    )
    .unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    let dir = repo_root().join("corpus/edge-cases");
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::copy(&path, tmp.path().join("openapi.yaml")).unwrap();
        std::fs::write(
            tmp.path().join("tsurara.config.yaml"),
            "input: openapi.yaml\noutput: out\n",
        )
        .unwrap();
        for command in ["generate", "check"] {
            let out = tsurara(tmp.path(), &[command, "--format", "json"]);
            let value: Value = serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}: {}",
                    path.display(),
                    String::from_utf8_lossy(&out.stdout)
                )
            });
            let errors: Vec<String> = validator
                .iter_errors(&value)
                .map(|e| e.to_string())
                .collect();
            assert!(
                errors.is_empty(),
                "{} {command}: {errors:?}\n{value}",
                path.display()
            );
            assert_eq!(value["command"], json!(command));
        }
    }
}

#[test]
fn generate_json_lists_files_and_warnings() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::copy(
        repo_root().join("corpus/edge-cases/names.yaml"),
        tmp.path().join("openapi.yaml"),
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("tsurara.config.yaml"),
        "input: openapi.yaml\noutput: out\n",
    )
    .unwrap();
    let out = json_of(&tsurara(tmp.path(), &["generate", "--format", "json"]));
    let target = &out["targets"][0];
    assert!(
        target["written"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "index.ts")
    );
    let warning = &target["warnings"][0];
    assert!(warning["code"].is_string() && warning["pointer"].is_string());
    let again = json_of(&tsurara(tmp.path(), &["generate", "--format", "json"]));
    assert!(
        again["targets"][0]["written"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let check = json_of(&tsurara(tmp.path(), &["check", "--format", "json"]));
    assert_eq!(check["targets"][0]["upToDate"], true);
}

/// Generated helpers: the query options, keys, filters and mock handlers of an operation
/// are in the manifest, `inspect operation` names them, and the TypeScript example shows the query
/// options next to the call.
#[test]
fn inspect_operation_names_the_generated_helpers() {
    let tmp = project();
    let show = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operation",
            "showPetById",
            "--target",
            "tsq",
            "--format",
            "json",
        ],
    ));
    let helpers = &show["helpers"];
    for (role, identifier, file) in [
        ("queryKey", "showPetByIdQueryKey", "query/showPetById.ts"),
        (
            "queryOptions",
            "showPetByIdQueryOptions",
            "query/showPetById.ts",
        ),
        (
            "queryFilter",
            "showPetByIdQueryFilter",
            "query/showPetById.ts",
        ),
        (
            "setQueriesData",
            "setShowPetByIdQueriesData",
            "query/showPetById.ts",
        ),
        (
            "getQueriesData",
            "getShowPetByIdQueriesData",
            "query/showPetById.ts",
        ),
        (
            "mockHandler",
            "showPetByIdHandler",
            "mocks/operations/showPetById.ts",
        ),
    ] {
        assert_eq!(helpers[role]["identifier"], identifier, "{role}: {helpers}");
        assert_eq!(helpers[role]["file"], file, "{role}: {helpers}");
    }
    let create = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operation",
            "createPets",
            "--target",
            "tsq",
            "--format",
            "json",
        ],
    ));
    assert_eq!(
        create["helpers"]["mutationOptions"]["identifier"],
        "createPetsMutationOptions"
    );
    assert!(
        create["helpers"].get("queryOptions").is_none(),
        "{}",
        create["helpers"]
    );
    // Without the options, nothing.
    let plain = json_of(&tsurara(
        tmp.path(),
        &[
            "inspect",
            "operation",
            "showPetById",
            "--target",
            "ts",
            "--format",
            "json",
        ],
    ));
    assert!(
        plain
            .get("helpers")
            .is_none_or(|h| h.as_object().is_none_or(|m| m.is_empty())),
        "{plain}"
    );
    // The text lists them.
    let text = tsurara(
        tmp.path(),
        &["inspect", "operation", "showPetById", "--target", "tsq"],
    );
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(
        text.contains("showPetByIdQueryOptions (query/showPetById.ts)"),
        "{text}"
    );
    assert!(text.contains("useQuery"), "{text}");
}

#[test]
fn typescript_examples_with_query_options_compile() {
    let tmp = project();
    let backend = tsurara_lang_typescript::TypeScript {
        options: tsurara_lang_typescript::TypeScriptOptions {
            query: Some(tsurara_lang_typescript::options::Query::Tanstack),
            mocks: Some(tsurara_lang_typescript::options::Mocks::Msw),
            ..Default::default()
        },
    };
    for op in OPERATIONS {
        let mut files = generated(&backend);
        let (file, code) = example(tmp.path(), "tsq", op);
        assert!(
            code.contains("QueryOptions") || code.contains("MutationOptions"),
            "{op}: {code}"
        );
        files.push((file, code));
        if let Err(e) = (TypeScriptChecker {
            options: TsOptions::default(),
        })
        .check(&files)
        {
            panic!("{op}: {e}");
        }
    }
}

/// Examples use the definition's first server, as the generated `servers` do.
#[test]
fn examples_use_the_server_of_the_definition() {
    let tmp = project();
    for target in ["ts", "py", "rs", "go"] {
        let (_, code) = example(tmp.path(), target, "listPets");
        assert!(
            code.contains("http://petstore.swagger.io/v1"),
            "{target}: {code}"
        );
        assert!(!code.contains("api.example.com"), "{target}: {code}");
    }
}
