//! Generated code runs directly with Node type stripping when it uses `.ts` import extensions, and does
//! not with the default `.js` extensions (design.md 7.3, chapter 17).
//!
//! Needs Node 22.18 or later; skipped otherwise unless `TSURARA_REQUIRE_NODE=1`.

use std::path::Path;
use std::process::Command;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_lang_typescript::options::ImportExtension;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::{TsOptions, assert_tsc_passes, repo_root, tsc_path};

const RUN: &str = r#"import { createClient } from '@tsurara/client';
import { getPetById } from './gen/index.ts';

const client = createClient({
  baseUrl: 'https://petstore.test',
  fetch: async () =>
    new Response(JSON.stringify({ id: 1, name: 'doggie', photoUrls: [] }), {
      headers: { 'content-type': 'application/json' },
    }),
});
const r = await getPetById(client, { path: { petId: 1 } });
if (!r.ok || r.data.name !== 'doggie') {
  console.error(r);
  process.exit(1);
}
console.log('ok');
"#;

/// Node 22.18 or later, or `None` to skip.
fn node() -> Option<()> {
    let required = std::env::var_os("TSURARA_REQUIRE_NODE").is_some();
    let version = Command::new("node")
        .arg("--version")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned());
    let parsed = version.as_deref().and_then(|v| {
        let mut parts = v.trim_start_matches('v').split('.');
        Some((
            parts.next()?.parse::<u32>().ok()?,
            parts.next()?.parse::<u32>().ok()?,
        ))
    });
    match parsed {
        Some(v) if v >= (22, 18) => Some(()),
        _ if required => panic!("Node 22.18 or later is required, found {version:?}"),
        _ => {
            eprintln!("skipping: Node 22.18 or later is required, found {version:?}");
            None
        }
    }
}

fn petstore() -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/petstore3.json");
    let url = url::Url::parse("https://example.com/petstore3.json").unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    DocumentSet::load(&url, &f as &dyn Fetcher).unwrap()
}

fn files(ext: ImportExtension) -> Vec<(String, String)> {
    let opts = TypeScriptOptions {
        import_extension: ext,
        ..TypeScriptOptions::default()
    };
    generate(&petstore(), &Options::default(), &opts)
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

/// A directory with `@tsurara/client` built into `node_modules`, the generated files in `gen/`, and
/// `run.ts`. Node does not strip types under `node_modules`, so the client is compiled.
fn project(ext: ImportExtension) -> tempfile::TempDir {
    let tsc = tsc_path().expect("tsc is installed");
    let dir = tempfile::tempdir().unwrap();
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
    for (path, contents) in files(ext) {
        let full = dir.path().join("gen").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    std::fs::write(dir.path().join("package.json"), r#"{ "type": "module" }"#).unwrap();
    std::fs::write(dir.path().join("run.ts"), RUN).unwrap();
    dir
}

fn run(dir: &Path) -> std::process::Output {
    Command::new("node")
        .arg("run.ts")
        .current_dir(dir)
        .output()
        .unwrap()
}

#[test]
fn ts_extensions_run_directly_in_node() {
    if node().is_none() || tsc_path().is_none() {
        return;
    }
    let dir = project(ImportExtension::Ts);
    let out = run(dir.path());
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "ok");
}

#[test]
fn js_extensions_do_not_run_directly_in_node() {
    if node().is_none() || tsc_path().is_none() {
        return;
    }
    let dir = project(ImportExtension::Js);
    let out = run(dir.path());
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("ERR_MODULE_NOT_FOUND"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn ts_extensions_type_check_with_consumer_settings() {
    let files = files(ImportExtension::Ts);
    // `allowImportingTsExtensions` with `noEmit` (the default options).
    assert_tsc_passes(&files, &TsOptions::default());
    // `rewriteRelativeImportExtensions`, for consumers that compile the output to JavaScript.
    assert_tsc_passes(
        &files,
        &TsOptions {
            rewrite_relative_import_extensions: true,
            ..TsOptions::default()
        },
    );
}
