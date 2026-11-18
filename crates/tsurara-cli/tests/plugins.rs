//! Plugin modules written with `@tsurara/plugin` (`plugins`), run through the binary with
//! Node.

use std::path::Path;
use std::process::{Command, Output};
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root, tsc_path};

const OPENAPI: &str = r##"
openapi: 3.1.0
info: { title: pets, version: "1" }
paths:
  /pets/{id}:
    get:
      operationId: getPet
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
components:
  schemas:
    Pet: { type: object, required: [id], properties: { id: { type: string } } }
"##;

fn node_ok() -> bool {
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

/// A project with `@tsurara/plugin` built into `node_modules`.
fn project(config_extra: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let sdk = dir.path().join("node_modules/@tsurara/plugin");
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
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("openapi.yaml"), OPENAPI).unwrap();
    std::fs::write(
        dir.path().join("tsurara.config.yaml"),
        format!("input: openapi.yaml\noutput: out\n{config_extra}"),
    )
    .unwrap();
    dir
}

fn tsurara(dir: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(dir)
        .output()
        .unwrap()
}

const FOUR_STAGES: &str = r#"import { appendFileSync } from 'node:fs';
import { definePlugin } from '@tsurara/plugin';

const seen = (stage: string): void => appendFileSync('stages.log', `${process.pid} ${stage}\n`);

export default definePlugin({
  name: 'four',
  transformDocument(document) {
    seen('transformDocument');
    const d = document as { info: { description?: string } };
    d.info.description = 'from the plugin';
    return d;
  },
  transformModel(model) {
    seen('transformModel');
    return { ...model, title: 'renamed' };
  },
  extend({ manifest, marker, options }) {
    seen('extend');
    const ext = (options as { importExtension: string }).importExtension;
    return [{ path: 'added.ts', contents: `${marker}\n// ${manifest?.operations[0]?.function?.identifier} ${ext}\n` }];
  },
  transformFiles(files) {
    seen('transformFiles');
    return files.map((f) => (f.path === 'added.ts' ? { ...f, contents: `${f.contents}// last\n` } : f));
  },
});
"#;

#[test]
fn a_plugin_takes_part_in_four_stages_in_one_process() {
    if !node_ok() {
        return;
    }
    let dir = project("plugins:\n  - ./four.plugin.ts\n");
    std::fs::write(dir.path().join("four.plugin.ts"), FOUR_STAGES).unwrap();
    let out = tsurara(dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let log = std::fs::read_to_string(dir.path().join("stages.log")).unwrap();
    let pids: std::collections::BTreeSet<&str> =
        log.lines().filter_map(|l| l.split(' ').next()).collect();
    let stages: Vec<&str> = log.lines().filter_map(|l| l.split(' ').nth(1)).collect();
    assert_eq!(
        stages,
        [
            "transformDocument",
            "transformModel",
            "extend",
            "transformFiles"
        ]
    );
    assert_eq!(pids.len(), 1, "{log}");
    let added = std::fs::read_to_string(dir.path().join("out/added.ts")).unwrap();
    assert!(added.contains("// getPet .js\n// last"), "{added}");
}

#[test]
fn a_plugin_of_another_protocol_fails_naming_both() {
    if !node_ok() {
        return;
    }
    let dir = project("plugins:\n  - { module: ./old.mjs, runtime: [node, ./old.mjs] }\n");
    std::fs::write(
        dir.path().join("old.mjs"),
        "process.stdin.once('data', () => { process.stdout.write(JSON.stringify({ type: 'hello', protocol: 99, name: 'old', stages: [] }) + '\\n'); });\n",
    )
    .unwrap();
    let out = tsurara(dir.path());
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("plugin protocol 99") && err.contains("tsurara speaks 1"),
        "{err}"
    );
}

#[test]
fn the_swr_example_generates_hooks_that_type_check() {
    if !node_ok() {
        return;
    }
    let example = repo_root().join("examples/plugin-swr");
    let dir = project("plugins:\n  - ./tsurara.plugin.ts\n");
    std::fs::copy(
        example.join("tsurara.plugin.ts"),
        dir.path().join("tsurara.plugin.ts"),
    )
    .unwrap();
    std::fs::copy(
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml"),
        dir.path().join("openapi.yaml"),
    )
    .unwrap();
    let out = tsurara(dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut files = Vec::new();
    let root = dir.path().join("out");
    collect(&root, &root, &mut files);
    assert!(
        files.iter().any(|(p, _)| p == "swr/useShowPetById.ts"),
        "{:?}",
        files.iter().map(|f| &f.0).collect::<Vec<_>>()
    );
    assert_tsc_passes(&files, &TsOptions::default());
}

fn collect(root: &Path, dir: &Path, out: &mut Vec<(String, String)>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(root, &path, out);
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((rel, std::fs::read_to_string(&path).unwrap()));
        }
    }
}

#[test]
fn init_writes_a_plugin_skeleton_that_runs() {
    if !node_ok() {
        return;
    }
    let dir = project("plugins:\n  - ./tsurara.plugin.ts\n");
    let init = Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .args(["init", "--plugin"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    let out = tsurara(dir.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let extra = std::fs::read_to_string(dir.path().join("out/extra/getPet.ts")).unwrap();
    assert!(
        extra.contains("import { getPet } from '../operations/getPet.js';"),
        "{extra}"
    );
}

/// `runPlugin` of `@tsurara/plugin/testing` runs the binary with a plugin and returns the files.
#[test]
fn run_plugin_returns_the_files() {
    if !node_ok() {
        return;
    }
    let dir = project("");
    std::fs::write(dir.path().join("four.plugin.ts"), FOUR_STAGES).unwrap();
    std::fs::write(
        dir.path().join("run.ts"),
        format!(
            "import {{ runPlugin }} from '@tsurara/plugin/testing';\nimport {{ readFileSync }} from 'node:fs';\nconst files = runPlugin(new URL('./four.plugin.ts', import.meta.url).pathname, {{ definition: readFileSync('openapi.yaml', 'utf8'), bin: {:?} }});\nconsole.log(files.get('added.ts'));\n",
            env!("CARGO_BIN_EXE_tsurara")
        ),
    )
    .unwrap();
    let out = Command::new("node")
        .arg("run.ts")
        .current_dir(dir.path())
        .output()
        .unwrap();
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("// last"),
        "{stdout}{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
