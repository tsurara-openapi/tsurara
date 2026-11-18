//! Types without the HTTP runtime: with `operations: false` every built-in backend emits
//! the declarations (with webhooks and channels) only, and the output passes the language's checker
//! with only the types layer of the runtime: no httpx, no tokio or reqwest, no HTTP code.

use std::process::Command;
use tsurara_backend::{Backend, OutputFile, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, python_or_skip,
    repo_root,
};

const DEFINITIONS: &[&str] = &[
    "crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml",
    "crates/tsurara-lang-typescript/tests/fixtures/kitchen-sink.yaml",
    "corpus/edge-cases/event-channels.yaml",
    // Binary properties (`File`) in schemas that are multipart bodies.
    "corpus/edge-cases/multipart.yaml",
];

fn load(path: &str) -> DocumentSet {
    let text = std::fs::read(repo_root().join(path)).unwrap();
    DocumentSet::from_value(parse_document(&text).unwrap())
}

fn types_only() -> Options {
    Options {
        operations: false,
        ..Options::default()
    }
}

fn files(backend: &dyn Backend, path: &str, opts: &Options) -> Vec<(String, String)> {
    generate(backend, &load(path), opts)
        .unwrap()
        .files
        .into_iter()
        .map(|OutputFile { path, contents }| (path, contents))
        .collect()
}

fn assert_none_contains(path: &str, files: &[(String, String)], needles: &[&str]) {
    for (file, contents) in files {
        for needle in needles {
            assert!(
                !contents.contains(needle),
                "{path}: {file} contains {needle:?}:\n{contents}"
            );
        }
    }
}

#[test]
fn typescript_types_import_no_runtime() {
    let backend = tsurara_lang_typescript::TypeScript {
        options: tsurara_lang_typescript::TypeScriptOptions {
            validators: Some(tsurara_lang_typescript::options::Validators::Zod),
            ..Default::default()
        },
    };
    for path in DEFINITIONS {
        // With the default options, the declarations and their schemas import no runtime module.
        let all = files(&backend, path, &Options::default());
        let declarations: Vec<(String, String)> = all
            .iter()
            .filter(|(f, _)| f.starts_with("schemas/") || f.starts_with("zod/schemas"))
            .cloned()
            .collect();
        assert!(!declarations.is_empty(), "{path}");
        assert_none_contains(path, &declarations, &["@tsurara/client"]);

        let out = files(&backend, path, &types_only());
        for (file, _) in &out {
            assert!(
                !file.starts_with("operations/") && file != "client.ts",
                "{path}: {file}"
            );
        }
        // Channels decode and validate messages with functions of the runtime that make no requests
        // (`decodeMessage`, `validateChannels`).
        let types: Vec<(String, String)> = out
            .iter()
            .filter(|(f, _)| !f.starts_with("channels/") && f != "zod/channels.ts")
            .cloned()
            .collect();
        assert_none_contains(path, &types, &["@tsurara/client'", "@tsurara/client\";"]);
        TypeScriptChecker {
            options: TsOptions::default(),
        }
        .check(&out)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

/// Imports the package and decodes a value with only the standard library and the runtime's sources:
/// `-S` leaves out site-packages, where httpx is installed.
const PYTHON_DECODE: &str = r#"import importlib, pkgutil, sys
import api
for m in pkgutil.walk_packages(api.__path__, "api."):
    importlib.import_module(m.name)
from api._typedefs import CODEC
from api.pet import Pet
pet = CODEC.decode({"ref": "Pet"}, {"id": 1, "name": "Pochi"})
assert isinstance(pet, Pet) and pet.name == "Pochi", pet
assert CODEC.encode({"ref": "Pet"}, pet) == {"id": 1, "name": "Pochi"}
assert "httpx" not in sys.modules
print("ok")
"#;

#[test]
fn python_types_need_no_httpx() {
    let backend = tsurara_lang_python::Python::default();
    for path in DEFINITIONS {
        let out = files(&backend, path, &types_only());
        assert!(
            out.iter().all(|(f, _)| !f.starts_with("operations/")),
            "{path}"
        );
        assert_none_contains(path, &out, &["httpx", "import Client"]);
        PythonChecker {
            package: "api".into(),
        }
        .check(&out)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    }

    let Some(python) = python_or_skip() else {
        return;
    };
    let src = repo_root().join("runtimes/python/src");
    let without_httpx = |args: &[&str], dir: &std::path::Path| {
        let mut path = std::ffi::OsString::from(&src);
        path.push(":");
        path.push(dir);
        Command::new(&python)
            .arg("-S")
            .args(args)
            .env("PYTHONPATH", path)
            .current_dir(dir)
            .output()
            .unwrap()
    };
    let dir = tempfile::tempdir().unwrap();
    // The environment really lacks httpx.
    assert!(
        !without_httpx(&["-c", "import httpx"], dir.path())
            .status
            .success()
    );

    let out = files(&backend, DEFINITIONS[0], &types_only());
    for (path, contents) in &out {
        let full = dir.path().join("api").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    let run = without_httpx(&["-c", PYTHON_DECODE], dir.path());
    assert!(
        run.status.success(),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );

    // The TypeDef vectors of spec/conformance run on the types layer alone.
    let script = repo_root().join("runtimes/python/tests/test_typedefs.py");
    let run = without_httpx(&[script.to_str().unwrap(), "--without-httpx"], dir.path());
    assert!(
        run.status.success() && String::from_utf8_lossy(&run.stdout).contains("ok"),
        "{}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
}

const RUST_DECODE: &str = r##"use tsurara_generated::api::types::Pet;

#[test]
fn decodes() {
    let pet: Pet = tsurara_client::serde_json::from_str(r#"{"id":1,"name":"Pochi"}"#).unwrap();
    assert_eq!(pet.name, "Pochi");
    assert_eq!(tsurara_client::serde_json::to_value(&pet).unwrap(), tsurara_client::serde_json::json!({"id": 1, "name": "Pochi"}));
}
"##;

#[test]
fn rust_types_build_without_the_client_feature() {
    let backend = tsurara_lang_rust::Rust::default();
    for path in DEFINITIONS {
        let out = files(&backend, path, &types_only());
        assert!(
            out.iter().all(|(f, _)| !f.starts_with("operations/")),
            "{path}"
        );
        assert_none_contains(path, &out, &["pub mod operations", "reqwest", "tokio"]);
        let tests = if *path == DEFINITIONS[0] {
            vec![("decode".to_owned(), RUST_DECODE.to_owned())]
        } else {
            Vec::new()
        };
        RustChecker {
            tests,
            http: false,
            mocks: false,
            types_only: true,
            ..Default::default()
        }
        .check(&out)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}

#[test]
fn go_types_have_no_http_code() {
    let backend = tsurara_lang_go::Go::default();
    for path in DEFINITIONS {
        let out = files(&backend, path, &types_only());
        assert_none_contains(path, &out, &["net/http", "context.Context", "tsurara.Do"]);
        GoChecker {
            tests: Vec::new(),
            tags: Vec::new(),
        }
        .check(&out)
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    }
}
