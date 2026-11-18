//! The plugin SDKs: the same small plugin (one file per operation with its function name)
//! written with `@tsurara/plugin`, `tsurara-plugin` for Python, the Go SDK and the Rust crate produces
//! identical files on petstore.

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;
use tsurara_testkit::{python_or_skip, repo_root, tsc_path};

const TS: &str = r#"import { definePlugin } from '@tsurara/plugin';

export default definePlugin({
  name: 'functions',
  extend({ manifest, marker }) {
    return (manifest?.operations ?? []).flatMap((op) =>
      op.function == null
        ? []
        : [{ path: `functions/${op.operationId}.txt`, contents: `${marker}\n${op.function.identifier}\n` }],
    );
  },
});
"#;

const PYTHON: &str = r#"from tsurara_plugin import ExtendInput, OutputFile, define_plugin


def extend(input: ExtendInput) -> list[OutputFile]:
    if input.manifest is None:
        return []
    return [
        OutputFile(path=f"functions/{op.operation_id}.txt", contents=f"{input.marker}\n{op.function.identifier}\n")
        for op in input.manifest.operations
        if op.function is not None
    ]


plugin = define_plugin(name="functions", extend=extend)
"#;

const GO: &str = r#"package main

import (
	tsuraraplugin "github.com/tsurara-openapi/tsurara/packages/plugin-go"
	"github.com/tsurara-openapi/tsurara/packages/plugin-go/protocol"
)

func main() {
	tsuraraplugin.Run(tsuraraplugin.Plugin{
		Name: "functions",
		Extend: func(in protocol.ExtendInput) ([]protocol.OutputFile, error) {
			var out []protocol.OutputFile
			if in.Manifest == nil || in.Manifest.Manifest == nil {
				return out, nil
			}
			for _, op := range in.Manifest.Manifest.Operations {
				if op.Function == nil || op.Function.Export == nil {
					continue
				}
				out = append(out, protocol.OutputFile{
					Path:     "functions/" + op.OperationID + ".txt",
					Contents: in.Marker + "\n" + op.Function.Export.Identifier + "\n",
				})
			}
			return out, nil
		},
	})
}
"#;

const RUST: &str = r#"use tsurara_plugin::{OutputFile, Plugin, run};

fn main() {
    let mut plugin = Plugin::new("functions");
    plugin.extend = Some(Box::new(|input| {
        let Some(manifest) = input.manifest else {
            return Ok(Vec::new());
        };
        Ok(manifest
            .operations
            .iter()
            .filter_map(|op| {
                let f = op.function.as_ref()?;
                Some(OutputFile {
                    path: format!("functions/{}.txt", op.operation_id),
                    contents: format!("{}\n{}\n", input.marker, f.identifier),
                })
            })
            .collect())
    }));
    run(plugin);
}
"#;

fn write(dir: &Path, path: &str, contents: &str) {
    let full = dir.join(path);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(full, contents).unwrap();
}

/// Generates petstore with one plugin and returns the plugin's files.
fn run_with(dir: &Path, plugin: &str, env: &[(&str, String)]) -> BTreeMap<String, String> {
    std::fs::copy(
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml"),
        dir.join("openapi.yaml"),
    )
    .unwrap();
    write(
        dir,
        "tsurara.config.yaml",
        &format!("input: openapi.yaml\noutput: out\nplugins:\n  - {plugin}\n"),
    );
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_tsurara"));
    cmd.arg("generate").current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().unwrap();
    assert!(
        out.status.success(),
        "{plugin}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let functions = dir.join("out/functions");
    std::fs::read_dir(&functions)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect()
}

#[test]
fn every_sdk_produces_the_same_files() {
    let node = Command::new("node").arg("--version").output().is_ok();
    let Some(python) = python_or_skip() else {
        return;
    };
    if !node {
        return;
    }
    let mut results = Vec::new();

    // TypeScript, with the SDK built into node_modules.
    let ts = tempfile::tempdir().unwrap();
    let sdk = ts.path().join("node_modules/@tsurara/plugin");
    let built = Command::new(tsc_path().expect("tsc"))
        .arg("-p")
        .arg(repo_root().join("packages/plugin/tsconfig.build.json"))
        .arg("--outDir")
        .arg(sdk.join("dist"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stdout)
    );
    std::fs::copy(
        repo_root().join("packages/plugin/package.json"),
        sdk.join("package.json"),
    )
    .unwrap();
    write(ts.path(), "package.json", r#"{ "type": "module" }"#);
    write(ts.path(), "plugin.ts", TS);
    results.push(("typescript", run_with(ts.path(), "./plugin.ts", &[])));

    // Python, with the SDK on PYTHONPATH and the runtime's virtual environment.
    let py = tempfile::tempdir().unwrap();
    write(py.path(), "plugin.py", PYTHON);
    let runtime = format!(
        "{{ module: ./plugin.py, runtime: [{}, -m, tsurara_plugin] }}",
        serde_json::to_string(&python.to_string_lossy()).unwrap()
    );
    let path = repo_root().join("packages/plugin-python/src");
    results.push((
        "python",
        run_with(
            py.path(),
            &runtime,
            &[("PYTHONPATH", path.to_string_lossy().into_owned())],
        ),
    ));

    // Go, as a main package in a module that replaces the SDK and the runtime with the repository's.
    let go = tempfile::tempdir().unwrap();
    write(
        go.path(),
        "plugin/go.mod",
        &format!(
            "module example.com/plugin\n\ngo 1.24\n\nrequire github.com/tsurara-openapi/tsurara/packages/plugin-go v0.0.0\n\nreplace github.com/tsurara-openapi/tsurara/packages/plugin-go => {}\n\nreplace github.com/tsurara-openapi/tsurara/runtimes/go => {}\n",
            repo_root().join("packages/plugin-go").display(),
            repo_root().join("runtimes/go").display()
        ),
    );
    write(go.path(), "plugin/main.go", GO);
    results.push((
        "go",
        run_with(
            go.path(),
            "{ module: ./plugin, runtime: [go, -C, ./plugin, run, .] }",
            &[
                ("GOFLAGS", "-mod=mod".into()),
                ("GOTOOLCHAIN", "local".into()),
            ],
        ),
    ));

    // Rust, as a binary crate that depends on the SDK crate.
    let rs = tempfile::tempdir().unwrap();
    write(
        rs.path(),
        "plugin/Cargo.toml",
        &format!(
            "[package]\nname = \"functions-plugin\"\nversion = \"0.0.0\"\nedition = \"2024\"\n\n[workspace]\n\n[dependencies]\ntsurara-plugin = {{ path = {:?} }}\n",
            repo_root().join("crates/tsurara-plugin").to_string_lossy()
        ),
    );
    write(rs.path(), "plugin/src/main.rs", RUST);
    let _ = std::fs::copy(
        repo_root().join("Cargo.lock"),
        rs.path().join("plugin/Cargo.lock"),
    );
    results.push((
        "rust",
        run_with(
            rs.path(),
            "{ module: ./plugin/Cargo.toml, runtime: [cargo, run, --quiet, --manifest-path] }",
            &[(
                "CARGO_TARGET_DIR",
                repo_root()
                    .join("target/plugin-sdk")
                    .to_string_lossy()
                    .into_owned(),
            )],
        ),
    ));

    let (first, expected) = &results[0];
    assert_eq!(expected.len(), 3, "{expected:?}");
    assert_eq!(
        expected["showPetById.txt"],
        "// Generated by tsurara. Do not edit.\nshowPetById\n"
    );
    for (name, files) in &results[1..] {
        assert_eq!(files, expected, "{name} differs from {first}");
    }
}
