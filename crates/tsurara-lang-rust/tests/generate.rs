//! The Rust backend's output compiles (clippy without warnings) and decodes and encodes JSON as the
//! definition says, and its call functions call a server.

use tsurara_lang_rust::{RustOptions, generate};
use tsurara_loader::{DocumentSet, Fetcher, MemoryFetcher, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, RustChecker, repo_root};

fn fixture(path: &std::path::Path) -> DocumentSet {
    let name = path.file_name().unwrap().to_string_lossy();
    let url = url::Url::parse(&format!("https://example.com/{name}")).unwrap();
    let mut f = MemoryFetcher::new();
    f.insert(url.as_str(), std::fs::read(path).unwrap());
    let mut set = DocumentSet::load(&url, &f as &dyn Fetcher).unwrap();
    tsurara_loader::normalize_swagger2(&mut set).unwrap();
    set
}

fn files(set: &DocumentSet) -> Vec<(String, String)> {
    generate(set, &Options::default(), &RustOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

/// Type modules import the types they use by name: a glob import of the parent in each of them
/// made name resolution quadratic in the number of types (OpenAI's 6,200 types took
/// 95 seconds of `cargo check` instead of 14).
#[test]
fn type_modules_import_types_by_name() {
    let path = repo_root().join("crates/tsurara-lang-typescript/tests/fixtures/kitchen-sink.yaml");
    let files = files(&fixture(&path));
    let mut named = 0;
    for (path, contents) in &files {
        if path.starts_with("types/") && path != "types/mod.rs" {
            assert!(!contents.contains("use super::*"), "{path}:\n{contents}");
            named += usize::from(contents.contains("use super::"));
        }
    }
    assert!(named > 0, "no type module refers to another");
}

#[test]
fn fixtures_and_edge_cases_compile_without_warnings() {
    let mut paths: Vec<std::path::PathBuf> = ["kitchen-sink.yaml", "petstore3.json"]
        .iter()
        .map(|n| {
            repo_root()
                .join("crates/tsurara-lang-typescript/tests/fixtures")
                .join(n)
        })
        .collect();
    for e in std::fs::read_dir(repo_root().join("corpus/edge-cases")).unwrap() {
        paths.push(e.unwrap().path());
    }
    let mut failures = Vec::new();
    for path in paths {
        let files = files(&fixture(&path));
        assert!(
            files.iter().any(|(p, _)| p == "mod.rs"),
            "{}",
            path.display()
        );
        if let Err(e) = RustChecker::default().check(&files) {
            failures.push(format!("{}: {e}", path.display()));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

const DOC: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      responses:
        "200":
          description: ok
          content: { application/json: { schema: { type: array, items: { $ref: "#/components/schemas/Pet" } } } }
components:
  schemas:
    Pet:
      type: object
      required: [id, petType, owner]
      properties:
        id: { type: integer, format: int64 }
        petType: { type: string }
        nickname: { type: string }
        owner: { type: [string, "null"] }
        note: { type: [string, "null"] }
        status: { $ref: "#/components/schemas/Status" }
        type: { type: string }
      additionalProperties: { type: integer }
    Status: { type: string, enum: [available, sold] }
    Level: { type: integer, enum: [1, 2] }
    Animal:
      oneOf: [{ $ref: "#/components/schemas/Cat" }, { $ref: "#/components/schemas/Dog" }]
      discriminator: { propertyName: kind }
    Cat: { type: object, required: [kind], properties: { kind: { type: string }, indoor: { type: boolean } } }
    Dog: { type: object, required: [kind], properties: { kind: { type: string } } }
    Id: { oneOf: [{ type: integer }, { type: string }] }
    Shape:
      oneOf: [{ $ref: "#/components/schemas/Square" }]
      discriminator:
        propertyName: kind
        mapping: { box: "#/components/schemas/Square", square: "#/components/schemas/Square" }
        defaultMapping: "#/components/schemas/Circle"
    Square: { type: object, required: [kind, side], properties: { kind: { type: string }, side: { type: integer } } }
    Circle: { type: object, required: [kind, r], properties: { kind: { type: string }, r: { type: number } } }
    Tree:
      type: object
      properties:
        children: { type: array, items: { $ref: "#/components/schemas/Tree" } }
        parent: { $ref: "#/components/schemas/Node" }
    Node: { oneOf: [{ $ref: "#/components/schemas/Tree" }, { type: string }] }
    Nested: { type: array, items: { $ref: "#/components/schemas/Nested" } }
    Forest: { type: object, additionalProperties: { $ref: "#/components/schemas/Grove" } }
    Grove: { type: array, items: { $ref: "#/components/schemas/Forest" } }
"##;

const ROUNDTRIP: &str = r#"
use tsurara_client::serde_json::{self, json};
use tsurara_generated::api::types::*;

#[test]
fn records_rename_fields_and_keep_missing_null_and_extra_values_apart() {
    let input = json!({ "id": 1, "petType": "cat", "owner": null, "note": null, "status": "lost", "type": "t", "extra": 5 });
    let pet: Pet = serde_json::from_value(input.clone()).unwrap();
    assert_eq!(pet.pet_type, "cat");
    assert_eq!(pet.owner, None);
    assert_eq!(pet.note, tsurara_client::Nullable::Null);
    assert_eq!(pet.nickname, None);
    assert_eq!(pet.status, Some(Status::Unknown("lost".into())));
    assert_eq!(pet.r#type.as_deref(), Some("t"));
    assert_eq!(pet.additional_properties.get("extra"), Some(&5));
    assert_eq!(serde_json::to_value(&pet).unwrap(), input);
    let sparse: Pet = serde_json::from_value(json!({ "id": 2, "petType": "dog", "owner": "o" })).unwrap();
    assert_eq!(sparse.note, tsurara_client::Nullable::Missing);
    assert_eq!(serde_json::to_value(&sparse).unwrap(), json!({ "id": 2, "petType": "dog", "owner": "o" }));
    let known: Status = serde_json::from_value(json!("sold")).unwrap();
    assert_eq!(known, Status::Sold);
    assert_eq!(LEVEL_1, 1);
    // A required field that is missing, and an integer with a fraction, are errors.
    assert!(serde_json::from_value::<Pet>(json!({ "petType": "cat", "owner": null })).is_err());
    assert!(serde_json::from_value::<Pet>(json!({ "id": 1.5, "petType": "cat", "owner": null })).is_err());
}

#[test]
fn unions_select_their_members() {
    let cat: Animal = serde_json::from_value(json!({ "kind": "Cat", "indoor": true })).unwrap();
    assert!(matches!(cat, Animal::Cat(Cat { indoor: Some(true), .. })));
    let other: Animal = serde_json::from_value(json!({ "kind": "Bird" })).unwrap();
    assert!(matches!(other, Animal::Unknown(_)));
    let id: Id = serde_json::from_value(json!("x")).unwrap();
    assert_eq!(id, Id::String("x".into()));
    assert!(serde_json::from_value::<Id>(json!(true)).is_err());
    let square: Shape = serde_json::from_value(json!({ "kind": "box", "side": 2 })).unwrap();
    assert!(matches!(square, Shape::Square(Square { side: 2, .. })));
    let oval: Shape = serde_json::from_value(json!({ "kind": "oval", "r": 1 })).unwrap();
    assert!(matches!(&oval, Shape::Circle(c) if c.kind == "oval"));
    assert_eq!(serde_json::to_value(&oval).unwrap(), json!({ "kind": "oval", "r": 1.0 }));
    // Aliases that contain themselves are newtypes.
    let nested: Nested = serde_json::from_value(json!([[], [[]]])).unwrap();
    assert_eq!(nested.0.len(), 2);
    assert_eq!(serde_json::to_value(&nested).unwrap(), json!([[], [[]]]));
    let forest: Forest = serde_json::from_value(json!({ "a": [{}] })).unwrap();
    assert_eq!(serde_json::to_value(&forest).unwrap(), json!({ "a": [{}] }));
    let tree: Tree = serde_json::from_value(json!({ "parent": { "children": [] } })).unwrap();
    assert!(matches!(tree.parent.as_deref(), Some(Node::Tree(_))));
}
"#;

#[test]
fn generated_types_decode_and_encode_json() {
    let set = DocumentSet::from_value(parse_document(DOC.as_bytes()).unwrap());
    let checker = RustChecker {
        tests: vec![("roundtrip".into(), ROUNDTRIP.into())],
        http: false,
        mocks: false,
        ..Default::default()
    };
    checker.check(&files(&set)).unwrap();
}

const OPERATIONS: &str = r##"
openapi: 3.1.0
info: { title: t, version: "1" }
paths:
  /pets:
    get:
      operationId: listPets
      parameters:
        - { name: limit, in: query, schema: { type: integer, format: int32 } }
        - { name: tags, in: query, schema: { type: array, items: { type: string } } }
      responses:
        "200":
          description: ok
          content: { application/json: { schema: { type: array, items: { $ref: "#/components/schemas/Pet" } } } }
    post:
      operationId: createPet
      requestBody:
        required: true
        content: { application/json: { schema: { $ref: "#/components/schemas/NewPet" } } }
      responses:
        "201": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "422": { description: bad, content: { application/json: { schema: { $ref: "#/components/schemas/Problem" } } } }
  /pets/{petId}:
    get:
      operationId: getPet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: integer, format: int64 } }
        - { name: X-Trace, in: header, schema: { type: string } }
      responses:
        "200": { description: ok, content: { application/json: { schema: { $ref: "#/components/schemas/Pet" } } } }
        "404": { description: missing, content: { application/json: { schema: { $ref: "#/components/schemas/Problem" } } } }
    delete:
      operationId: deletePet
      parameters:
        - { name: petId, in: path, required: true, schema: { type: integer, format: int64 } }
      responses:
        "204": { description: gone }
  /upload:
    post:
      operationId: upload
      requestBody:
        content:
          multipart/form-data:
            schema:
              type: object
              required: [file]
              properties:
                file: { type: string, format: binary }
                note: { type: string }
      responses:
        "200": { description: ok, content: { text/plain: { schema: { type: string } } } }
  /raw:
    put:
      operationId: putRaw
      requestBody:
        required: true
        content:
          application/octet-stream: { schema: { type: string, format: binary } }
          application/json: { schema: { $ref: "#/components/schemas/Pet" } }
      responses:
        "200": { description: ok, content: { application/octet-stream: { schema: { type: string, format: binary } } } }
  /slow:
    get:
      operationId: slow
      responses:
        "200": { description: ok, content: { application/json: { schema: { type: object } } } }
components:
  schemas:
    Pet:
      type: object
      required: [id, name]
      properties: { id: { type: integer, format: int64 }, name: { type: string } }
    NewPet: { type: object, required: [name], properties: { name: { type: string } } }
    Problem: { type: object, required: [message], properties: { message: { type: string } } }
"##;

const CALLS: &str = r##"
use tsurara_client::{Client, ErrorBody, Failure, File};
use tsurara_generated::api::operations::*;
use tsurara_generated::api::types::*;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// Answers per path: the request target, the body and the content type come back in the response.
async fn serve() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            tokio::spawn(async move {
                let mut data = Vec::new();
                let mut buf = [0u8; 4096];
                let (head, body) = loop {
                    let n = socket.read(&mut buf).await.unwrap();
                    data.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&data).into_owned();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let head = text[..end].to_owned();
                        let length: usize = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length: ").map(|v| v.trim().parse().unwrap()))
                            .unwrap_or(0);
                        if data.len() >= end + 4 + length {
                            break (head, data[end + 4..end + 4 + length].to_vec());
                        }
                    }
                    if n == 0 {
                        return;
                    }
                };
                let mut request_line = head.lines().next().unwrap().split(' ');
                let method = request_line.next().unwrap().to_owned();
                let target = request_line.next().unwrap().to_owned();
                let header = |name: &str| {
                    head.lines()
                        .find_map(|l| l.to_ascii_lowercase().strip_prefix(&format!("{name}: ")).map(str::to_owned))
                        .unwrap_or_default()
                };
                let text = String::from_utf8_lossy(&body).into_owned();
                let (status, ctype, out): (u16, &str, Vec<u8>) = match (method.as_str(), target.as_str()) {
                    ("GET", t) if t.starts_with("/pets?") || t == "/pets" => (200, "application/json", format!("[{{\"id\":0,\"name\":{:?}}}]", t).into_bytes()),
                    ("POST", "/pets") if text.contains("\"bad\"") => (422, "application/json", b"{\"message\":\"bad name\"}".to_vec()),
                    ("POST", "/pets") => (201, "application/json", format!("{{\"id\":7,\"name\":{:?}}}", text).into_bytes()),
                    ("GET", "/pets/1") => (200, "application/json", format!("{{\"id\":1,\"name\":{:?}}}", header("x-trace")).into_bytes()),
                    ("GET", "/pets/2") => (404, "application/json", b"{\"message\":\"no\"}".to_vec()),
                    ("GET", "/pets/3") => (500, "text/plain", b"boom".to_vec()),
                    ("DELETE", "/pets/1") => (204, "", Vec::new()),
                    ("POST", "/upload") => (200, "text/plain", format!("{}\n{text}", header("content-type")).into_bytes()),
                    ("PUT", "/raw") => (200, "application/octet-stream", [header("content-type").as_bytes(), b"|", &body].concat()),
                    ("GET", "/slow") => {
                        tokio::time::sleep(Duration::from_secs(2)).await;
                        (200, "application/json", b"{}".to_vec())
                    }
                    _ => (400, "text/plain", Vec::new()),
                };
                let mut response = format!("HTTP/1.1 {status} X\r\ncontent-length: {}\r\nconnection: close\r\n", out.len());
                if !ctype.is_empty() {
                    response.push_str(&format!("content-type: {ctype}\r\n"));
                }
                response.push_str("\r\n");
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.write_all(&out).await;
            });
        }
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn generated_functions_call_a_server() {
    let base = serve().await;
    let client = Client::builder(&base).build();

    // Optional parameters go in the options, which default to nothing.
    let all = list_pets(&client, Default::default()).await.unwrap();
    assert_eq!(all.data[0].name, "/pets");
    let some = list_pets(&client, ListPetsOptions { limit: Some(2), tags: Some(vec!["a".into(), "b".into()]), ..Default::default() }).await.unwrap();
    assert_eq!(some.status, 200);
    assert_eq!(some.data[0].name, "/pets?limit=2&tags=a&tags=b");

    // Required parameters and bodies are arguments.
    let pet = get_pet(&client, 1, GetPetOptions { x_trace: Some("t1".into()), ..Default::default() }).await.unwrap();
    assert_eq!(pet.data, Pet { id: 1, name: "t1".into() });
    let created = create_pet(&client, NewPet { name: "n".into() }, Default::default()).await.unwrap();
    assert_eq!(created.status, 201);
    assert_eq!(created.data.name, r#"{"name":"n"}"#);
    let deleted = delete_pet(&client, 1, Default::default()).await.unwrap();
    assert_eq!(deleted.status, 204);

    // Documented errors have their type; other statuses keep the payload.
    match get_pet(&client, 2, Default::default()).await {
        Err(Failure::Http { status: 404, body: ErrorBody::Documented(p), .. }) => assert_eq!(p.message, "no"),
        other => panic!("{other:?}"),
    }
    match create_pet(&client, NewPet { name: "bad".into() }, Default::default()).await {
        Err(Failure::Http { status: 422, body: ErrorBody::Documented(p), .. }) => assert_eq!(p.message, "bad name"),
        other => panic!("{other:?}"),
    }
    match get_pet(&client, 3, Default::default()).await {
        Err(e @ Failure::Http { status: 500, body: ErrorBody::Undocumented(_), .. }) => assert_eq!(e.kind(), "http"),
        other => panic!("{other:?}"),
    }

    // Multipart bodies send files as parts; text responses are strings.
    let file = File { content: b"hello".to_vec(), filename: Some("a.txt".into()), content_type: Some("text/plain".into()) };
    let body = UploadBody { file, note: Some("hi".into()) };
    let echoed = upload(&client, UploadOptions { body: Some(body), ..Default::default() }).await.unwrap();
    assert!(echoed.data.starts_with("multipart/form-data; boundary="), "{}", echoed.data);
    assert!(echoed.data.contains("filename=\"a.txt\""), "{}", echoed.data);
    assert!(echoed.data.contains("hello"), "{}", echoed.data);
    assert!(echoed.data.contains("name=\"note\""), "{}", echoed.data);

    // Several media types: one variant each; binary bodies and responses are bytes.
    let raw = File { content: b"xyz".to_vec(), ..File::default() };
    let bytes = put_raw(&client, PutRawBody::OctetStream(raw), Default::default()).await.unwrap();
    assert_eq!(bytes.data, b"application/octet-stream|xyz");
    let json = put_raw(&client, PutRawBody::Json(Pet { id: 1, name: "p".into() }), Default::default()).await.unwrap();
    assert_eq!(json.data, br#"application/json|{"id":1,"name":"p"}"#);

    // The timeout of a call.
    let timed_out = slow(&client, SlowOptions { timeout: Some(Duration::from_millis(100)) }).await;
    assert!(matches!(timed_out, Err(Failure::Timeout { .. })), "{timed_out:?}");
}
"##;

#[test]
fn generated_functions_call_a_server() {
    let set = DocumentSet::from_value(parse_document(OPERATIONS.as_bytes()).unwrap());
    let files = files(&set);
    let module = files
        .iter()
        .find(|(p, _)| p == "mod.rs")
        .map(|(_, c)| c.clone())
        .unwrap();
    assert!(module.contains("pub mod operations;"), "{module}");
    assert!(
        module.contains(&format!(
            "// Requires tsurara-client {}",
            tsurara_lang_rust::RUNTIME_REQUIREMENT
        )),
        "{module}"
    );
    let checker = RustChecker {
        tests: vec![("calls".into(), CALLS.into())],
        http: true,
        mocks: false,
        ..Default::default()
    };
    checker.check(&files).unwrap();
}
