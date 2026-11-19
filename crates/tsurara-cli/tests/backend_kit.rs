//! Shared naming and lowering for external backends: a Rust backend built against the
//! crates as published generates what the same code generates in-process, and a JavaScript backend
//! that declares Go's capabilities and a Go-like naming policy receives `tir` and names for the
//! edge-case corpus.

#[path = "../../../examples/backend-rust/src/backend.rs"]
mod backend;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::{Options, build_model};
use tsurara_testkit::repo_root;

/// The crates an external Rust backend depends on, in dependency order.
const CRATES: &[&str] = &[
    "tsurara-loader",
    "tsurara-sem",
    "tsurara-naming",
    "tsurara-pretty",
    "tsurara-lower",
    "tsurara-sample",
    "tsurara-backend",
];

fn edge_cases() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(repo_root().join("corpus/edge-cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "yaml"))
        .collect();
    paths.sort();
    paths
}

fn generate_with(dir: &Path, definition: &Path, language: &str) -> BTreeMap<String, String> {
    std::fs::copy(definition, dir.join("openapi.yaml")).unwrap();
    std::fs::write(
        dir.join("tsurara.config.yaml"),
        format!("input: openapi.yaml\noutput: out\nlanguage: {language}\n"),
    )
    .unwrap();
    let _ = std::fs::remove_dir_all(dir.join("out"));
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}: {}",
        definition.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    let mut files = BTreeMap::new();
    if let Ok(entries) = std::fs::read_dir(dir.join("out")) {
        for e in entries {
            let p = e.unwrap().path();
            files.insert(
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            );
        }
    }
    files
}

/// Packages the crates as `cargo package` would publish them and unpacks them, so that the example
/// builds against their published form (normalized manifests, no path dependencies).
fn packaged_crates(target: &Path) -> PathBuf {
    let mut cmd = Command::new("cargo");
    cmd.args(["package", "--no-verify", "--allow-dirty", "--quiet"])
        .current_dir(repo_root())
        .env("CARGO_TARGET_DIR", target);
    for c in CRATES {
        cmd.args(["-p", c]);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let unpacked = target.join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).unwrap();
    for c in CRATES {
        let tar = Command::new("tar")
            .arg("xzf")
            .arg(target.join(format!("package/{c}-{}.crate", env!("CARGO_PKG_VERSION"))))
            .arg("-C")
            .arg(&unpacked)
            .output()
            .unwrap();
        assert!(
            tar.status.success(),
            "{}",
            String::from_utf8_lossy(&tar.stderr)
        );
        let manifest = std::fs::read_to_string(
            unpacked.join(format!("{c}-{}/Cargo.toml", env!("CARGO_PKG_VERSION"))),
        )
        .unwrap();
        assert!(
            !manifest.contains("path = \".."),
            "{c} has a path dependency:\n{manifest}"
        );
    }
    unpacked
}

#[test]
fn rust_backend_on_published_crates_matches_in_process() {
    if Command::new("cargo").arg("--version").output().is_err() {
        return;
    }
    let target = repo_root().join("target/backend-kit");
    let unpacked = packaged_crates(&target);
    // The example as a user would have it, with crates.io patched to the packaged crates.
    let project = tempfile::tempdir().unwrap();
    let example = repo_root().join("examples/backend-rust");
    std::fs::create_dir_all(project.path().join("src")).unwrap();
    std::fs::copy(
        example.join("Cargo.toml"),
        project.path().join("Cargo.toml"),
    )
    .unwrap();
    for f in ["main.rs", "backend.rs"] {
        std::fs::copy(
            example.join("src").join(f),
            project.path().join("src").join(f),
        )
        .unwrap();
    }
    let mut config = String::from("[patch.crates-io]\n");
    for c in CRATES {
        config.push_str(&format!(
            "{c} = {{ path = {:?} }}\n",
            unpacked
                .join(format!("{c}-{}", env!("CARGO_PKG_VERSION")))
                .to_string_lossy()
        ));
    }
    std::fs::create_dir_all(project.path().join(".cargo")).unwrap();
    std::fs::write(project.path().join(".cargo/config.toml"), config).unwrap();
    let _ = std::fs::copy(
        repo_root().join("Cargo.lock"),
        project.path().join("Cargo.lock"),
    );
    let build = Command::new("cargo")
        .args(["build", "--quiet"])
        .current_dir(project.path())
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = target.join("debug/tsurara-backend-example");

    let dir = tempfile::tempdir().unwrap();
    let language = format!("{{ command: [{:?}] }}", binary.to_string_lossy());
    for path in edge_cases() {
        let external = generate_with(dir.path(), &path, &language);
        let mut set =
            DocumentSet::from_value(parse_document(&std::fs::read(&path).unwrap()).unwrap());
        tsurara_loader::normalize_swagger2(&mut set).unwrap();
        let model = build_model(&set, &Options::default()).unwrap();
        let (files, _) = backend::generate(&model);
        let in_process: BTreeMap<String, String> =
            files.into_iter().map(|f| (f.path, f.contents)).collect();
        assert_eq!(external, in_process, "{}", path.display());
    }
}

/// Every name in a scope is unique, also ignoring case for files.
fn assert_unique<'a>(what: &str, names: impl Iterator<Item = &'a str>, fold: bool) {
    let mut seen = std::collections::HashSet::new();
    for n in names {
        let key = if fold { n.to_lowercase() } else { n.to_owned() };
        assert!(seen.insert(key), "{what}: {n} is not unique");
    }
}

#[test]
fn javascript_backend_receives_lowered_types_and_names() {
    if Command::new("node").arg("--version").output().is_err() {
        return;
    }
    let script = repo_root().join("examples/backend-typescript/backend.mjs");
    let language = format!("{{ command: [node, {:?}] }}", script.to_string_lossy());
    let dir = tempfile::tempdir().unwrap();
    for path in edge_cases() {
        // The backend throws on a union, an intersection, an inline record or a base record.
        let files = generate_with(dir.path(), &path, &language);
        assert_unique(
            &format!("{} files", path.display()),
            files.keys().map(String::as_str),
            true,
        );
        let types: Vec<String> = files
            .values()
            .filter_map(|c| {
                c.lines()
                    .nth(2)?
                    .strip_prefix("type ")?
                    .split(' ')
                    .next()
                    .map(str::to_owned)
            })
            .collect();
        assert_eq!(types.len(), files.len(), "{}", path.display());
        assert_unique(
            &format!("{} types", path.display()),
            types.iter().map(String::as_str),
            false,
        );
        for (file, contents) in &files {
            let members: Vec<&str> = contents
                .lines()
                .filter(|l| l.starts_with('\t'))
                .filter_map(|l| l.trim().split(' ').next())
                .collect();
            assert_unique(
                &format!("{} {file}", path.display()),
                members.into_iter(),
                false,
            );
        }
    }
}

#[test]
fn naming_needs_lowering() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("backend.mjs"),
        "process.stdout.write(JSON.stringify({ protocol: 2, semVersions: [1], markers: { generated: '//' }, naming: { contexts: {} } }));\n",
    )
    .unwrap();
    std::fs::copy(
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml"),
        dir.path().join("openapi.yaml"),
    )
    .unwrap();
    std::fs::write(
        dir.path().join("tsurara.config.yaml"),
        "input: openapi.yaml\noutput: out\nlanguage: { command: [node, backend.mjs] }\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("without `lowering`"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
