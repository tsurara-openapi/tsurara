//! The mock transport of the `testing` feature: routing vectors and calls through it.
#![cfg(feature = "testing")]

use tsurara_client::serde::Deserialize;
use tsurara_client::testing::{MockResponse, MockTransport};
use tsurara_client::{Client, ErrorBody, Failure, Input, OperationDef};

#[derive(Debug, Deserialize, PartialEq)]
#[serde(crate = "tsurara_client::serde")]
struct Pet {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(crate = "tsurara_client::serde")]
struct Problem {
    message: String,
}

#[test]
fn routing_vectors() {
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spec/conformance/mock-routing.json"
    ))
    .unwrap();
    let file: serde_json::Value = serde_json::from_str(&text).unwrap();
    for v in file["vectors"].as_array().unwrap() {
        let name = v["name"].as_str().unwrap();
        let mut mock = MockTransport::new().base_path(v["basePath"].as_str().unwrap());
        for r in v["routes"].as_array().unwrap() {
            mock = mock.route(
                r["method"].as_str().unwrap(),
                r["path"].as_str().unwrap(),
                |_| Ok(MockResponse::new(204)),
            );
        }
        let found = mock.matches(
            v["request"]["method"].as_str().unwrap(),
            v["request"]["path"].as_str().unwrap(),
        );
        match v["expect"]["route"].as_u64() {
            None => assert!(found.is_none(), "{name}: {found:?}"),
            Some(i) => {
                let (index, params) = found.unwrap_or_else(|| panic!("{name}: no route"));
                assert_eq!(index as u64, i, "{name}");
                let want: std::collections::BTreeMap<String, String> =
                    serde_json::from_value(v["expect"]["params"].clone()).unwrap();
                assert_eq!(params, want, "{name}");
            }
        }
    }
}

fn get_pet() -> OperationDef {
    OperationDef::from_json(
        r#"{ "v": 1, "id": "getPet", "method": "GET", "path": "/pets/{petId}",
             "params": { "path": { "petId": {} } },
             "responses": { "200": { "parseAs": "json" }, "404": { "parseAs": "json" } } }"#,
    )
}

fn pet(id: i64) -> Input {
    Input {
        path: vec![("petId".into(), id.into())],
        ..Input::default()
    }
}

#[tokio::test]
async fn calls_through_the_mock_get_typed_results() {
    let mock = MockTransport::new()
        .base_path("/v1")
        .operation(&get_pet(), |req| {
            Ok(if req.params["petId"] == "1" {
                MockResponse::json(200, &serde_json::json!({ "id": 1, "name": "Pochi" }))
            } else {
                MockResponse::json(404, &serde_json::json!({ "message": "no pet" }))
            })
        });
    let client = Client::builder("https://api.test/v1")
        .transport(mock.clone())
        .build();
    let ok = client
        .request::<Pet, Problem>(&get_pet(), pet(1))
        .await
        .unwrap();
    assert_eq!(
        ok.data,
        Pet {
            id: 1,
            name: "Pochi".into()
        }
    );
    match client.request::<Pet, Problem>(&get_pet(), pet(9)).await {
        Err(Failure::Http {
            status: 404,
            body: ErrorBody::Documented(p),
            ..
        }) => assert_eq!(p.message, "no pet"),
        other => panic!("{other:?}"),
    }
    let calls = mock.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].route.as_deref(), Some("getPet"));
    assert_eq!(calls[1].params["petId"], "9");
}

#[tokio::test]
async fn an_unmatched_request_is_a_transport_failure() {
    let client = Client::builder("https://api.test")
        .transport(MockTransport::new())
        .build();
    match client.request::<Pet, Problem>(&get_pet(), pet(1)).await {
        Err(Failure::Network { cause, .. }) => {
            assert!(cause.to_string().contains("GET /pets/1"), "{cause}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn unmatched_requests_can_be_recorded_and_handlers_can_fail() {
    let mock = MockTransport::new().record_unmatched();
    let client = Client::builder("https://api.test")
        .transport(mock.clone())
        .build();
    match client.request::<Pet, Problem>(&get_pet(), pet(1)).await {
        Err(Failure::Http { status: 501, .. }) => {}
        other => panic!("{other:?}"),
    }
    assert_eq!(mock.unmatched().len(), 1);

    let failing =
        MockTransport::new().route("GET", "/pets/{petId}", |_| Err("connection reset".into()));
    let client = Client::builder("https://api.test")
        .transport(failing)
        .build();
    assert!(matches!(
        client.request::<Pet, Problem>(&get_pet(), pet(1)).await,
        Err(Failure::Network { .. })
    ));
}
