//! The model round-trips through JSON (design.md 18.4.6).

use std::path::Path;
use tsurara_backend::generate_from_model;
use tsurara_lang_typescript::TypeScript;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2};
use tsurara_sem::{Model, Options, SEM_VERSION, build_model};

fn load(name: &str) -> DocumentSet {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    normalize_swagger2(&mut set).unwrap();
    set
}

#[test]
fn every_fixture_model_round_trips_through_json() {
    for name in [
        "kitchen-sink.yaml",
        "petstore3.json",
        "v3.0-petstore-expanded.yaml",
        "v3.0-petstore.yaml",
        "v3.1-non-oauth-scopes.yaml",
        "v3.1-webhook-example.yaml",
    ] {
        let model = build_model(&load(name), &Options::default()).unwrap();
        let json = serde_json::to_value(&model).unwrap();
        assert_eq!(json["semVersion"], SEM_VERSION, "{name}");
        let back: Model = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(format!("{back:?}"), format!("{model:?}"), "{name}");
        assert_eq!(serde_json::to_value(&back).unwrap(), json, "{name}");
    }
}

#[test]
fn an_edited_model_generates_the_edited_names() {
    let opts = Options::default();
    let model = build_model(&load("petstore3.json"), &opts).unwrap();
    let mut json = serde_json::to_value(&model).unwrap();
    let request = json["names"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|r| r["kind"]["schema"]["preferred"] == "Pet")
        .expect("a name request for Pet");
    request["kind"]["schema"]["preferred"] = "Animal".into();
    let edited: Model = serde_json::from_value(json).unwrap();
    let out = generate_from_model(&TypeScript::default(), &edited, &opts).unwrap();
    assert!(out.files.iter().any(|f| f.path == "schemas/Animal.ts"));
    assert!(!out.files.iter().any(|f| f.path == "schemas/Pet.ts"));
}
