//! The manifest of each built-in backend names identifiers its files really export: a
//! file that imports every identifier of the manifest from the file the manifest names passes the
//! language's checker (tsc, pyright, cargo clippy, go build).

use tsurara_backend::manifest::Manifest;
use tsurara_backend::{Backend, OutputFile, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, repo_root,
};

/// Petstore, and the naming edge cases (collisions, reserved words, renamed derived names).
fn definitions() -> Vec<(&'static str, DocumentSet)> {
    [
        "crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml",
        "corpus/edge-cases/names.yaml",
    ]
    .into_iter()
    .map(|path| {
        let text = std::fs::read(repo_root().join(path)).unwrap();
        (
            path,
            DocumentSet::from_value(parse_document(&text).unwrap()),
        )
    })
    .collect()
}

fn generated(backend: &dyn Backend, set: &DocumentSet) -> (Vec<(String, String)>, Manifest) {
    let out = generate(backend, set, &Options::default()).unwrap();
    let manifest = out.manifest.expect("built-in backends return a manifest");
    assert!(!manifest.types.is_empty() && !manifest.operations.is_empty());
    let files = out
        .files
        .into_iter()
        .map(|OutputFile { path, contents }| (path, contents))
        .collect();
    (files, manifest)
}

/// Every export of the manifest, with whether it is a value (a function or an OperationDef).
fn exports(manifest: &Manifest) -> Vec<(&str, &str, bool)> {
    let mut out: Vec<(&str, &str, bool)> = manifest
        .types
        .iter()
        .map(|t| (t.export.identifier.as_str(), t.export.file.as_str(), false))
        .collect();
    for op in &manifest.operations {
        for e in op.function.iter().chain(&op.operation_def) {
            out.push((&e.identifier, &e.file, true));
        }
        for e in op.types.values() {
            out.push((&e.identifier, &e.file, false));
        }
    }
    out
}

#[test]
fn typescript_manifest_names_exported_identifiers() {
    for (path, set) in definitions() {
        let (mut files, manifest) =
            generated(&tsurara_lang_typescript::TypeScript::default(), &set);
        let mut check = String::new();
        let mut used = Vec::new();
        for (i, (ident, file, value)) in exports(&manifest).into_iter().enumerate() {
            let module = file.strip_suffix(".ts").unwrap();
            let kind = if value { "" } else { "type " };
            check.push_str(&format!(
                "import {kind}{{ {ident} as E{i} }} from './{module}.js';\n"
            ));
            used.push(if value {
                format!("typeof E{i}")
            } else {
                format!("E{i}")
            });
        }
        check.push_str(&format!("\nexport type Exports = [{}];\n", used.join(", ")));
        files.push(("manifest_check.ts".into(), check));
        if let Err(e) = (TypeScriptChecker {
            options: TsOptions::default(),
        })
        .check(&files)
        {
            panic!("{path}: {e}");
        }
    }
}

#[test]
fn python_manifest_names_exported_identifiers() {
    for (path, set) in definitions() {
        let (mut files, manifest) = generated(&tsurara_lang_python::Python::default(), &set);
        let mut check = String::new();
        let mut used = Vec::new();
        for (i, (ident, file, _)) in exports(&manifest).into_iter().enumerate() {
            let module = file.strip_suffix(".py").unwrap().replace('/', ".");
            check.push_str(&format!("from .{module} import {ident} as E{i}\n"));
            used.push(format!("E{i}"));
        }
        check.push_str(&format!("\nEXPORTS = ({},)\n", used.join(", ")));
        files.push(("manifest_check.py".into(), check));
        let checker = PythonChecker {
            package: "api".into(),
        };
        if let Err(e) = checker.check(&files) {
            panic!("{path}: {e}");
        }
    }
}

#[test]
fn rust_manifest_names_exported_identifiers() {
    for (path, set) in definitions() {
        let (files, manifest) = generated(&tsurara_lang_rust::Rust::default(), &set);
        // Types are re-exported from `types`, functions and their types from `operations`.
        let mut test = String::from("#![allow(unused_imports)]\n\n");
        for (ident, file, _) in exports(&manifest) {
            let module = file.split('/').next().unwrap();
            test.push_str(&format!("use tsurara_generated::api::{module}::{ident};\n"));
        }
        test.push_str("\n#[test]\nfn exports() {}\n");
        let checker = RustChecker {
            tests: vec![("manifest".into(), test)],
            http: false,
            mocks: false,
            ..Default::default()
        };
        if let Err(e) = checker.check(&files) {
            panic!("{path}: {e}");
        }
    }
}

#[test]
fn go_manifest_names_exported_identifiers() {
    for (path, set) in definitions() {
        let (files, manifest) = generated(&tsurara_lang_go::Go::default(), &set);
        // One package: the file does not matter to Go, but it must be one of the output files.
        let mut test = String::from("package api\n\nimport \"testing\"\n\n");
        for (ident, file, value) in exports(&manifest) {
            assert!(
                files.iter().any(|(p, _)| p == file),
                "{file} is not generated"
            );
            if value {
                test.push_str(&format!("var _ = {ident}\n\n"));
            } else {
                test.push_str(&format!("var _ *{ident}\n\n"));
            }
        }
        test.push_str("func TestExports(t *testing.T) {}\n");
        let checker = GoChecker {
            tests: vec![("manifest".into(), test)],
            tags: Vec::new(),
        };
        if let Err(e) = checker.check(&files) {
            panic!("{path}: {e}");
        }
    }
}
