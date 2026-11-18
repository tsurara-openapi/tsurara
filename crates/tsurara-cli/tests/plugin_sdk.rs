//! The plugin SDKs' types come from the protocol schema: tsurara generates them from
//! `spec/plugin-protocol.schema.json` itself. `TSURARA_UPDATE_SCHEMA=1` rewrites them, and the test
//! fails when they are out of date.

use serde_json::{Value, json};
use tsurara_lang_typescript::options::{ImportExtension, Layout};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::DocumentSet;
use tsurara_sem::Options;
use tsurara_testkit::repo_root;

/// An OpenAPI document with the `$defs` of the protocol schema as components.
pub fn protocol_document() -> Value {
    let schema: Value = serde_json::from_str(
        &std::fs::read_to_string(repo_root().join("spec/plugin-protocol.schema.json")).unwrap(),
    )
    .unwrap();
    let text = serde_json::to_string(&schema["$defs"])
        .unwrap()
        .replace("\"#/$defs/", "\"#/components/schemas/");
    let defs: Value = serde_json::from_str(&text).unwrap();
    json!({
        "openapi": "3.1.0",
        "info": { "title": "tsurara plugin protocol", "version": "1" },
        "paths": {},
        "components": { "schemas": defs },
    })
}

#[test]
fn typescript_sdk_types_are_up_to_date() {
    let set = DocumentSet::from_value(protocol_document());
    let opts = TypeScriptOptions {
        layout: Layout::Single,
        import_extension: ImportExtension::Ts,
        ..TypeScriptOptions::default()
    };
    let out = generate(&set, &types_only(), &opts).unwrap();
    let index = out
        .files
        .iter()
        .find(|f| f.path == "index.ts")
        .expect("one file")
        .contents
        .clone();
    let path = repo_root().join("packages/plugin/src/generated/protocol.ts");
    if std::env::var_os("TSURARA_UPDATE_SCHEMA").is_some() {
        std::fs::write(&path, &index).unwrap();
        return;
    }
    assert!(
        std::fs::read_to_string(&path).unwrap_or_default() == index,
        "packages/plugin/src/generated/protocol.ts is out of date; run `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test plugin_sdk`"
    );
}

/// Writes `files` into `dir` (`TSURARA_UPDATE_SCHEMA=1`, replacing what is there) or checks that
/// `dir` holds exactly them.
fn check_dir(dir: &std::path::Path, files: &[(String, String)], what: &str) {
    if std::env::var_os("TSURARA_UPDATE_SCHEMA").is_some() {
        let _ = std::fs::remove_dir_all(dir);
        for (path, contents) in files {
            let full = dir.join(path);
            std::fs::create_dir_all(full.parent().unwrap()).unwrap();
            std::fs::write(full, contents).unwrap();
        }
        return;
    }
    let mut on_disk = Vec::new();
    fn walk(root: &std::path::Path, dir: &std::path::Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).into_iter().flatten() {
            let p = e.unwrap().path();
            if p.is_dir() {
                if p.file_name().is_some_and(|n| n == "__pycache__") {
                    continue;
                }
                walk(root, &p, out);
            } else {
                out.push(
                    p.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                );
            }
        }
    }
    walk(dir, dir, &mut on_disk);
    on_disk.sort();
    let mut want: Vec<String> = files.iter().map(|(p, _)| p.clone()).collect();
    want.sort();
    let same = on_disk == want
        && files
            .iter()
            .all(|(p, c)| std::fs::read_to_string(dir.join(p)).unwrap_or_default() == *c);
    assert!(
        same,
        "{what} is out of date; run `TSURARA_UPDATE_SCHEMA=1 cargo test -p tsurara-cli --test plugin_sdk`"
    );
}

#[test]
fn python_and_go_sdk_types_are_up_to_date() {
    let set = DocumentSet::from_value(protocol_document());
    let pairs = |o: tsurara_backend::Output| -> Vec<(String, String)> {
        o.files.into_iter().map(|f| (f.path, f.contents)).collect()
    };
    let py = tsurara_lang_python::generate(&set, &types_only(), &Default::default()).unwrap();
    check_dir(
        &repo_root().join("packages/plugin-python/src/tsurara_plugin/protocol"),
        &pairs(py),
        "packages/plugin-python/src/tsurara_plugin/protocol",
    );
    let go = tsurara_lang_go::generate(
        &set,
        &types_only(),
        &tsurara_lang_go::GoOptions {
            package: "protocol".into(),
            ..Default::default()
        },
    )
    .unwrap();
    check_dir(
        &repo_root().join("packages/plugin-go/protocol"),
        &pairs(go),
        "packages/plugin-go/protocol",
    );
}

/// The protocol has no operations: its types need only the types layer of each runtime.
fn types_only() -> Options {
    Options {
        operations: false,
        ..Options::default()
    }
}
