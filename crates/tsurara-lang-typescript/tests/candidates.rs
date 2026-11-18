//! Hoisting candidates: inline objects, enums and unions carry a preferred name and a location, so
//! that a backend can turn every one of them into a named type (design.md 18.4.2).

use std::collections::BTreeMap;
use std::path::Path;
use tsurara_backend::hoist::claim_candidates;
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, normalize_swagger2};
use tsurara_naming::{Context, Names, NamingPolicy, pascal_case};
use tsurara_sem::{Options, build_model};

/// A nominal language's policy: PascalCase types, no reserved words.
struct Nominal;

impl NamingPolicy for Nominal {
    fn convert(&self, _: Context, raw: &str, _: bool) -> String {
        pascal_case(raw)
    }
    fn is_reserved(&self, _: Context, _: &str) -> bool {
        false
    }
    fn reserved_suffix(&self, _: Context) -> &str {
        ""
    }
}

fn load_text(name: &str, text: &str) -> DocumentSet {
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), text.as_bytes().to_vec());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    normalize_swagger2(&mut set).unwrap();
    set
}

fn fixture(name: &str) -> String {
    std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name),
    )
    .unwrap()
}

/// Names every declaration, then every candidate, with the nominal policy.
fn hoist(set: &DocumentSet, overrides: &BTreeMap<String, String>) -> Vec<(String, String)> {
    let model = build_model(set, &Options::default()).unwrap();
    let mut names = Names::new();
    for d in &model.schemas {
        let ident = Nominal.convert(Context::Type, &d.preferred_name, true);
        names.claim(&Nominal, Context::Type, "types", &ident, "");
    }
    claim_candidates(&model, &mut names, &Nominal, "types", overrides)
        .into_iter()
        .map(|(loc, name)| (format!("#{}", loc.pointer), name))
        .collect()
}

#[test]
fn every_candidate_gets_a_deterministic_name() {
    let set = load_text("kitchen-sink.yaml", &fixture("kitchen-sink.yaml"));
    let a = hoist(&set, &BTreeMap::new());
    assert!(a.len() > 5, "{a:?}");
    assert_eq!(a, hoist(&set, &BTreeMap::new()));
    // Parameters are named after their `name`.
    assert!(a.iter().any(|(_, n)| n == "GetItemFilter"), "{a:?}");
    // Names come from the owner and the path, not from numbers.
    assert!(
        a.iter().any(
            |(p, n)| p.ends_with("/requestBody/content/multipart~1form-data/schema")
                && n.ends_with("Body")
        ),
        "{a:?}"
    );
}

#[test]
fn adding_an_inline_property_does_not_change_existing_names() {
    let text = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths: {}
components:
  schemas:
    Pet:
      type: object
      properties:
        owner: { type: object, properties: { name: { type: string } } }
        status: { type: string, enum: [a, b] }
        EXTRA
"##;
    let before = hoist(
        &load_text("a.yaml", &text.replace("EXTRA", "")),
        &BTreeMap::new(),
    );
    assert_eq!(
        before,
        [
            (
                "#/components/schemas/Pet/properties/owner".to_owned(),
                "PetOwner".to_owned()
            ),
            (
                "#/components/schemas/Pet/properties/status".to_owned(),
                "PetStatus".to_owned()
            ),
        ]
    );
    let after = hoist(
        &load_text(
            "a.yaml",
            &text.replace("EXTRA", "tags: { type: array, items: { type: object } }"),
        ),
        &BTreeMap::new(),
    );
    for pair in &before {
        assert!(after.contains(pair), "{after:?}");
    }
    assert!(after.contains(&(
        "#/components/schemas/Pet/properties/tags/items".to_owned(),
        "PetTagsItem".to_owned()
    )));
    // Candidates can be pinned with naming.overrides by their pointer.
    let pinned = hoist(
        &load_text("a.yaml", &text.replace("EXTRA", "")),
        &BTreeMap::from([(
            "#/components/schemas/Pet/properties/owner".to_owned(),
            "Human".to_owned(),
        )]),
    );
    assert_eq!(pinned[0].1, "Human");
}
