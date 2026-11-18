//! The `Input` methods generated functions use to pass typed values.

use tsurara_client::serde::{Serialize, Serializer};
use tsurara_client::serde_json::{Value, json};
use tsurara_client::{Body, File, Input, Part};

#[derive(Serialize)]
#[serde(crate = "tsurara_client::serde")]
struct Upload {
    name: String,
    file: File,
    #[serde(skip_serializing_if = "Option::is_none")]
    thumbnail: Option<File>,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    extras: Vec<File>,
    tags: Vec<String>,
}

fn file(text: &str) -> File {
    File {
        content: text.as_bytes().to_vec(),
        filename: Some(format!("{text}.txt")),
        content_type: Some("text/plain".into()),
    }
}

#[test]
fn parameters_are_encoded_as_json_values() {
    let mut input = Input::default();
    input.path("id", &5);
    input.query("tags", &vec!["a", "b"]);
    input.header("x-trace", &"t");
    input.cookie("session", &true);
    input.querystring(&json!({ "a": 1 }));
    input.value_body(&json!({ "x": [1] }));
    input.content_type("application/json");
    assert_eq!(input.path, [("id".to_owned(), json!(5))]);
    assert_eq!(input.query, [("tags".to_owned(), json!(["a", "b"]))]);
    assert_eq!(input.headers, [("x-trace".to_owned(), json!("t"))]);
    assert_eq!(input.cookies, [("session".to_owned(), json!(true))]);
    assert_eq!(input.querystring, Some(json!({ "a": 1 })));
    assert_eq!(input.body, Some(Body::Value(json!({ "x": [1] }))));
    assert_eq!(input.content_type.as_deref(), Some("application/json"));
    assert_eq!(input.encode_error, None);
}

#[test]
fn multipart_bodies_keep_files_as_parts() {
    let mut input = Input::default();
    input.multipart_body(&Upload {
        name: "n".into(),
        file: file("a"),
        thumbnail: None,
        note: Some("hi".into()),
        extras: vec![file("b"), file("c")],
        tags: vec!["x".into()],
    });
    let Some(Body::Multipart(parts)) = input.body else {
        panic!("{:?}", input.body)
    };
    assert_eq!(
        parts,
        [
            ("name".to_owned(), Part::Value(json!("n"))),
            ("file".to_owned(), Part::File(file("a"))),
            ("note".to_owned(), Part::Value(json!("hi"))),
            ("extras".to_owned(), Part::File(file("b"))),
            ("extras".to_owned(), Part::File(file("c"))),
            ("tags".to_owned(), Part::Value(json!(["x"]))),
        ]
    );
}

#[test]
fn files_outside_multipart_are_their_bytes() {
    assert_eq!(
        tsurara_client::serde_json::to_value(file("ab")).unwrap(),
        json!([97, 98])
    );
    let back: File = tsurara_client::serde_json::from_value(json!([97, 98])).unwrap();
    assert_eq!(back.content, b"ab");
}

struct Broken;

impl Serialize for Broken {
    fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(tsurara_client::serde::ser::Error::custom("broken"))
    }
}

#[test]
fn values_that_fail_to_encode_are_kept_as_an_error() {
    let mut input = Input::default();
    input.query("a", &Broken);
    input.query("b", &1);
    assert!(input.encode_error.as_deref().unwrap().contains("broken"));
    assert_eq!(input.query, [("b".to_owned(), Value::from(1))]);
}
