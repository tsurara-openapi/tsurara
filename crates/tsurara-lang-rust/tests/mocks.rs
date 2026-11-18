//! Mock factories and routes (`rust.mocks`): the petstore call functions answered by the
//! generated routes through `MockTransport`, with the same samples as the TypeScript mocks.

use tsurara_lang_rust::{Rust, RustOptions};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, RustChecker, repo_root};

fn files(mocks: bool) -> Vec<(String, String)> {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/v3.0-petstore.yaml");
    let set = DocumentSet::from_value(parse_document(&std::fs::read(path).unwrap()).unwrap());
    let backend = Rust {
        options: RustOptions {
            mocks,
            ..RustOptions::default()
        },
    };
    tsurara_backend::generate(&backend, &set, &Options::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

#[test]
fn without_the_option_there_are_no_mocks() {
    let files = files(false);
    assert!(files.iter().all(|(p, _)| !p.starts_with("mocks/")));
    assert!(!files.iter().any(|(_, c)| c.contains("mocks")));
}

const TEST: &str = r#"
use tsurara_client::testing::MockTransport;
use tsurara_client::{ErrorBody, Failure};
use tsurara_generated::api::mocks::operations::{ShowPetByIdMockResponse, routes, show_pet_by_id as mock_show};
use tsurara_generated::api::mocks::schemas::mock_pet;
use tsurara_generated::api::operations::{list_pets, show_pet_by_id};
use tsurara_generated::api::types::{Error, Pet};

#[tokio::test]
async fn routes_answer_with_the_samples() {
    let client = tsurara_client::Client::builder("https://petstore.test/v1")
        .transport(routes(MockTransport::new().base_path("/v1")))
        .build();
    let pet = show_pet_by_id(&client, "7".into(), Default::default()).await.unwrap();
    assert_eq!((pet.data.id, pet.data.name.as_str(), pet.data.tag.as_deref()), (1, "name", Some("tag")));
    let pets = list_pets(&client, Default::default()).await.unwrap();
    assert_eq!(pets.data.len(), 1);
    let renamed = Pet { name: "Tama".into(), ..mock_pet() };
    assert_eq!(renamed.id, 1);
}

#[tokio::test]
async fn a_route_answers_with_a_documented_error() {
    let mock = mock_show(MockTransport::new().base_path("/v1"), |_| {
        ShowPetByIdMockResponse::Default(404, Error { code: 404, message: "gone".into() })
    });
    let client = tsurara_client::Client::builder("https://petstore.test/v1").transport(mock).build();
    match show_pet_by_id(&client, "7".into(), Default::default()).await {
        Err(Failure::Http { status: 404, body: ErrorBody::Documented(e), .. }) => assert_eq!(e.message, "gone"),
        other => panic!("{other:?}"),
    }
}
"#;

#[test]
fn routes_answer_the_call_functions() {
    let checker = RustChecker {
        tests: vec![("mocks".into(), TEST.into())],
        http: false,
        mocks: true,
        ..Default::default()
    };
    if let Err(e) = checker.check(&files(true)) {
        panic!("{e}");
    }
}
