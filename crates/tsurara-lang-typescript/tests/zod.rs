//! Zod schemas for response validation (`typescript.validators: zod`).

use std::path::Path;
use std::process::Command;
use tsurara_lang_typescript::options::{ImportExtension, Validators};
use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root, tsc_path};

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    DocumentSet::load(&url, &f as &dyn Fetcher).unwrap()
}

fn files(name: &str, opts: &TypeScriptOptions) -> Vec<(String, String)> {
    generate(&load(name), &Options::default(), opts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn zod() -> TypeScriptOptions {
    TypeScriptOptions {
        validators: Some(Validators::Zod),
        ..TypeScriptOptions::default()
    }
}

#[test]
fn without_the_option_nothing_mentions_zod() {
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        for (path, contents) in files(name, &TypeScriptOptions::default()) {
            assert!(
                !contents.contains("zod") && !path.starts_with("zod/"),
                "{name} {path}"
            );
        }
    }
}

#[test]
fn zod_schemas_pass_tsc() {
    for name in ["petstore3.json", "kitchen-sink.yaml"] {
        let files = files(name, &zod());
        assert!(
            files.iter().any(|(p, _)| p == "zod/operations.ts"),
            "{name}"
        );
        // The types and functions never import the Zod files.
        for (path, contents) in &files {
            if !path.starts_with("zod/") {
                assert!(!contents.contains("zod"), "{name} {path}");
            }
        }
        for (_, options) in TsOptions::matrix() {
            assert_tsc_passes(&files, &options);
        }
    }
}

#[test]
fn petstore_zod_snapshot() {
    let files = files("petstore3.json", &zod());
    let pick = |p: &str| files.iter().find(|(path, _)| path == p).unwrap().1.clone();
    insta::assert_snapshot!(format!(
        "{}\n{}\n{}",
        pick("zod/Pet.ts"),
        pick("zod/Order.ts"),
        pick("zod/operations.ts")
    ));
}

const RUN: &str = r#"import { createClient, validateResponses } from '@tsurara/client';
import { getPetById } from './gen/index.ts';
import { responseSchemas } from './gen/zod/operations.ts';

const body = process.argv[2];
const client = createClient({
  baseUrl: 'https://petstore.example/api/v3',
  validate: validateResponses(responseSchemas),
  fetch: async () => new Response(body, { status: 200, headers: { 'content-type': 'application/json' } }),
});
const r = await getPetById(client, { path: { petId: 1 } });
console.log(r.kind === 'validation' ? `validation ${r.issues.length}` : r.kind);
"#;

#[test]
fn generated_schemas_validate_responses_at_run_time() {
    let Some(tsc) = tsc_path() else { return };
    let dir = tempfile::tempdir().unwrap();
    // @tsurara/client built into node_modules (Node does not strip types there), zod linked.
    let client = dir.path().join("node_modules/@tsurara/client");
    let built = Command::new(tsc)
        .arg("-p")
        .arg(repo_root().join("runtimes/typescript/tsconfig.build.json"))
        .arg("--outDir")
        .arg(client.join("dist"))
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stdout)
    );
    std::fs::copy(
        repo_root().join("runtimes/typescript/package.json"),
        client.join("package.json"),
    )
    .unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        std::fs::canonicalize(repo_root().join("node_modules/zod")).unwrap(),
        dir.path().join("node_modules/zod"),
    )
    .unwrap();
    let opts = TypeScriptOptions {
        import_extension: ImportExtension::Ts,
        ..zod()
    };
    for (path, contents) in files("petstore3.json", &opts) {
        let full = dir.path().join("gen").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), RUN).unwrap();
    let run = |body: &str| {
        let out = Command::new("node")
            .arg("run.ts")
            .arg(body)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    assert_eq!(
        run(r#"{"id":1,"name":"Rex","photoUrls":[],"status":"sold"}"#),
        "success"
    );
    assert_eq!(
        run(r#"{"id":1,"photoUrls":"x","status":"lost"}"#),
        "validation 3"
    );
}
