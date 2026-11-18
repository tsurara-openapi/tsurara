//! Backends written as external programs (design.md 18.11).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BACKEND: &str = r#"
const [mode] = process.argv.slice(2);
const versions = process.env.SEM_VERSIONS ? JSON.parse(process.env.SEM_VERSIONS) : [1];
const protocol = Number(process.env.PROTOCOL ?? 1);
if (mode === 'describe') {
  process.stdout.write(JSON.stringify({
    protocol,
    semVersions: versions,
    markers: { generated: '# made by the test backend', sidecars: ['.uid'] },
  }));
} else if (mode === 'generate') {
  let s = '';
  process.stdin.on('data', (c) => (s += c));
  process.stdin.on('end', () => {
    const req = JSON.parse(s);
    const files = req.model.schemas
      .filter((d) => !d.writable)
      .map((d) => ({
        path: `${d.preferredName}.txt`,
        contents: `# made by the test backend\n${d.preferredName} ${req.options.suffix}\n`,
      }));
    if (req.protocol !== protocol) process.exit(3);
    const manifest = {
      manifestVersion: 1,
      language: 'test',
      types: req.model.schemas.map((d) => ({ name: d.id, identifier: d.preferredName, file: `${d.preferredName}.txt` })),
      operations: [],
    };
    process.stdout.write(JSON.stringify({ files, warnings: [], manifest }));
  });
} else {
  process.exit(2);
}
"#;

fn write(dir: &Path, path: &str, contents: &str) -> PathBuf {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

fn tsurara(cwd: &Path, env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_tsurara"));
    c.arg("generate").current_dir(cwd);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn has_node() -> bool {
    Command::new("node").arg("--version").output().is_ok()
}

fn setup(schemas: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(
        tmp.path(),
        "api/openapi.yaml",
        &format!(
            "openapi: 3.1.0\ninfo: {{ title: t, version: \"1\" }}\npaths: {{}}\ncomponents:\n  schemas:\n{schemas}"
        ),
    );
    write(tmp.path(), "backend/main.mjs", BACKEND);
    write(
        tmp.path(),
        "tsurara.config.yaml",
        "input: api/openapi.yaml\noutput: out\nlanguage:\n  command: [node, backend/main.mjs]\n  options: { suffix: ok }\n",
    );
    tmp
}

#[test]
fn an_external_backend_generates_and_its_marker_finds_stale_files() {
    if !has_node() {
        return;
    }
    let tmp = setup("    Pet: { type: string }\n    Tag: { type: string }\n");
    let out = tsurara(tmp.path(), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pet = std::fs::read_to_string(tmp.path().join("out/Pet.txt")).unwrap();
    assert_eq!(pet, "# made by the test backend\nPet ok\n");
    assert!(tmp.path().join("out/Tag.txt").exists());
    write(tmp.path(), "out/Tag.txt.uid", "uid");
    write(tmp.path(), "out/notes.txt", "written by hand\n");
    // Tag is removed from the definition: its file and sidecar are deleted, other files are kept.
    write(
        tmp.path(),
        "api/openapi.yaml",
        "openapi: 3.1.0\ninfo: { title: t, version: \"1\" }\npaths: {}\ncomponents:\n  schemas:\n    Pet: { type: string }\n",
    );
    let out = tsurara(tmp.path(), &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!tmp.path().join("out/Tag.txt").exists());
    assert!(!tmp.path().join("out/Tag.txt.uid").exists());
    assert!(tmp.path().join("out/notes.txt").exists());
}

#[test]
fn an_external_backend_must_accept_the_model_version() {
    if !has_node() {
        return;
    }
    let tmp = setup("    Pet: { type: string }\n");
    let out = tsurara(tmp.path(), &[("SEM_VERSIONS", "[99]")]);
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("model version 1"), "{stderr}");
    assert!(stderr.contains("99"), "{stderr}");
}

/// Writes the identifiers of the manifest it receives into `manifest.txt`.
const EXTEND: &str = r#"
let s = '';
process.stdin.on('data', (c) => (s += c));
process.stdin.on('end', () => {
  const { manifest, marker } = JSON.parse(s);
  const names = manifest ? manifest.types.map((t) => t.identifier).join(' ') : 'none';
  process.stdout.write(JSON.stringify([{ path: 'manifest.txt', contents: `${marker}\n${names}\n` }]));
});
"#;

#[test]
fn protocol_2_passes_the_backend_manifest_to_extend_hooks_and_protocol_1_passes_none() {
    if !has_node() {
        return;
    }
    let tmp = setup("    Pet: { type: string }\n");
    write(tmp.path(), "hooks/extend.mjs", EXTEND);
    let config = tmp.path().join("tsurara.config.yaml");
    let base = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{base}hooks:\n  extend: [node, hooks/extend.mjs]\n"),
    )
    .unwrap();
    for (protocol, expected) in [("2", "Pet"), ("1", "none")] {
        let out = tsurara(tmp.path(), &[("PROTOCOL", protocol)]);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let manifest = std::fs::read_to_string(tmp.path().join("out/manifest.txt")).unwrap();
        assert_eq!(
            manifest,
            format!("# made by the test backend\n{expected}\n"),
            "protocol {protocol}"
        );
    }
}
