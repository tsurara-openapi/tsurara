//! Client behavior beyond the conformance vectors.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tsurara_client::{
    BoxFuture, Client, ErrorBody, Failure, HttpRequest, HttpResponse, Input, OperationDef, Retry,
    Transport, TransportError, serde::Deserialize,
};

fn def() -> OperationDef {
    OperationDef::from_json(
        r#"{ "v": 1, "id": "getPet", "method": "GET", "path": "/pets/{id}",
             "params": { "path": { "id": {} } },
             "responses": { "200": { "parseAs": "json" }, "404": { "parseAs": "json" } } }"#,
    )
}

fn input() -> Input {
    Input {
        path: vec![("id".into(), 1.into())],
        ..Input::default()
    }
}

/// Answers with the responses in order (the last one repeats).
struct Script {
    responses: Vec<HttpResponse>,
    calls: Arc<AtomicUsize>,
    delay: Duration,
}

impl Transport for Script {
    fn send(&self, _: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        let response = self.responses[n.min(self.responses.len() - 1)].clone();
        let delay = self.delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            Ok(response)
        })
    }
}

fn json(status: u16, body: &str) -> HttpResponse {
    HttpResponse {
        status,
        headers: vec![("content-type".into(), "application/json".into())],
        body: body.as_bytes().to_vec(),
    }
}

fn script(responses: Vec<HttpResponse>) -> (Script, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    (
        Script {
            responses,
            calls: calls.clone(),
            delay: Duration::ZERO,
        },
        calls,
    )
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(crate = "tsurara_client::serde")]
struct Pet {
    id: i64,
    name: String,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(crate = "tsurara_client::serde")]
struct Problem {
    title: String,
}

#[tokio::test]
async fn bodies_decode_into_types() {
    let (t, _) = script(vec![json(200, r#"{"id":1,"name":"Rex"}"#)]);
    let client = Client::builder("https://x").transport(t).build();
    let ok = client
        .request::<Pet, Problem>(&def(), input())
        .await
        .unwrap();
    assert_eq!(
        ok.data,
        Pet {
            id: 1,
            name: "Rex".into()
        }
    );

    let (t, _) = script(vec![json(404, r#"{"title":"Not Found"}"#)]);
    let client = Client::builder("https://x").transport(t).build();
    match client.request::<Pet, Problem>(&def(), input()).await {
        Err(Failure::Http {
            status: 404,
            body: ErrorBody::Documented(p),
            ..
        }) => assert_eq!(p.title, "Not Found"),
        other => panic!("{other:?}"),
    }

    // A body that does not match the type is a parse failure, and `?` works with Failure.
    let (t, _) = script(vec![json(200, r#"{"id":"one"}"#)]);
    let client = Client::builder("https://x").transport(t).build();
    let r: Result<(), Box<dyn std::error::Error>> = async {
        client.request::<Pet, Problem>(&def(), input()).await?;
        Ok(())
    }
    .await;
    assert!(
        r.unwrap_err()
            .to_string()
            .contains("getPet failed with parse")
    );
}

#[tokio::test]
async fn retries_honor_retry_after_and_stop_at_the_timeout() {
    let mut busy = json(503, "{}");
    busy.headers.push(("retry-after".into(), "0".into()));
    let (t, calls) = script(vec![busy.clone(), json(200, r#"{"id":1,"name":"a"}"#)]);
    let client = Client::builder("https://x")
        .transport(t)
        .retry(Retry::default())
        .build();
    assert!(
        client
            .request::<Pet, Problem>(&def(), input())
            .await
            .is_ok()
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);

    let mut later = json(429, "{}");
    later.headers.push(("retry-after".into(), "10".into()));
    let (t, calls) = script(vec![later]);
    let client = Client::builder("https://x")
        .transport(t)
        .retry(Retry::default())
        .timeout(Duration::from_secs(1))
        .build();
    let r = client.request::<Pet, Problem>(&def(), input()).await;
    assert_eq!(r.unwrap_err().status(), Some(429));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn timeouts_and_hooks() {
    let calls = Arc::new(AtomicUsize::new(0));
    let slow = Script {
        responses: vec![json(200, "{}")],
        calls,
        delay: Duration::from_secs(5),
    };
    let client = Client::builder("https://x")
        .transport(slow)
        .timeout(Duration::from_millis(20))
        .build();
    let r = client.request_raw(&def(), input()).await;
    assert_eq!(r.unwrap_err().kind(), "timeout");

    let seen = Arc::new(Mutex::new(Vec::new()));
    let (t, _) = script(vec![json(201, "{}")]);
    let s1 = seen.clone();
    let s2 = seen.clone();
    let client = Client::builder("https://x")
        .transport(t)
        .on_undocumented_status(move |op, status| {
            s1.lock().unwrap().push(format!("{} {status}", op.id))
        })
        .on_result(move |op, kind| s2.lock().unwrap().push(format!("{} {kind}", op.id)))
        .build();
    let r = client.request_raw(&def(), input()).await;
    assert!(matches!(
        r,
        Err(Failure::Http {
            status: 201,
            body: ErrorBody::Undocumented(_),
            ..
        })
    ));
    assert_eq!(*seen.lock().unwrap(), ["getPet 201", "getPet http"]);
}

#[tokio::test]
async fn the_reqwest_transport_talks_to_a_server() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let request = String::from_utf8_lossy(&buf[..n]).into_owned();
        let body = r#"{"id":7,"name":"Rex"}"#;
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
        request
    });
    let client = Client::builder(&format!("http://{addr}"))
        .header("x-trace", "t1")
        .build();
    let pet = client
        .request::<Pet, Problem>(&def(), input())
        .await
        .unwrap()
        .data;
    assert_eq!(
        pet,
        Pet {
            id: 7,
            name: "Rex".into()
        }
    );
    let request = server.await.unwrap();
    assert!(request.starts_with("GET /pets/1 HTTP/1.1"), "{request}");
    assert!(
        request.to_ascii_lowercase().contains("x-trace: t1"),
        "{request}"
    );
}

#[tokio::test]
async fn values_that_failed_to_encode_are_unexpected_without_a_request() {
    let (script, calls) = script(vec![json(200, "{}")]);
    let client = Client::builder("https://api.test")
        .transport(script)
        .build();
    let mut input = input();
    input.encode_error = Some("broken".into());
    let r = client.request_raw(&def(), input).await;
    assert!(matches!(r, Err(Failure::Unexpected { ref cause, .. }) if cause.contains("broken")));
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[derive(Debug, PartialEq, serde::Deserialize)]
struct Tick {
    event: String,
    data: Pet,
    id: Option<String>,
}

#[tokio::test]
async fn events_arrive_as_the_server_writes_them_and_dropping_closes() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (first_read, wait_for_reader) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let _ = socket.read(&mut buf).await.unwrap();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        let chunk = |s: &str| format!("{:x}\r\n{s}\r\n", s.len());
        socket
            .write_all(chunk("event: tick\ndata: {\"id\":1,\"name\":\"a\"}\nid: 1\n\n").as_bytes())
            .await
            .unwrap();
        // The second event is written only once the client has read the first.
        wait_for_reader.await.unwrap();
        socket
            .write_all(chunk("event: tick\ndata: {\"id\":2,\"name\":\"b\"}\n\n").as_bytes())
            .await
            .unwrap();
        // The client drops the stream; reading then ends.
        let mut rest = Vec::new();
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            socket.read_to_end(&mut rest),
        )
        .await
        .expect("the client closes the connection");
    });
    let def: OperationDef = serde_json::from_value(serde_json::json!({
        "v": 1, "id": "ticks", "method": "GET", "path": "/ticks",
        "responses": { "200": { "parseAs": "events", "jsonEvents": true } }
    }))
    .unwrap();
    let client = Client::builder(&format!("http://{addr}"))
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let mut ticks = client
        .request_items::<Tick, Problem>(&def, Input::default())
        .await
        .unwrap()
        .data;
    let first = ticks.next().await.unwrap().unwrap();
    assert_eq!(
        first.data,
        Pet {
            id: 1,
            name: "a".into()
        }
    );
    assert_eq!(first.id.as_deref(), Some("1"));
    first_read.send(()).unwrap();
    let second = ticks.next().await.unwrap().unwrap();
    assert_eq!((second.event.as_str(), second.data.id), ("tick", 2));
    drop(ticks);
    server.await.unwrap();
}

/// Answers each connection with the next response, and returns the requests it read.
#[cfg(feature = "reqwest")]
async fn serve(
    listener: tokio::net::TcpListener,
    responses: Vec<String>,
) -> tokio::task::JoinHandle<Vec<String>> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    tokio::spawn(async move {
        let mut requests = Vec::new();
        for response in responses {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            requests.push(String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase());
            socket.write_all(response.as_bytes()).await.unwrap();
        }
        requests
    })
}

#[cfg(feature = "reqwest")]
fn redirect(status: &str, location: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
    )
}

#[cfg(feature = "reqwest")]
#[tokio::test]
async fn redirects_are_followed_within_the_origin_only() {
    let other = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let other_addr = other.local_addr().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let body = r#"{"id":2,"name":"Rex"}"#;
    let ok = format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let server = serve(
        listener,
        vec![
            redirect("302 Found", "/pets/2"),
            ok,
            redirect(
                "307 Temporary Redirect",
                &format!("http://{other_addr}/pets/9"),
            ),
        ],
    )
    .await;
    let client = Client::builder(&format!("http://{addr}"))
        .header("x-api-key", "secret")
        .timeout(Duration::from_secs(5))
        .build();

    let pet = client
        .request::<Pet, Problem>(&def(), input())
        .await
        .unwrap();
    assert_eq!(pet.data.id, 2);

    let r = client.request::<Pet, Problem>(&def(), input()).await;
    assert!(
        matches!(r, Err(Failure::Http { status: 307, .. })),
        "a redirect to another origin is the result: {r:?}"
    );
    let requests = server.await.unwrap();
    assert!(requests[1].starts_with("get /pets/2 "), "{}", requests[1]);
    assert!(requests[1].contains("x-api-key: secret"), "{}", requests[1]);
    let leaked = tokio::time::timeout(Duration::from_millis(200), other.accept()).await;
    assert!(leaked.is_err(), "the other origin was connected to");
}

#[tokio::test]
async fn text_bodies_are_decoded_with_their_charset() {
    let def = OperationDef::from_json(
        r#"{ "v": 1, "id": "export", "method": "GET", "path": "/export",
             "responses": { "200": { "parseAs": "text" } } }"#,
    );
    let text = |content_type: &str, body: &[u8]| HttpResponse {
        status: 200,
        headers: vec![("content-type".into(), content_type.into())],
        body: body.to_vec(),
    };
    // `名前,値` in Shift_JIS.
    let shift_jis = [150, 188, 145, 79, 44, 146, 108];
    for (content_type, body) in [
        ("text/csv; charset=Shift_JIS", &shift_jis[..]),
        ("text/csv; charset=\"shift_jis\"", &shift_jis[..]),
        ("text/csv", "名前,値".as_bytes()),
    ] {
        let (t, _) = script(vec![text(content_type, body)]);
        let client = Client::builder("https://x").transport(t).build();
        let ok = client
            .request::<String, String>(&def, Input::default())
            .await
            .unwrap();
        assert_eq!(ok.data, "名前,値", "{content_type}");
    }
    let (t, _) = script(vec![text("text/csv; charset=x-no-such-charset", b"x")]);
    let client = Client::builder("https://x").transport(t).build();
    match client
        .request::<String, String>(&def, Input::default())
        .await
    {
        Err(Failure::Parse { cause, .. }) => {
            assert!(cause.contains("x-no-such-charset"), "{cause}")
        }
        other => panic!("{other:?}"),
    }
}
