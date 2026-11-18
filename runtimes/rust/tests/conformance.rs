//! Runs the language-neutral conformance vectors in spec/conformance (design.md 18.9).

use serde_json::{Map, Value, json};
use std::sync::{Arc, Mutex};
use tsurara_client::{
    Body, BodyReader, BoxFuture, Client, ErrorBody, Failure, File, HttpRequest, HttpResponse,
    Input, OperationDef, Part, Payload, StreamingResponse, Transport, TransportError,
};

fn load(name: &str) -> Vec<Value> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/conformance")
        .join(name);
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

/// Answers every request with `response` and records the requests.
#[derive(Clone)]
struct Mock {
    response: HttpResponse,
    sent: Arc<Mutex<Vec<HttpRequest>>>,
}

impl Transport for Mock {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        self.sent.lock().unwrap().push(request);
        let response = self.response.clone();
        Box::pin(async move { Ok(response) })
    }
}

fn file(v: &Value) -> Option<File> {
    if let Some(b) = v.get("$blob") {
        return Some(File {
            content: b["text"].as_str().unwrap().as_bytes().to_vec(),
            filename: None,
            content_type: b["type"].as_str().map(str::to_owned),
        });
    }
    let f = v.get("$file")?;
    Some(File {
        content: f["text"].as_str().unwrap().as_bytes().to_vec(),
        filename: f["name"].as_str().map(str::to_owned),
        content_type: f["type"].as_str().map(str::to_owned),
    })
}

fn pairs(v: &Value) -> Vec<(String, Value)> {
    v.as_object()
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default()
}

/// The input of a vector, with tagged binary values as files.
fn input(v: &Value) -> Input {
    let body = v.get("body").map(|b| {
        if let Some(f) = file(b) {
            return Body::Binary(f);
        }
        match b.as_object() {
            Some(m) if m.values().any(|x| file(x).is_some()) => Body::Multipart(
                m.iter()
                    .map(|(k, x)| {
                        (
                            k.clone(),
                            file(x).map_or_else(|| Part::Value(x.clone()), Part::File),
                        )
                    })
                    .collect(),
            ),
            _ => Body::Value(b.clone()),
        }
    });
    Input {
        path: pairs(&v["path"]),
        query: pairs(&v["query"]),
        headers: pairs(&v["headers"]),
        cookies: pairs(&v["cookies"]),
        querystring: v.get("querystring").cloned(),
        body,
        content_type: v
            .get("contentType")
            .and_then(Value::as_str)
            .map(str::to_owned),
        ..Input::default()
    }
}

fn client(
    response: HttpResponse,
    credentials: Map<String, Value>,
) -> (Client, Arc<Mutex<Vec<HttpRequest>>>) {
    let sent = Arc::new(Mutex::new(Vec::new()));
    let client = Client::builder("https://api.test")
        .transport(Mock {
            response,
            sent: sent.clone(),
        })
        .auth(move |scheme, _| {
            credentials
                .get(&scheme.scheme)
                .and_then(Value::as_str)
                .map(str::to_owned)
        })
        .build();
    (client, sent)
}

fn kind<S, E>(r: &Result<S, Failure<E>>) -> &'static str {
    match r {
        Ok(_) => "success",
        Err(f) => f.kind(),
    }
}

/// Parses a multipart body into parts: a string for text parts, `(filename, type, text)` otherwise.
fn multipart(content_type: &str, body: &[u8]) -> Vec<(String, Value)> {
    let boundary = content_type.split("boundary=").nth(1).unwrap();
    let text = String::from_utf8_lossy(body);
    text.split(&format!("--{boundary}"))
        .filter(|p| !p.trim().is_empty() && p.trim() != "--")
        .map(|part| {
            let (head, content) = part
                .trim_start_matches("\r\n")
                .split_once("\r\n\r\n")
                .unwrap();
            let content = content.strip_suffix("\r\n").unwrap_or(content);
            let field = |key: &str| {
                head.split(&format!("{key}=\""))
                    .nth(1)
                    .map(|r| r.split('"').next().unwrap().to_owned())
            };
            let ctype = head
                .lines()
                .find_map(|l| l.strip_prefix("Content-Type: ").map(str::to_owned));
            let name = field("name").unwrap();
            match (field("filename"), ctype) {
                (None, None) => (name, json!(content)),
                (filename, ctype) => (name, json!([filename, ctype, content])),
            }
        })
        .collect()
}

#[tokio::test]
async fn request_vectors() {
    let mut failures = Vec::new();
    for v in load("requests.json") {
        let name = v["name"].as_str().unwrap().to_owned();
        let def: OperationDef = serde_json::from_value(v["operation"].clone()).unwrap();
        let credentials = v
            .get("credentials")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let (client, sent) = client(HttpResponse::new(204), credentials);
        let r = client.request_raw(&def, input(&v["input"])).await;
        let expect = &v["expect"];
        let sent = sent.lock().unwrap();
        let mut check = |ok: bool, what: String| {
            if !ok {
                failures.push(format!("{name}: {what}"));
            }
        };
        if let Some(k) = expect.get("kind") {
            check(
                kind(&r) == k && sent.is_empty(),
                format!("expected {k} without a request, got {}", kind(&r)),
            );
            continue;
        }
        let Some(req) = sent.first() else {
            check(
                false,
                format!("nothing sent: {:?}", r.err().map(|e| e.to_string())),
            );
            continue;
        };
        check(
            req.url == expect["url"],
            format!("url {} != {}", req.url, expect["url"]),
        );
        check(
            req.method == expect["method"],
            format!("method {}", req.method),
        );
        for (h, want) in expect
            .get("headers")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            let got = req.header(h);
            check(
                got == want.as_str(),
                format!("header {h}: {got:?} != {want}"),
            );
        }
        let want = &expect["body"];
        if want.is_null() {
            check(
                req.body.is_empty(),
                format!("unexpected body {:?}", String::from_utf8_lossy(&req.body)),
            );
        } else if let Some(text) = want.get("text") {
            check(
                String::from_utf8_lossy(&req.body) == text.as_str().unwrap(),
                format!("body {:?}", String::from_utf8_lossy(&req.body)),
            );
        } else if let Some(bytes) = want.get("bytes") {
            check(
                String::from_utf8_lossy(&req.body) == bytes["text"].as_str().unwrap(),
                "bytes".into(),
            );
        } else {
            let got = multipart(req.header("content-type").unwrap_or_default(), &req.body);
            let form = want["form"].as_array().unwrap();
            check(
                got.len() == form.len(),
                format!("{} parts, expected {}", got.len(), form.len()),
            );
            for ((name, value), w) in got.iter().zip(form) {
                let w_name = w[0].as_str().unwrap();
                let w_value = &w[1];
                let expected = match (w_value.get("$file"), w_value.get("$blob")) {
                    (Some(f), _) => json!([f["name"], f["type"], f["text"]]),
                    (None, Some(b)) => json!([
                        value.get(0).cloned().unwrap_or(Value::Null),
                        b["type"],
                        b["text"]
                    ]),
                    _ => w_value.clone(),
                };
                check(
                    name == w_name && *value == expected,
                    format!("part {name}: {value} != {expected}"),
                );
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn neutral(payload: &Payload) -> Value {
    match payload {
        Payload::None => Value::Null,
        Payload::Json(v) => v.clone(),
        Payload::Text(t) => json!(t),
        Payload::Bytes {
            content,
            content_type,
        } => {
            json!({ "$blob": { "type": content_type, "text": String::from_utf8_lossy(content) } })
        }
    }
}

#[tokio::test]
async fn response_vectors() {
    let mut failures = Vec::new();
    for v in load("responses.json") {
        let name = v["name"].as_str().unwrap().to_owned();
        let def: OperationDef = serde_json::from_value(v["operation"].clone()).unwrap();
        let r = &v["response"];
        let mut response = HttpResponse::new(r["status"].as_u64().unwrap() as u16);
        for (k, h) in r["headers"].as_object().into_iter().flatten() {
            response
                .headers
                .push((k.clone(), h.as_str().unwrap().to_owned()));
        }
        response.body = r["body"]["text"]
            .as_str()
            .unwrap_or_default()
            .as_bytes()
            .to_vec();
        let (client, _) = client(response, Map::new());
        let result = client.request_raw(&def, Input::default()).await;
        let expect = &v["expect"];
        let mut problems = Vec::new();
        if kind(&result) != expect["kind"] {
            problems.push(format!("kind {} != {}", kind(&result), expect["kind"]));
        }
        match &result {
            Ok(s) => {
                if s.status != expect["status"] || neutral(&s.data) != expect["data"] {
                    problems.push(format!("{} {}", s.status, neutral(&s.data)));
                }
            }
            Err(Failure::Http { status, body, .. }) => {
                let (documented, payload) = match body {
                    ErrorBody::Documented(p) => (true, p),
                    ErrorBody::Undocumented(p) => (false, p),
                };
                if *status != expect["status"]
                    || documented != expect["documented"]
                    || neutral(payload) != expect["error"]
                {
                    problems.push(format!("{status} {documented} {}", neutral(payload)));
                }
            }
            Err(_) => {}
        }
        if !problems.is_empty() {
            failures.push(format!("{name}: {}", problems.join(", ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Answers with a body that arrives in chunks, one per read.
struct Chunks {
    status: u16,
    headers: Vec<(String, String)>,
    chunks: Vec<Vec<u8>>,
}

struct ChunkReader(std::collections::VecDeque<Vec<u8>>);

impl BodyReader for ChunkReader {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, TransportError>> {
        let chunk = self.0.pop_front();
        Box::pin(async move { Ok(chunk) })
    }
}

impl Transport for Chunks {
    fn send(&self, _: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        unreachable!("the client reads responses with send_streaming")
    }

    fn send_streaming(
        &self,
        _: HttpRequest,
    ) -> BoxFuture<'_, Result<StreamingResponse, TransportError>> {
        let response = StreamingResponse {
            status: self.status,
            headers: self.headers.clone(),
            body: Box::new(ChunkReader(self.chunks.iter().cloned().collect())),
        };
        Box::pin(async move { Ok(response) })
    }
}

#[tokio::test]
async fn sequence_vectors() {
    let mut failures = Vec::new();
    for v in load("sequences.json") {
        let name = v["name"].as_str().unwrap().to_owned();
        let def: OperationDef = serde_json::from_value(v["operation"].clone()).unwrap();
        let r = &v["response"];
        let client = Client::builder("https://api.test")
            .transport(Chunks {
                status: r["status"].as_u64().unwrap() as u16,
                headers: r["headers"]
                    .as_object()
                    .unwrap()
                    .iter()
                    .map(|(k, h)| (k.clone(), h.as_str().unwrap().to_owned()))
                    .collect(),
                chunks: r["chunks"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| c.as_str().unwrap().as_bytes().to_vec())
                    .collect(),
            })
            .build();
        let result = client
            .request_items::<Value, Value>(&def, Input::default())
            .await;
        let expect = &v["expect"];
        let mut problems = Vec::new();
        if kind(&result) != expect["kind"] {
            problems.push(format!("kind {} != {}", kind(&result), expect["kind"]));
        }
        match result {
            Ok(mut s) => {
                let mut items = Vec::new();
                let mut failed = false;
                while let Some(item) = s.data.next().await {
                    match item {
                        Ok(v) => items.push(v),
                        Err(_) => {
                            failed = true;
                            break;
                        }
                    }
                }
                if s.status != expect["status"]
                    || Value::Array(items.clone()) != expect["items"]
                    || failed != expect["error"].as_bool().unwrap_or(false)
                {
                    problems.push(format!("{} {items:?} failed={failed}", s.status));
                }
            }
            Err(Failure::Http {
                status,
                body: ErrorBody::Documented(e),
                ..
            }) => {
                if status != expect["status"] || e != expect["error"] {
                    problems.push(format!("{status} {e}"));
                }
            }
            Err(f) => problems.push(f.to_string()),
        }
        if !problems.is_empty() {
            failures.push(format!("{name}: {}", problems.join(", ")));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
