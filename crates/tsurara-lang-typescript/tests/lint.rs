//! Generated code passes Biome's recommended lint rules, since users often lint it along with their
//! own code (formatting is theirs to choose, so it is not checked).

use std::path::Path;
use std::process::Command;
use tsurara_lang_typescript::TypeScriptOptions;
use tsurara_lang_typescript::generate;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2};
use tsurara_sem::Options;
use tsurara_testkit::{repo_root, required};
use url::Url;

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(&path).unwrap());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    normalize_swagger2(&mut set).unwrap();
    set
}

#[test]
fn generated_code_passes_biome_lint() {
    let biome = repo_root().join("node_modules/.bin/biome");
    if !biome.exists() {
        assert!(
            !required("typescript"),
            "biome is required; run `pnpm install`"
        );
        eprintln!("skipping: node_modules/.bin/biome was not found");
        return;
    }
    for name in [
        "kitchen-sink.yaml",
        "petstore3.json",
        "v3.1-webhook-example.yaml",
    ] {
        let out = generate(
            &load(name),
            &Options::default(),
            &TypeScriptOptions::default(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        for f in &out.files {
            let path = dir.path().join("src").join(&f.path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &f.contents).unwrap();
        }
        std::fs::write(
            dir.path().join("biome.json"),
            // The recommended rules are the default.
            r#"{ "formatter": { "enabled": false } }"#,
        )
        .unwrap();
        let result = Command::new(&biome)
            .args(["lint", "--error-on-warnings", "--max-diagnostics=50", "src"])
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{name}:\n{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }
}

#[test]
fn empty_types_and_lists_are_written_compactly() {
    let doc = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /todos:
    get:
      operationId: todosList
      responses: { "204": { description: ok } }
"##;
    let set = DocumentSet::from_value(tsurara_loader::parse_document(doc.as_bytes()).unwrap());
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    let all: String = out.files.iter().map(|f| f.contents.as_str()).collect();
    assert!(all.contains("export const servers = [] as const;"), "{all}");
    assert!(
        all.contains("export type TodosListData = Record<never, never>;"),
        "{all}"
    );
    assert!(!all.contains("= {};"), "{all}");
}
