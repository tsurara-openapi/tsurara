//! Security requirements in OperationDefs and the exported scheme names.

use tsurara_lang_typescript::{TypeScriptOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher};
use tsurara_sem::Options;

#[test]
fn petstore_operations_carry_their_security() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/petstore3.json");
    let url = url::Url::parse("https://example.com/petstore3.json").unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    let set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    let out = generate(&set, &Options::default(), &TypeScriptOptions::default()).unwrap();
    let file = |p: &str| &out.files.iter().find(|f| f.path == p).unwrap().contents;
    assert!(
        file("client.ts").contains("export type SecuritySchemeName = 'petstore_auth' | 'api_key';"),
        "{}",
        file("client.ts")
    );
    let get = file("operations/getPetById.ts");
    assert!(
        get.contains("security: [[{ scheme: 'api_key', type: 'apiKey', in: 'header', name: 'api_key' }], [{ scheme: 'petstore_auth', type: 'oauth2', scopes: ['write:pets', 'read:pets'] }]],"),
        "{get}"
    );
}
