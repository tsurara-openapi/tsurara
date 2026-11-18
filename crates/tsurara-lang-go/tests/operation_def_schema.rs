//! Every OperationDef the Go backend embeds validates against `spec/operation-def.schema.json`
//! (design.md 18.9).

use tsurara_lang_go::{GoOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;
use tsurara_testkit::repo_root;

fn validator() -> jsonschema::Validator {
    let read = |name: &str| -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(repo_root().join("spec").join(name)).unwrap())
            .unwrap()
    };
    let registry = jsonschema::Registry::new()
        .add(
            "https://raw.githubusercontent.com/tsurara-openapi/tsurara/main/spec/type-def.schema.json",
            read("type-def.schema.json"),
        )
        .unwrap()
        .prepare()
        .unwrap();
    jsonschema::options()
        .with_registry(&registry)
        .build(&read("operation-def.schema.json"))
        .unwrap()
}

/// The JSON in `tsurara.ParseOperationDef(...)`: a raw string, or an interpreted one.
fn embedded(code: &str) -> serde_json::Value {
    let start =
        code.find("ParseOperationDef(").expect("an OperationDef") + "ParseOperationDef(".len();
    let rest = &code[start..];
    let text = if let Some(raw) = rest.strip_prefix('`') {
        raw[..raw.find('`').unwrap()].to_owned()
    } else {
        let end = rest.find(")\n").unwrap();
        serde_json::from_str::<String>(&rest[..end]).unwrap()
    };
    serde_json::from_str(&text).unwrap()
}

#[test]
fn embedded_operation_defs_validate_against_the_schema() {
    let validator = validator();
    let mut paths = vec![
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/kitchen-sink.yaml"),
        repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/petstore3.json"),
    ];
    let mut edge: Vec<_> = std::fs::read_dir(repo_root().join("corpus/edge-cases"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    edge.sort();
    paths.extend(edge);
    let mut checked = 0;
    for path in paths {
        let name = path.file_name().unwrap().to_string_lossy();
        let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
        let mut f = MemoryFetcher::new();
        f.insert(url.as_str(), std::fs::read(&path).unwrap());
        let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
        tsurara_loader::normalize_swagger2(&mut set).unwrap();
        let out = generate(&set, &Options::default(), &GoOptions::default()).unwrap();
        for file in out.files.iter().filter(|f| f.path.ends_with("_op_gen.go")) {
            let def = embedded(&file.contents);
            let errors: Vec<String> = validator.iter_errors(&def).map(|e| e.to_string()).collect();
            assert!(errors.is_empty(), "{name} {}: {errors:?}\n{def}", file.path);
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked}");
}
