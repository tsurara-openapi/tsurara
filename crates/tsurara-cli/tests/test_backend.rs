//! `tsurara test-backend` and `tsurara spec`: the example backend of 134 passes with its
//! checker, the same backend with a naming bug fails with the definition named, and `spec` writes the
//! runtime contract of this version.

use std::process::{Command, Output};
use tsurara_testkit::repo_root;

fn test_backend(env: &[(&str, &str)]) -> Output {
    let example = repo_root().join("examples/backend-typescript");
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsurara"));
    cmd.arg("test-backend")
        .arg("--backend")
        .arg("node backend.mjs")
        .arg("--check")
        .arg("node check.mjs")
        // Relative arguments resolve against the current directory.
        .current_dir(&example);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

fn has_node() -> bool {
    Command::new("node").arg("--version").output().is_ok()
}

#[test]
fn the_example_backend_passes() {
    if !has_node() {
        return;
    }
    let out = test_backend(&[]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("ok   edge-cases/names.yaml"), "{stdout}");
    assert!(stdout.contains(", 0 failed"), "{stdout}");
}

#[test]
fn a_naming_bug_fails_with_the_definition_named() {
    if !has_node() {
        return;
    }
    let out = test_backend(&[("EXAMPLE_NAMING_BUG", "1")]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!out.status.success(), "{stdout}");
    assert!(
        stdout.contains("FAIL edge-cases/names.yaml (kept in "),
        "{stdout}"
    );
    assert!(stdout.contains("PetStatus is already declared"), "{stdout}");
    // The failing definition is kept for reproduction.
    let kept = stdout
        .lines()
        .find_map(|l| l.strip_prefix("FAIL edge-cases/names.yaml (kept in "))
        .and_then(|l| l.strip_suffix(')'))
        .unwrap();
    let kept = std::path::Path::new(kept);
    assert!(kept.join("openapi.yaml").exists() && kept.join("tsurara.config.yaml").exists());
    let _ = std::fs::remove_dir_all(kept.parent().unwrap());
}

#[test]
fn definitions_from_a_directory_are_added() {
    if !has_node() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    std::fs::copy(
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml"),
        dir.path().join("petstore.yaml"),
    )
    .unwrap();
    let example = repo_root().join("examples/backend-typescript");
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .args(["test-backend", "--backend"])
        .arg(format!("node {}", example.join("backend.mjs").display()))
        .arg("--definitions")
        .arg(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{stdout}");
    assert!(stdout.contains("petstore.yaml"), "{stdout}");
}

#[test]
fn spec_writes_the_runtime_contract() {
    let dir = tempfile::tempdir().unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("spec")
        .arg(dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());
    for path in [
        "README.md",
        "operation-def.schema.json",
        "sem-model.schema.json",
        "plugin-protocol.schema.json",
        "conformance/requests.json",
        "conformance/typedefs.json",
    ] {
        assert_eq!(
            std::fs::read_to_string(dir.path().join(path)).unwrap(),
            std::fs::read_to_string(repo_root().join("spec").join(path)).unwrap(),
            "{path}"
        );
    }
}
