//! Corpus tests (design.md 13.1): generates from the edge-case definitions in
//! `corpus/edge-cases` and from the real-world definitions in `corpus/sources.json`, with the `tsurara`
//! binary, and passes the output through every language's checker: `tsc` (7.0.2 and 5.8) with each
//! tsconfig of the matrix in design.md chapter 13, pyright, `cargo clippy`, and gofmt, go vet and
//! go build.
//!
//! The real-world definitions compile as Rust in a few minutes (Stripe's 11,400 types in about
//! 100 seconds of `cargo clippy` on a laptop), so that test runs on every push too.
//!
//! The real-world definitions are downloaded by `node scripts/fetch-corpus.mjs` into
//! `target/corpus`, and checked with `TSURARA_CORPUS=1` (most of the suite's time, so not on every
//! local run) or `TSURARA_REQUIRE_CORPUS=1` (CI's corpus job, where missing ones fail). Checks whose
//! files and inputs passed before are skipped by the testkit's cache.

use std::path::{Path, PathBuf};
use std::process::Command;
use tsurara_testkit::{
    Checker, GoChecker, PythonChecker, RustChecker, TsOptions, TypeScriptChecker, repo_root,
};

struct Entry {
    name: String,
    path: PathBuf,
}

fn edge_cases() -> Vec<Entry> {
    let dir = repo_root().join("corpus/edge-cases");
    let mut entries: Vec<Entry> = std::fs::read_dir(&dir)
        .expect("corpus/edge-cases")
        .map(|e| e.expect("entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .map(|path| Entry {
            name: path.file_stem().unwrap().to_string_lossy().into_owned(),
            path,
        })
        .collect();
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    assert!(!entries.is_empty());
    entries
}

fn real_world() -> Vec<Entry> {
    let sources: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("corpus/sources.json")).expect("sources.json"),
    )
    .expect("json");
    let required = std::env::var("TSURARA_REQUIRE_CORPUS").is_ok_and(|v| v == "1");
    // The real-world definitions take most of the time of the test suite; they run when asked for
    // (`TSURARA_CORPUS=1`) or required (CI's corpus job), not on every local `cargo test`.
    if !required && !std::env::var("TSURARA_CORPUS").is_ok_and(|v| v == "1") {
        eprintln!("skipping the real-world definitions: set TSURARA_CORPUS=1 to check them");
        return Vec::new();
    }
    let mut entries = Vec::new();
    for source in sources["sources"].as_array().expect("sources") {
        let name = source["name"].as_str().expect("name").to_owned();
        let path = repo_root()
            .join("target/corpus")
            .join(source["file"].as_str().expect("file"));
        if path.exists() {
            entries.push(Entry { name, path });
        } else if required {
            panic!("{name} is missing; run `node scripts/fetch-corpus.mjs`");
        } else {
            eprintln!("skipping {name}: run `node scripts/fetch-corpus.mjs` to download it");
        }
    }
    entries
}

/// Runs `tsurara generate` for one definition and returns the written files, relative to the output
/// directory.
fn generate(entry: &Entry, language: &str) -> Vec<(String, String)> {
    generate_with(entry, language, "")
}

/// `generate` with more configuration (YAML appended to the file).
fn generate_with(entry: &Entry, language: &str, extra: &str) -> Vec<(String, String)> {
    let dir = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        dir.path().join("tsurara.config.yaml"),
        format!(
            "input: {}\noutput: out\nlanguage: {language}\n{extra}",
            serde_json::to_string(&entry.path.to_string_lossy()).unwrap()
        ),
    )
    .expect("write config");
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(dir.path())
        .output()
        .expect("run tsurara");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "{} ({language}): tsurara generate failed:\n{stderr}",
        entry.name
    );
    // A reference that does not resolve in these definitions is a bug in tsurara, not in them.
    assert!(
        !stderr.contains("warning[unresolved-ref]"),
        "{} ({language}): unresolved references:\n{stderr}",
        entry.name
    );
    let root = dir.path().join("out");
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    assert!(!files.is_empty(), "{}: no files", entry.name);
    files
}

fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, String)>) {
    for e in std::fs::read_dir(dir).expect("read dir") {
        let path = e.expect("entry").path();
        if path.is_dir() {
            collect(root, &path, files);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            files.push((rel, std::fs::read_to_string(&path).expect("utf-8")));
        }
    }
}

fn check_typescript(entries: &[Entry]) {
    let mut failures = Vec::new();
    for entry in entries {
        let files = generate(entry, "typescript");
        for (label, options) in TsOptions::matrix() {
            if let Err(e) = (TypeScriptChecker { options }).check(&files) {
                failures.push(format!("{} with {label}: {e}", entry.name));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

fn check_python(entries: &[Entry]) {
    let checker = PythonChecker {
        package: "api".into(),
    };
    let mut failures = Vec::new();
    for entry in entries {
        let files = generate(entry, "python");
        if let Err(e) = checker.check(&files) {
            failures.push(format!("{}: {e}", entry.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn edge_cases_pass_tsc() {
    check_typescript(&edge_cases());
}

#[test]
fn edge_cases_pass_pyright() {
    check_python(&edge_cases());
}

#[test]
fn real_world_definitions_pass_tsc() {
    check_typescript(&real_world());
}

#[test]
fn real_world_definitions_pass_pyright() {
    check_python(&real_world());
}

/// The Zod schemas (`typescript.validators: zod`) and the TanStack Query options
/// (`typescript.query: tanstack`) with the strictest tsconfig.
fn check_zod(entries: &[Entry]) {
    let mut failures = Vec::new();
    for entry in entries {
        let files = generate_with(
            entry,
            "typescript",
            "typescript:\n  validators: zod\n  validatorConstraints: true\n  query: tanstack\n  mocks: msw\n",
        );
        if let Err(e) = (TypeScriptChecker {
            options: TsOptions::default(),
        })
        .check(&files)
        {
            failures.push(format!(
                "{} with zod, query options and mocks: {e}",
                entry.name
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn edge_cases_pass_tsc_with_zod_schemas() {
    check_zod(&edge_cases());
}

/// The routes for `mockFetch` (`typescript.mocks: fetch`); the MSW handlers are checked
/// with the Zod schemas above.
#[test]
fn edge_cases_pass_tsc_with_fetch_mocks() {
    let mut failures = Vec::new();
    for entry in &edge_cases() {
        let files = generate_with(entry, "typescript", "typescript:\n  mocks: fetch\n");
        if let Err(e) = (TypeScriptChecker {
            options: TsOptions::default(),
        })
        .check(&files)
        {
            failures.push(format!("{} with fetch mocks: {e}", entry.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn real_world_zod_schemas_pass_tsc() {
    check_zod(&real_world());
}

fn check_rust(entries: &[Entry]) {
    let mut failures = Vec::new();
    for entry in entries {
        let files = generate(entry, "rust");
        if let Err(e) = RustChecker::default().check(&files) {
            failures.push(format!("{}: {e}", entry.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn edge_cases_compile_as_rust() {
    check_rust(&edge_cases());
}

#[test]
fn real_world_definitions_compile_as_rust() {
    check_rust(&real_world());
}

fn check_go(entries: &[Entry]) {
    let mut failures = Vec::new();
    for entry in entries {
        let files = generate(entry, "go");
        if let Err(e) = GoChecker::default().check(&files) {
            failures.push(format!("{}: {e}", entry.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn edge_cases_build_as_go() {
    check_go(&edge_cases());
}

#[test]
fn real_world_definitions_build_as_go() {
    check_go(&real_world());
}

/// Generated mocks of Python, Rust and Go (`python.mocks`, `rust.mocks`, `go.mocks`) on the
/// edge cases, with each language's checker (Rust and Go with their mock feature and tag).
#[test]
fn edge_cases_pass_the_checkers_with_mocks() {
    let mut failures = Vec::new();
    for entry in &edge_cases() {
        let py = generate_with(entry, "python", "python:\n  mocks: true\n");
        if let Err(e) = (PythonChecker {
            package: "api".into(),
        })
        .check(&py)
        {
            failures.push(format!("{} (python mocks): {e}", entry.name));
        }
        let rs = generate_with(entry, "rust", "rust:\n  mocks: true\n");
        if let Err(e) = (RustChecker {
            tests: Vec::new(),
            http: false,
            mocks: true,
            ..Default::default()
        })
        .check(&rs)
        {
            failures.push(format!("{} (rust mocks): {e}", entry.name));
        }
        let go = generate_with(entry, "go", "go:\n  package: api\n  mocks: true\n");
        if let Err(e) = (GoChecker {
            tests: Vec::new(),
            tags: vec!["tsurara_mocks".into()],
        })
        .check(&go)
        {
            failures.push(format!("{} (go mocks): {e}", entry.name));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
