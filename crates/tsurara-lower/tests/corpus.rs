//! The fixtures lower without unions, inheritance or inline records, and the validator passes.

use std::path::Path;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2};
use tsurara_lower::{Capabilities, lower, validate};
use tsurara_sem::{Options, build_model};

#[test]
fn fixtures_lower_with_minimal_capabilities() {
    let dir =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tsurara-lang-typescript/tests/fixtures");
    let mut count = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
        let mut f = MemoryFetcher::new();
        f.insert(url.as_str(), std::fs::read(&path).unwrap());
        let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
        normalize_swagger2(&mut set).unwrap();
        let model = build_model(&set, &Options::default()).unwrap();
        for caps in [Capabilities::MINIMAL, Capabilities::ALL] {
            let (module, _) = lower(&model, &caps);
            let errors = validate(&module, &caps);
            assert!(errors.is_empty(), "{name}: {errors:#?}");
            assert!(module.decls.len() >= model.schemas.len());
        }
        count += 1;
    }
    assert!(count >= 5);
}
