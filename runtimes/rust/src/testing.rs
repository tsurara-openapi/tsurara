//! A mock transport for tests (feature `testing`): answers from handlers routed by
//! operation, so tests need neither a server nor a mocking library. Routing follows spec/README.md
//! (Mock transports), shared with the runtimes of the other languages.
//!
//! ```ignore
//! let mock = MockTransport::new()
//!     .base_path("/v1")
//!     .operation(&get_pet_def(), |req| Ok(MockResponse::json(200, &json!({ "id": req.params["petId"] }))));
//! let client = Client::builder("https://api.test/v1").transport(mock.clone()).build();
//! ```

use crate::{
    BoxError, BoxFuture, HttpRequest, HttpResponse, OperationDef, Transport, TransportError,
};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// What a handler receives.
#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    pub url: String,
    /// Path parameters, percent-decoded.
    pub params: BTreeMap<String, String>,
    /// The query pairs, percent-decoded.
    pub query: Vec<(String, String)>,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// What a handler returns.
#[derive(Debug, Clone, Default)]
pub struct MockResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl MockResponse {
    /// A response without a body.
    pub fn new(status: u16) -> Self {
        Self {
            status,
            ..Self::default()
        }
    }

    /// A JSON body with `Content-Type: application/json`.
    pub fn json(status: u16, value: &impl serde::Serialize) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: serde_json::to_vec(value).expect("the value serializes"),
        }
    }

    /// A text body with its Content-Type.
    pub fn text(status: u16, content_type: &str, text: &str) -> Self {
        Self {
            status,
            headers: vec![("content-type".into(), content_type.into())],
            body: text.as_bytes().to_vec(),
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_ascii_lowercase(), value.into()));
        self
    }
}

/// A recorded call.
#[derive(Debug, Clone)]
pub struct MockCall {
    pub method: String,
    pub url: String,
    /// The operation id of the route, when the route was given an OperationDef.
    pub route: Option<String>,
    pub params: BTreeMap<String, String>,
    pub body: Vec<u8>,
}

type Handler = Arc<dyn Fn(&MockRequest) -> Result<MockResponse, BoxError> + Send + Sync>;

enum Part {
    Literal(String),
    Param(String),
}

struct Route {
    method: String,
    /// Per segment: its parts (one literal part for a literal segment).
    segments: Vec<Vec<Part>>,
    literals: usize,
    operation: Option<String>,
    handler: Handler,
}

#[derive(Default)]
struct State {
    calls: Vec<MockCall>,
    unmatched: Vec<MockCall>,
}

/// A [`Transport`] that answers from handlers. Clones share the routes and the recorded calls, so a
/// test keeps one to read [`MockTransport::calls`] after giving another to the client.
#[derive(Clone, Default)]
pub struct MockTransport {
    routes: Arc<Vec<Route>>,
    base_path: String,
    record_unmatched: bool,
    state: Arc<Mutex<State>>,
}

impl MockTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Removed from request paths before routing (the path of the client's base URL).
    pub fn base_path(mut self, path: &str) -> Self {
        self.base_path = path.to_owned();
        self
    }

    /// Answer unmatched requests with 501 and record them, instead of failing the transport.
    pub fn record_unmatched(mut self) -> Self {
        self.record_unmatched = true;
        self
    }

    /// Answers the operation's method and path template.
    pub fn operation(
        self,
        def: &OperationDef,
        handler: impl Fn(&MockRequest) -> Result<MockResponse, BoxError> + Send + Sync + 'static,
    ) -> Self {
        self.add(
            &def.method,
            &def.path,
            Some(def.id.clone()),
            Arc::new(handler),
        )
    }

    /// Answers `method` and the path template `path`.
    pub fn route(
        self,
        method: &str,
        path: &str,
        handler: impl Fn(&MockRequest) -> Result<MockResponse, BoxError> + Send + Sync + 'static,
    ) -> Self {
        self.add(method, path, None, Arc::new(handler))
    }

    fn add(
        mut self,
        method: &str,
        path: &str,
        operation: Option<String>,
        handler: Handler,
    ) -> Self {
        let mut literals = 0;
        let segments = path
            .split('/')
            .map(|segment| {
                let mut parts = Vec::new();
                let mut rest = segment;
                while let Some(start) = rest.find('{') {
                    let Some(end) = rest[start..].find('}') else {
                        break;
                    };
                    if start > 0 {
                        parts.push(Part::Literal(rest[..start].to_owned()));
                    }
                    parts.push(Part::Param(rest[start + 1..start + end].to_owned()));
                    rest = &rest[start + end + 1..];
                }
                if !rest.is_empty() || parts.is_empty() {
                    parts.push(Part::Literal(rest.to_owned()));
                }
                if parts.iter().all(|p| matches!(p, Part::Literal(_))) {
                    literals += 1;
                }
                parts
            })
            .collect();
        Arc::get_mut(&mut self.routes)
            .expect("routes are added before the transport is shared")
            .push(Route {
                method: method.to_ascii_uppercase(),
                segments,
                literals,
                operation,
                handler,
            });
        self
    }

    /// The index of the route for a request path and its decoded parameters (spec/README.md).
    pub fn matches(&self, method: &str, path: &str) -> Option<(usize, BTreeMap<String, String>)> {
        let path = path.split('?').next().unwrap_or("");
        let rest = path.strip_prefix(self.base_path.as_str())?;
        let parts: Vec<&str> = rest.split('/').collect();
        let mut best: Option<(usize, BTreeMap<String, String>, usize)> = None;
        for (index, r) in self.routes.iter().enumerate() {
            if !r.method.eq_ignore_ascii_case(method) || r.segments.len() != parts.len() {
                continue;
            }
            let mut params = BTreeMap::new();
            let ok = r
                .segments
                .iter()
                .zip(&parts)
                .all(|(segment, part)| match_segment(segment, part, &mut params));
            if ok && best.as_ref().is_none_or(|(_, _, l)| r.literals > *l) {
                best = Some((index, params, r.literals));
            }
        }
        best.map(|(index, params, _)| (index, params))
    }

    /// The calls that matched a route, in order.
    pub fn calls(&self) -> Vec<MockCall> {
        self.state.lock().expect("not poisoned").calls.clone()
    }

    /// With [`MockTransport::record_unmatched`], the requests no route matched.
    pub fn unmatched(&self) -> Vec<MockCall> {
        self.state.lock().expect("not poisoned").unmatched.clone()
    }
}

/// Matches one segment; parameters are greedy, so the last match wins inside a segment.
fn match_segment(parts: &[Part], text: &str, params: &mut BTreeMap<String, String>) -> bool {
    match parts.split_first() {
        None => text.is_empty(),
        Some((Part::Literal(l), rest)) => text
            .strip_prefix(l.as_str())
            .is_some_and(|t| match_segment(rest, t, params)),
        Some((Part::Param(name), rest)) => {
            let ends: Vec<usize> = text.char_indices().map(|(i, c)| i + c.len_utf8()).collect();
            for end in ends.into_iter().rev() {
                let mut tail = params.clone();
                if match_segment(rest, &text[end..], &mut tail) {
                    tail.insert(name.clone(), percent_decode(&text[..end]));
                    *params = tail;
                    return true;
                }
            }
            false
        }
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16);
        if bytes[i] == b'%' {
            if let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex(*b)),
                bytes.get(i + 2).and_then(|b| hex(*b)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn split_url(url: &str) -> (&str, &str) {
    // `https://host:port/path?query` → path with query.
    let after_scheme = url.split_once("://").map_or(url, |(_, rest)| rest);
    let path_start = after_scheme.find('/').unwrap_or(after_scheme.len());
    let path_and_query = &after_scheme[path_start..];
    let path = path_and_query.split('?').next().unwrap_or("");
    (path, path_and_query)
}

impl Transport for MockTransport {
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        Box::pin(async move {
            let (path, path_and_query) = split_url(&request.url);
            let path = if path.is_empty() { "/" } else { path };
            let found = self.matches(&request.method, path);
            let call = MockCall {
                method: request.method.clone(),
                url: request.url.clone(),
                route: found
                    .as_ref()
                    .and_then(|(i, _)| self.routes[*i].operation.clone()),
                params: found.as_ref().map(|(_, p)| p.clone()).unwrap_or_default(),
                body: request.body.clone(),
            };
            let Some((index, params)) = found else {
                if self.record_unmatched {
                    self.state
                        .lock()
                        .expect("not poisoned")
                        .unmatched
                        .push(call);
                    return Ok(HttpResponse::new(501));
                }
                return Err(TransportError::Network(
                    format!("tsurara mock: no route for {} {path}", request.method).into(),
                ));
            };
            self.state.lock().expect("not poisoned").calls.push(call);
            let query = path_and_query
                .split_once('?')
                .map(|(_, q)| {
                    q.split('&')
                        .filter(|p| !p.is_empty())
                        .map(|p| {
                            let (k, v) = p.split_once('=').unwrap_or((p, ""));
                            (percent_decode(k), percent_decode(v))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let mock = MockRequest {
                method: request.method.clone(),
                url: request.url.clone(),
                params,
                query,
                headers: request.headers.clone(),
                body: request.body.clone(),
            };
            match (self.routes[index].handler)(&mock) {
                Ok(r) => Ok(HttpResponse {
                    status: r.status,
                    headers: r.headers,
                    body: r.body,
                }),
                Err(e) => Err(TransportError::Network(
                    format!("tsurara mock: the handler failed: {e}").into(),
                )),
            }
        })
    }
}
