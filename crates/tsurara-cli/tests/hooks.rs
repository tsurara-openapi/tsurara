//! External command hooks (design.md chapter 11), run through the `tsurara` binary with Node
//! scripts.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const OPENAPI: &str = r##"
openapi: 3.1.0
info: { title: pets, version: "1" }
paths: {}
components:
  schemas:
    Pet: { type: object, properties: { id: { type: string } } }
"##;

fn write(dir: &Path, path: &str, contents: &str) -> PathBuf {
    let path = dir.join(path);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, contents).unwrap();
    path
}

fn tsurara(cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_tsurara"))
        .arg("generate")
        .current_dir(cwd)
        .output()
        .unwrap()
}

fn setup(hooks: &str) -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    write(tmp.path(), "api/openapi.yaml", OPENAPI);
    write(
        tmp.path(),
        "tsurara.config.yaml",
        &format!("input: api/openapi.yaml\noutput: src/generated\nhooks:\n{hooks}"),
    );
    tmp
}

/// A Node script that reads JSON from stdin, applies `body` to `value`, and writes it to stdout.
fn json_script(body: &str) -> String {
    format!(
        "let s = ''; process.stdin.on('data', (c) => (s += c)); process.stdin.on('end', () => {{ const value = JSON.parse(s); {body}; process.stdout.write(JSON.stringify(value)); }});\n"
    )
}

fn has_node() -> bool {
    Command::new("node").arg("--version").output().is_ok()
}

#[test]
fn transform_document_rewrites_the_definition() {
    if !has_node() {
        return;
    }
    let tmp = setup("  transformDocument: [node, hooks/add.mjs]\n");
    write(
        tmp.path(),
        "hooks/add.mjs",
        &json_script("value.components.schemas.Added = { type: 'string' }"),
    );
    let out = tsurara(tmp.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let added = tmp.path().join("src/generated/schemas/Added.ts");
    let contents = std::fs::read_to_string(added).unwrap();
    assert!(
        contents.contains("export type Added = string;"),
        "{contents}"
    );
}

#[test]
fn transform_files_and_after_write_run_in_order() {
    if !has_node() {
        return;
    }
    let tmp = setup(
        "  transformFiles:\n    - [node, hooks/files.mjs, one]\n    - [node, hooks/files.mjs, two]\n  afterWrite: [node, hooks/after.mjs, \"{output}\"]\n",
    );
    write(
        tmp.path(),
        "hooks/files.mjs",
        &json_script(
            "const f = value.find((f) => f.path === 'schemas/Pet.ts'); f.contents += '// ' + process.argv[2] + '\\n'",
        ),
    );
    write(
        tmp.path(),
        "hooks/after.mjs",
        "import { writeFileSync } from 'node:fs';\nwriteFileSync(process.argv[2] + '/../after.txt', process.argv[2]);\n",
    );
    let out = tsurara(tmp.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let pet = std::fs::read_to_string(tmp.path().join("src/generated/schemas/Pet.ts")).unwrap();
    assert!(pet.ends_with("// one\n// two\n"), "{pet}");
    let after = std::fs::read_to_string(tmp.path().join("src/after.txt")).unwrap();
    let output = tmp.path().join("src/generated").canonicalize().unwrap();
    assert_eq!(Path::new(&after).canonicalize().unwrap(), output);
}

#[test]
fn failing_hooks_stop_with_the_exit_code_and_stderr() {
    if !has_node() {
        return;
    }
    let tmp = setup("  transformDocument: [node, hooks/fail.mjs]\n");
    write(
        tmp.path(),
        "hooks/fail.mjs",
        "process.stderr.write('cannot transform');\nprocess.exit(3);\n",
    );
    let out = tsurara(tmp.path());
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("hooks/fail.mjs"), "{stderr}");
    assert!(stderr.contains("exit code 3"), "{stderr}");
    assert!(stderr.contains("cannot transform"), "{stderr}");
    assert!(!tmp.path().join("src/generated").exists());
}

#[test]
fn hooks_that_print_invalid_json_are_errors() {
    if !has_node() {
        return;
    }
    let tmp = setup("  transformFiles: [node, -e, \"process.stdout.write('nope')\"]\n");
    let out = tsurara(tmp.path());
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("not valid JSON"), "{stderr}");
}

#[test]
fn transform_model_rewrites_the_model() {
    if !has_node() {
        return;
    }
    let tmp = setup("  transformModel: [node, hooks/rename.mjs]\n");
    write(
        tmp.path(),
        "hooks/rename.mjs",
        &json_script(
            "for (const r of value.names) if (r.kind.schema?.preferred === 'Pet') r.kind.schema.preferred = 'Animal'",
        ),
    );
    let out = tsurara(tmp.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(tmp.path().join("src/generated/schemas/Animal.ts").exists());
    assert!(!tmp.path().join("src/generated/schemas/Pet.ts").exists());
}

#[test]
fn transform_model_must_keep_the_model_version() {
    if !has_node() {
        return;
    }
    let tmp = setup("  transformModel: [node, hooks/bump.mjs]\n");
    write(
        tmp.path(),
        "hooks/bump.mjs",
        &json_script("value.semVersion = 99"),
    );
    let out = tsurara(tmp.path());
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("semVersion"), "{stderr}");
}

const TWO_OPERATIONS: &str = r##"
openapi: 3.1.0
info: { title: pets, version: "1" }
paths:
  /pets/{id}:
    get:
      operationId: getPet
      parameters: [{ name: id, in: path, required: true, schema: { type: string } }]
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
  /pets:
    post:
      operationId: addPet
      requestBody: { content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
      responses: { "204": { description: ok } }
components:
  schemas:
    Pet: { type: object, properties: { id: { type: string } } }
"##;

/// An `extend` hook that adds one file per operation, importing the call function and its Data type
/// as the manifest names them.
const EXTEND_HOOK: &str = r#"
let s = '';
process.stdin.on('data', (c) => (s += c));
process.stdin.on('end', () => {
  const { manifest, marker, language } = JSON.parse(s);
  if (language !== 'typescript') throw new Error(language);
  const files = manifest.operations.map((op) => {
    const fn = op.function;
    const data = op.types.data;
    const js = (file) => '../' + file.replace(/\.ts$/, '.js');
    const name = 'use' + fn.identifier[0].toUpperCase() + fn.identifier.slice(1);
    return {
      path: `hooks/${name}.ts`,
      contents: `${marker}\n\nimport type { Client } from '@tsurara/client';\nimport { ${fn.identifier} } from '${js(fn.file)}';\nimport type { ${data.identifier} } from '${js(data.file)}';\n\nexport const ${name} = (client: Client<unknown>, data: ${data.identifier}): ReturnType<typeof ${fn.identifier}> =>\n  ${fn.identifier}(client, data);\n`,
    };
  });
  process.stdout.write(JSON.stringify(files));
});
"#;

#[test]
fn extend_adds_files_from_the_manifest_and_they_are_deleted_with_their_operation() {
    if !has_node() {
        return;
    }
    let tmp = setup("  extend: [node, hooks/extend.mjs]\n");
    write(tmp.path(), "api/openapi.yaml", TWO_OPERATIONS);
    write(tmp.path(), "hooks/extend.mjs", EXTEND_HOOK);
    let out = tsurara(tmp.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let generated = tmp.path().join("src/generated");
    let hook = std::fs::read_to_string(generated.join("hooks/useGetPet.ts")).unwrap();
    assert!(
        hook.contains("import { getPet } from '../operations/getPet.js';"),
        "{hook}"
    );
    assert!(generated.join("hooks/useAddPet.ts").exists());

    let mut files = Vec::new();
    collect(&generated, &generated, &mut files);
    tsurara_testkit::assert_tsc_passes(&files, &tsurara_testkit::TsOptions::default());

    // Without `addPet`, its hook file is stale and deleted like a generated file.
    write(
        tmp.path(),
        "api/openapi.yaml",
        &TWO_OPERATIONS.replace("  /pets:\n    post:", "  /unused:\n    x-post:"),
    );
    let out = tsurara(tmp.path());
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(generated.join("hooks/useGetPet.ts").exists());
    assert!(!generated.join("hooks/useAddPet.ts").exists());
}

#[test]
fn extend_files_without_the_marker_fail() {
    if !has_node() {
        return;
    }
    let tmp = setup("  extend: [node, hooks/extend.mjs]\n");
    write(
        tmp.path(),
        "hooks/extend.mjs",
        "process.stdin.resume(); process.stdin.on('end', () => process.stdout.write(JSON.stringify([{ path: 'x.ts', contents: 'export {};' }])));\n",
    );
    let out = tsurara(tmp.path());
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("`x.ts` does not start with the generated-file marker"),
        "{stderr}"
    );
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
