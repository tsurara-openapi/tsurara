//! A vendored runtime (`runtime: vendor`): for each language, petstore generated with
//! the runtime copied into the output passes the language's checker in a project without the runtime
//! package, calls a test server through the copy, and the copy passes the conformance vectors. With
//! the CLI, editing the copy fails `tsurara check`, generating restores it, and leaving vendoring
//! deletes it.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::Command;
use tsurara_backend::{Backend, OutputFile, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, RuntimeSource};
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, python_or_skip,
    repo_root, tsc_path,
};

const PETSTORE: &str = "crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml";

fn vendored() -> Options {
    Options {
        runtime: RuntimeSource::Vendor,
        ..Options::default()
    }
}

fn files(backend: &dyn Backend) -> Vec<(String, String)> {
    let text = std::fs::read(repo_root().join(PETSTORE)).unwrap();
    let set = DocumentSet::from_value(parse_document(&text).unwrap());
    generate(backend, &set, &vendored())
        .unwrap()
        .files
        .into_iter()
        .map(|OutputFile { path, contents }| (path, contents))
        .collect()
}

fn write_all(dir: &Path, files: &[(String, String)]) {
    for (path, contents) in files {
        let full = dir.join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
}

/// A server that answers `GET /v1/pets/1` with a pet, and anything else with 404. Returns its base
/// URL.
fn serve() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut first = String::new();
            if reader.read_line(&mut first).is_err() {
                continue;
            }
            loop {
                let mut line = String::new();
                if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                    break;
                }
            }
            let (status, body) = if first.starts_with("GET /v1/pets/1 ") {
                ("200 OK", r#"{"id":1,"name":"Pochi"}"#)
            } else {
                ("404 Not Found", r#"{"code":404,"message":"no"}"#)
            };
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    url
}

fn assert_ok(what: &str, out: &std::process::Output) {
    assert!(
        out.status.success(),
        "{what}:\n{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// No generated file imports the runtime package.
fn assert_self_contained(files: &[(String, String)], needles: &[&str]) {
    for (path, contents) in files {
        for needle in needles {
            assert!(!contents.contains(needle), "{path} contains {needle:?}");
        }
    }
}

fn node_supports_type_stripping() -> bool {
    Command::new("node")
        .arg("--version")
        .output()
        .is_ok_and(|o| {
            let v = String::from_utf8_lossy(&o.stdout);
            let mut parts = v.trim().trim_start_matches('v').split('.');
            let major: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            let minor: u32 = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
            (major, minor) >= (22, 18)
        })
}

const TS_RUN: &str = r#"import { createClient } from './api/_runtime/index.ts';
import { showPetById } from './api/operations/showPetById.ts';

const api = createClient({ baseUrl: process.argv[2] ?? '' });
const r = await showPetById(api, { path: { petId: '1' } });
if (!r.ok || r.data.name !== 'Pochi') throw new Error(JSON.stringify(r));
console.log('ok');
"#;

#[test]
fn typescript_vendored_runtime() {
    let backend = tsurara_lang_typescript::TypeScript::default();
    let out = files(&backend);
    assert!(out.iter().any(|(p, _)| p == "_runtime/index.ts"));
    assert_self_contained(&out, &["from '@tsurara/client", "// @tsurara/client "]);
    TypeScriptChecker {
        options: TsOptions::default(),
    }
    .check(&out)
    .unwrap();

    if !node_supports_type_stripping() || tsc_path().is_none() {
        assert!(
            std::env::var_os("TSURARA_REQUIRE_NODE").is_none(),
            "Node 22.18 or later is required"
        );
        return;
    }
    // A project without node_modules, run with Node's type stripping.
    let dir = tempfile::tempdir().unwrap();
    let ts = tsurara_lang_typescript::TypeScript {
        options: tsurara_lang_typescript::TypeScriptOptions {
            import_extension: tsurara_lang_typescript::options::ImportExtension::Ts,
            ..Default::default()
        },
    };
    let out: Vec<(String, String)> = files(&ts)
        .into_iter()
        .map(|(p, c)| (format!("api/{p}"), c))
        .collect();
    write_all(dir.path(), &out);
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), TS_RUN).unwrap();
    let run = Command::new("node")
        .arg("run.ts")
        .arg(serve())
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_ok("node run.ts", &run);

    // The conformance vectors against the copy.
    let test = repo_root().join("runtimes/typescript/test");
    let spec = format!("file://{}/", repo_root().join("spec/conformance").display());
    for name in ["conformance.test.ts", "helpers.ts"] {
        let source = std::fs::read_to_string(test.join(name))
            .unwrap()
            .replace("'../src/index.ts'", "'../api/_runtime/index.ts'")
            .replace(
                "`../../../spec/conformance/${name}`",
                &format!("`{spec}${{name}}`"),
            );
        write_all(dir.path(), &[(format!("test/{name}"), source)]);
    }
    let run = Command::new("node")
        .args(["--test", "test/conformance.test.ts"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_ok("node --test", &run);
}

const PY_RUN: &str = r#"import sys

sys.modules["tsurara_client"] = None  # type: ignore[assignment]  # the package is not installed

from api._runtime import Client, Success
from api.operations import show_pet_by_id

with Client(sys.argv[1]) as client:
    r = show_pet_by_id(client, pet_id="1")
    assert isinstance(r, Success) and r.data.name == "Pochi", r
print("ok")
"#;

#[test]
fn python_vendored_runtime() {
    let out = files(&tsurara_lang_python::Python::default());
    assert!(out.iter().any(|(p, _)| p == "_runtime/__init__.py"));
    for (path, contents) in &out {
        assert!(
            !contents.contains("from tsurara_client")
                && !contents.contains("import tsurara_client"),
            "{path}"
        );
    }
    PythonChecker {
        package: "api".into(),
    }
    .check(&out)
    .unwrap();

    let Some(python) = python_or_skip() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let packaged: Vec<(String, String)> = out
        .iter()
        .map(|(p, c)| (format!("api/{p}"), c.clone()))
        .collect();
    write_all(dir.path(), &packaged);
    std::fs::write(dir.path().join("run.py"), PY_RUN).unwrap();
    let run = Command::new(&python)
        .arg("run.py")
        .arg(serve())
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_ok("python run.py", &run);

    // The conformance vectors against the copy, installed as `tsurara_client` ahead of the package.
    let shim = tempfile::tempdir().unwrap();
    let copy: Vec<(String, String)> = out
        .iter()
        .filter_map(|(p, c)| {
            p.strip_prefix("_runtime/")
                .map(|rest| (format!("tsurara_client/{rest}"), c.clone()))
        })
        .collect();
    write_all(shim.path(), &copy);
    let check = Command::new(&python)
        .args([
            "-c",
            "import tsurara_client; print(tsurara_client.__file__)",
        ])
        .env("PYTHONPATH", shim.path())
        .output()
        .unwrap();
    assert!(
        String::from_utf8_lossy(&check.stdout).contains(&*shim.path().to_string_lossy()),
        "{}",
        String::from_utf8_lossy(&check.stdout)
    );
    let tests = repo_root().join("runtimes/python/tests");
    let run = Command::new(&python)
        .args(["-m", "pytest", "-q", "-p", "no:cacheprovider"])
        .arg(tests.join("test_conformance.py"))
        .arg(tests.join("test_typedefs.py"))
        .env("PYTHONPATH", shim.path())
        .current_dir(shim.path())
        .output()
        .unwrap();
    assert_ok("pytest", &run);
}

#[test]
fn rust_vendored_runtime() {
    let backend = tsurara_lang_rust::Rust {
        options: tsurara_lang_rust::RustOptions {
            runtime_crate: "crate::api::runtime".into(),
            ..Default::default()
        },
    };
    let out = files(&backend);
    assert!(out.iter().any(|(p, _)| p == "runtime/mod.rs"));
    assert_self_contained(&out, &["tsurara_client::", "Requires tsurara-client"]);

    let url = serve();
    let call = format!(
        r#"use tsurara_generated::api::operations::show_pet_by_id;
use tsurara_generated::api::runtime::Client;

#[tokio::test]
async fn calls_the_server() {{
    let client = Client::builder({url:?}).build();
    let pet = show_pet_by_id(&client, "1".into(), Default::default()).await.unwrap();
    assert_eq!(pet.data.name, "Pochi");
}}
"#
    );
    let spec = repo_root().join("spec/conformance");
    let conformance = std::fs::read_to_string(repo_root().join("runtimes/rust/tests/conformance.rs"))
        .unwrap()
        .replace("tsurara_client::", "tsurara_generated::api::runtime::")
        .replace(
            "std::path::Path::new(env!(\"CARGO_MANIFEST_DIR\"))\n        .join(\"../../spec/conformance\")",
            &format!("std::path::Path::new({:?})", spec.to_string_lossy()),
        );
    assert!(conformance.contains("tsurara_generated::api::runtime::"));
    RustChecker {
        tests: vec![("call".into(), call), ("conformance".into(), conformance)],
        vendored: tsurara_lang_rust::vendored_dependencies(true)
            .into_iter()
            .map(str::to_owned)
            .collect(),
        ..Default::default()
    }
    .check(&out)
    .unwrap();

    // Without `runtimeCrate`, the generator cannot place the copy.
    let text = std::fs::read(repo_root().join(PETSTORE)).unwrap();
    let set = DocumentSet::from_value(parse_document(&text).unwrap());
    let err = generate(&tsurara_lang_rust::Rust::default(), &set, &vendored()).unwrap_err();
    assert!(err.to_string().contains("rust.runtimeCrate"), "{err}");
}

#[test]
fn go_vendored_runtime() {
    // GoChecker's module is `example.com/gen` with the package in `api`; the configuration computes
    // this path from go.mod.
    let runtime = "example.com/gen/api/internal/tsurara";
    let backend = tsurara_lang_go::Go {
        options: tsurara_lang_go::GoOptions {
            package: "api".into(),
            runtime_module: runtime.into(),
            ..Default::default()
        },
    };
    let mut out = files(&backend);
    assert!(out.iter().any(|(p, _)| p == "internal/tsurara/client.go"));
    assert_self_contained(&out, &["github.com/tsurara-openapi/tsurara/runtimes/go"]);
    // The conformance vectors run in the copy.
    let conformance = std::fs::read_to_string(repo_root().join("runtimes/go/conformance_test.go"))
        .unwrap()
        .replace(
            "\"github.com/tsurara-openapi/tsurara/runtimes/go\"",
            &format!("{runtime:?}"),
        )
        .replace(
            "\"../../spec/conformance/\"",
            &format!(
                "{:?}",
                format!("{}/", repo_root().join("spec/conformance").display())
            ),
        );
    out.push(("internal/tsurara/conformance_test.go".into(), conformance));
    let url = serve();
    GoChecker {
        tests: vec![(
            "call".into(),
            format!(
                "package api\n\nimport (\n\t\"context\"\n\t\"testing\"\n\n\ttsurara \"{runtime}\"\n)\n\nfunc TestCall(t *testing.T) {{\n\tc := tsurara.NewClient({url:?})\n\tpet, err := ShowPetByID(context.Background(), c, \"1\", nil)\n\tif err != nil {{\n\t\tt.Fatal(err)\n\t}}\n\tif pet.Data.Name != \"Pochi\" {{\n\t\tt.Fatal(pet.Data)\n\t}}\n}}\n"
            ),
        )],
        tags: Vec::new(),
    }
    .check(&out)
    .unwrap();
}

#[test]
fn check_reports_edits_and_leaving_vendoring_deletes_the_copy() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(repo_root().join(PETSTORE), dir.path().join("openapi.yaml")).unwrap();
    let config = dir.path().join("tsurara.config.yaml");
    let tsurara = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_tsurara"))
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap()
    };
    std::fs::write(
        &config,
        "input: openapi.yaml\noutput: out\nruntime: vendor\n",
    )
    .unwrap();
    assert_ok("generate", &tsurara(&["generate"]));
    let copy = dir.path().join("out/_runtime/client.ts");
    let original = std::fs::read_to_string(&copy).unwrap();
    assert_ok("check", &tsurara(&["check"]));

    std::fs::write(&copy, format!("{original}\n// edited\n")).unwrap();
    assert!(!tsurara(&["check"]).status.success());
    assert_ok("generate", &tsurara(&["generate"]));
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), original);

    std::fs::write(&config, "input: openapi.yaml\noutput: out\n").unwrap();
    assert_ok("generate", &tsurara(&["generate"]));
    assert!(!dir.path().join("out/_runtime").join("index.ts").exists());
    let index = std::fs::read_to_string(dir.path().join("out/index.ts")).unwrap();
    assert!(index.contains("// @tsurara/client "), "{index}");
}

#[test]
fn go_runtime_path_comes_from_go_mod() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(repo_root().join(PETSTORE), dir.path().join("openapi.yaml")).unwrap();
    std::fs::write(
        dir.path().join("go.mod"),
        "module example.com/app\n\ngo 1.24\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tsurara.config.yaml"),
        "input: openapi.yaml\noutput: internal/petstore\nlanguage: go\nruntime: vendor\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_ok("generate", &out);
    let op =
        std::fs::read_to_string(dir.path().join("internal/petstore/list_pets_op_gen.go")).unwrap();
    assert!(
        op.contains("\"example.com/app/internal/petstore/internal/tsurara\""),
        "{op}"
    );
}
