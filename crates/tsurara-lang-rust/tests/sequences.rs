//! Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences
//! are `tsurara_client::Items` of typed items, read with `Client::request_items`.

use tsurara_lang_rust::{RustOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, RustChecker, repo_root};

fn files() -> Vec<(String, String)> {
    let text = std::fs::read(repo_root().join("corpus/edge-cases/event-streams.yaml")).unwrap();
    let set = DocumentSet::from_value(parse_document(&text).unwrap());
    generate(&set, &Options::default(), &RustOptions::default())
        .unwrap()
        .files
        .into_iter()
        .map(|f| (f.path, f.contents))
        .collect()
}

fn file<'a>(files: &'a [(String, String)], path: &str) -> &'a str {
    &files
        .iter()
        .find(|(p, _)| p == path)
        .unwrap_or_else(|| panic!("no {path}"))
        .1
}

#[test]
fn sequences_are_items() {
    let files = files();
    let stream = file(&files, "operations/stream.rs");
    assert!(
        stream.contains("-> tsurara_client::ApiResult<tsurara_client::Items<StreamEvent>, "),
        "{stream}"
    );
    assert!(
        stream.contains("client.request_items(&DEF, input).await"),
        "{stream}"
    );
    assert!(
        stream.contains(r#""parseAs":"events","jsonEvents":["note","noteDeleted"]"#),
        "{stream}"
    );
    let lines = file(&files, "operations/lines.rs");
    assert!(lines.contains("tsurara_client::Items<Note>"), "{lines}");
    assert!(lines.contains(r#""parseAs":"jsonLines""#), "{lines}");
    // JSON next to a stream is read as JSON, as before.
    let both = file(&files, "operations/both.rs");
    assert!(both.contains("tsurara_client::ApiResult<Note, "), "{both}");
    assert!(both.contains("client.request(&DEF, input).await"), "{both}");
}

const READ: &str = r##"
use tsurara_generated::api::operations::*;
use tsurara_client::{BoxFuture, Client, HttpRequest, HttpResponse, Transport, TransportError};

struct Answer;

impl Transport for Answer {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        let (ctype, body): (&str, &str) = match request.url.as_str() {
            "https://api.test/stream" => ("text/event-stream", "event: note\ndata: {\"id\":\"1\",\"text\":\"hi\"}\nid: 5\n\nevent: noteDeleted\ndata: {\"id\":\"1\"}\n\nevent: unknown\ndata: x\n\n"),
            "https://api.test/mixed" => ("text/event-stream", "event: note\ndata: {\"id\":\"2\"}\n\nevent: pong\ndata: x\n\n"),
            "https://api.test/lines" => ("application/x-ndjson", "{\"id\":\"3\"}\n{\"id\":\"4\"}\n"),
            _ => ("application/json-seq", "\u{1e}{\"count\":1}\n"),
        };
        let mut response = HttpResponse::new(200);
        response.headers.push(("content-type".into(), ctype.into()));
        response.body = body.as_bytes().to_vec();
        Box::pin(async move { Ok(response) })
    }
}

#[tokio::test]
async fn generated_functions_read_typed_items() {
    let client = Client::builder("https://api.test").transport(Answer).build();

    let mut events = stream(&client, Default::default()).await.unwrap().data;
    let mut seen = Vec::new();
    while let Some(e) = events.next().await {
        seen.push(format!("{:?}", e.unwrap()));
    }
    assert_eq!(seen.len(), 3, "{seen:?}");
    assert!(seen[0].contains("StreamNoteEvent") && seen[0].contains("\"hi\""), "{seen:?}");
    assert!(seen[1].contains("StreamNoteDeletedEvent"), "{seen:?}");
    // An event the definition does not list is kept.
    assert!(seen[2].contains("unknown"), "{seen:?}");

    let mut mixed = mixed(&client, Default::default()).await.unwrap().data;
    let first = format!("{:?}", mixed.next().await.unwrap().unwrap());
    assert!(first.contains("MixedNoteEvent") && first.contains("\"2\""), "{first}");
    let second = format!("{:?}", mixed.next().await.unwrap().unwrap());
    assert!(second.contains("MixedPingEvent") && second.contains("\"x\""), "{second}");
    assert!(mixed.next().await.is_none());

    let mut notes = lines(&client, Default::default()).await.unwrap().data;
    let mut ids = Vec::new();
    while let Some(n) = notes.next().await {
        ids.push(n.unwrap().id);
    }
    assert_eq!(ids, ["3", "4"]);

    let mut counts = seq(&client, Default::default()).await.unwrap().data;
    assert_eq!(counts.next().await.unwrap().unwrap().count, 1);
    assert!(counts.next().await.is_none());
}
"##;

#[test]
fn generated_functions_read_typed_items() {
    RustChecker {
        tests: vec![("read".into(), READ.into())],
        http: false,
        mocks: false,
        ..Default::default()
    }
    .check(&files())
    .unwrap();
}
