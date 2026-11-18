//! The HTTP client (feature `client`): transports, requests, Results and retries.

#[allow(unused_imports)]
use crate::{
    File, ItemError, Items, OPERATION_DEF_VERSION, OperationDef, OperationInfo, Retry, SecurityUse,
    retry_after,
};

use crate::def::{ParseAs, ResponseDef};
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// A request as the transport sends it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpRequest {
    pub method: String,
    pub url: String,
    /// Lower-case names.
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

/// A response as the transport returns it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HttpResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl HttpResponse {
    pub fn new(status: u16) -> HttpResponse {
        HttpResponse {
            status,
            ..HttpResponse::default()
        }
    }

    pub fn header(&self, name: &str) -> Option<&str> {
        header(&self.headers, name)
    }
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

#[derive(Debug)]
pub enum TransportError {
    /// The connection failed (DNS, connect, TLS, a broken connection).
    Network(BoxError),
    Timeout,
}

impl fmt::Display for TransportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportError::Network(e) => write!(f, "network error: {e}"),
            TransportError::Timeout => f.write_str("timeout"),
        }
    }
}

impl std::error::Error for TransportError {}

/// The body of a response, read a chunk at a time. Dropping it closes the response.
pub trait BodyReader: Send {
    /// The next chunk, or `None` at the end of the body.
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, TransportError>>;
}

/// A response whose headers have arrived and whose body is read as it comes.
pub struct StreamingResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: Box<dyn BodyReader>,
}

impl fmt::Debug for StreamingResponse {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StreamingResponse")
            .field("status", &self.status)
            .field("headers", &self.headers)
            .finish_non_exhaustive()
    }
}

/// A body that was read whole, handed over as one chunk.
struct Buffered(Option<Vec<u8>>);

impl BodyReader for Buffered {
    fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, TransportError>> {
        let chunk = self.0.take().filter(|c| !c.is_empty());
        Box::pin(async move { Ok(chunk) })
    }
}

/// Sends requests. The default is reqwest; tests and other platforms plug in their own.
pub trait Transport: Send + Sync {
    /// Sends a request and reads the whole response.
    fn send(&self, request: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>>;

    /// Sends a request and returns once the headers have arrived, with the body to read. The client
    /// sends every request this way, so that responses read item by item ([`Items`]) arrive as the
    /// server writes them. The default reads the whole body with [`Transport::send`] first.
    fn send_streaming(
        &self,
        request: HttpRequest,
    ) -> BoxFuture<'_, Result<StreamingResponse, TransportError>> {
        Box::pin(async move {
            let r = self.send(request).await?;
            Ok(StreamingResponse {
                status: r.status,
                headers: r.headers,
                body: Box::new(Buffered(Some(r.body))),
            })
        })
    }
}

/// Reads the rest of a body.
async fn read_all(body: &mut dyn BodyReader) -> Result<Vec<u8>, TransportError> {
    let mut out = Vec::new();
    while let Some(chunk) = body.next_chunk().await? {
        out.extend_from_slice(&chunk);
    }
    Ok(out)
}

#[cfg(feature = "reqwest")]
mod reqwest_transport {
    use super::*;

    /// The default transport over a `reqwest::Client`. The default client follows redirects within
    /// the origin of the request only: a redirect to another origin is not followed, and its response
    /// is the result, so that headers such as API keys never reach another host.
    #[derive(Debug, Clone)]
    pub struct ReqwestTransport(pub reqwest::Client);

    impl Default for ReqwestTransport {
        fn default() -> Self {
            let policy = reqwest::redirect::Policy::custom(|attempt| {
                let first = attempt.previous().first().map(|u| u.origin());
                if attempt.previous().len() >= 10 {
                    attempt.error("stopped after 10 redirects")
                } else if first.is_some_and(|o| o != attempt.url().origin()) {
                    attempt.stop()
                } else {
                    attempt.follow()
                }
            });
            let client = reqwest::Client::builder()
                .redirect(policy)
                .build()
                .unwrap_or_default();
            ReqwestTransport(client)
        }
    }

    impl Transport for ReqwestTransport {
        fn send(
            &self,
            request: HttpRequest,
        ) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
            Box::pin(async move {
                let method = reqwest::Method::from_bytes(request.method.as_bytes())
                    .map_err(|e| TransportError::Network(e.into()))?;
                let mut builder = self.0.request(method, &request.url).body(request.body);
                for (k, v) in &request.headers {
                    builder = builder.header(k, v);
                }
                let response = builder.send().await.map_err(classify)?;
                let status = response.status().as_u16();
                let headers = response
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_owned(),
                            String::from_utf8_lossy(v.as_bytes()).into_owned(),
                        )
                    })
                    .collect();
                let body = response.bytes().await.map_err(classify)?.to_vec();
                Ok(HttpResponse {
                    status,
                    headers,
                    body,
                })
            })
        }

        fn send_streaming(
            &self,
            request: HttpRequest,
        ) -> BoxFuture<'_, Result<StreamingResponse, TransportError>> {
            Box::pin(async move {
                let method = reqwest::Method::from_bytes(request.method.as_bytes())
                    .map_err(|e| TransportError::Network(e.into()))?;
                let mut builder = self.0.request(method, &request.url).body(request.body);
                for (k, v) in &request.headers {
                    builder = builder.header(k, v);
                }
                let response = builder.send().await.map_err(classify)?;
                let headers = response
                    .headers()
                    .iter()
                    .map(|(k, v)| {
                        (
                            k.as_str().to_owned(),
                            String::from_utf8_lossy(v.as_bytes()).into_owned(),
                        )
                    })
                    .collect();
                Ok(StreamingResponse {
                    status: response.status().as_u16(),
                    headers,
                    body: Box::new(ReqwestBody(response)),
                })
            })
        }
    }

    struct ReqwestBody(reqwest::Response);

    impl BodyReader for ReqwestBody {
        fn next_chunk(&mut self) -> BoxFuture<'_, Result<Option<Vec<u8>>, TransportError>> {
            Box::pin(async move { Ok(self.0.chunk().await.map_err(classify)?.map(|b| b.to_vec())) })
        }
    }

    fn classify(e: reqwest::Error) -> TransportError {
        if e.is_timeout() {
            TransportError::Timeout
        } else {
            TransportError::Network(e.into())
        }
    }
}

#[cfg(feature = "reqwest")]
pub use reqwest_transport::ReqwestTransport;

/// A multipart part.
#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Value(Value),
    File(File),
}

/// A request body.
#[derive(Debug, Clone, PartialEq)]
pub enum Body {
    /// JSON, form, text, or multipart without files: serialized by the media type.
    Value(Value),
    /// A binary body.
    Binary(File),
    /// Multipart parts in order.
    Multipart(Vec<(String, Part)>),
}

/// The input of a call: parameters as JSON values, in order.
#[derive(Debug, Clone, Default)]
pub struct Input {
    pub path: Vec<(String, Value)>,
    pub query: Vec<(String, Value)>,
    pub headers: Vec<(String, Value)>,
    pub cookies: Vec<(String, Value)>,
    pub querystring: Option<Value>,
    pub body: Option<Body>,
    /// The request media type, when the operation has several.
    pub content_type: Option<String>,
    /// Overrides the client's timeout.
    pub timeout: Option<Duration>,
    /// Why a value given to the `Input` methods could not be encoded; the request then returns
    /// `Failure::Unexpected` without being sent.
    pub encode_error: Option<String>,
}

/// A response body as read by `parseAs`.
#[derive(Debug, Clone, PartialEq)]
pub enum Payload {
    None,
    Json(Value),
    Text(String),
    Bytes {
        content: Vec<u8>,
        content_type: Option<String>,
    },
}

impl Payload {
    /// The body as a JSON value, for decoding into a typed value: text is a string, bytes are an
    /// array of numbers (which `Vec<u8>` decodes), no body is `null` (which `()` decodes).
    pub fn into_value(self) -> Value {
        match self {
            Payload::None => Value::Null,
            Payload::Json(v) => v,
            Payload::Text(t) => Value::String(t),
            Payload::Bytes { content, .. } => {
                Value::Array(content.into_iter().map(Value::from).collect())
            }
        }
    }
}

/// A success status of the definition.
#[derive(Debug, Clone)]
pub struct Success<T> {
    pub status: u16,
    pub data: T,
    pub headers: Vec<(String, String)>,
    pub operation: OperationInfo,
}

/// The body of an HTTP error.
#[derive(Debug, Clone)]
pub enum ErrorBody<E> {
    /// A status the definition documents, with its type.
    Documented(E),
    /// A status it does not document (also a 2xx when other successes are documented).
    Undocumented(Payload),
}

/// Every outcome but a success (spec/README.md, Result kinds).
#[derive(Debug)]
pub enum Failure<E> {
    Http {
        status: u16,
        body: ErrorBody<E>,
        headers: Vec<(String, String)>,
        operation: OperationInfo,
    },
    Network {
        cause: BoxError,
        operation: OperationInfo,
    },
    Timeout {
        operation: OperationInfo,
    },
    /// The body could not be read as the definition says (Content-Type, JSON, the type).
    Parse {
        status: u16,
        content_type: Option<String>,
        cause: String,
        operation: OperationInfo,
    },
    /// The input cannot be sent, or a hook failed.
    Unexpected {
        cause: String,
        operation: OperationInfo,
    },
}

impl<E> Failure<E> {
    /// The contract's kind: `http`, `network`, `timeout`, `parse` or `unexpected`.
    pub fn kind(&self) -> &'static str {
        match self {
            Failure::Http { .. } => "http",
            Failure::Network { .. } => "network",
            Failure::Timeout { .. } => "timeout",
            Failure::Parse { .. } => "parse",
            Failure::Unexpected { .. } => "unexpected",
        }
    }

    pub fn operation(&self) -> &OperationInfo {
        match self {
            Failure::Http { operation, .. }
            | Failure::Network { operation, .. }
            | Failure::Timeout { operation }
            | Failure::Parse { operation, .. }
            | Failure::Unexpected { operation, .. } => operation,
        }
    }

    /// The HTTP status, when there was a response.
    pub fn status(&self) -> Option<u16> {
        match self {
            Failure::Http { status, .. } | Failure::Parse { status, .. } => Some(*status),
            _ => None,
        }
    }
}

impl<E: fmt::Debug> fmt::Display for Failure<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = &self.operation().id;
        match self {
            Failure::Http { status, .. } => write!(f, "tsurara: {id} failed with HTTP {status}"),
            Failure::Network { cause, .. } => {
                write!(f, "tsurara: {id} failed with network: {cause}")
            }
            Failure::Timeout { .. } => write!(f, "tsurara: {id} failed with timeout"),
            Failure::Parse { cause, .. } => write!(f, "tsurara: {id} failed with parse: {cause}"),
            Failure::Unexpected { cause, .. } => {
                write!(f, "tsurara: {id} failed with unexpected: {cause}")
            }
        }
    }
}

impl<E: fmt::Debug> std::error::Error for Failure<E> {}

pub type ApiResult<S, E> = Result<Success<S>, Failure<E>>;

/// Returns the credential for a security scheme of an operation, or `None`.
pub type AuthFn = dyn Fn(&SecurityUse, &OperationInfo) -> Option<String> + Send + Sync;

/// Told about every response whose status the definition does not document.
pub type UndocumentedFn = dyn Fn(&OperationInfo, u16) + Send + Sync;

/// Observes every outcome by its kind.
pub type ResultFn = dyn Fn(&OperationInfo, &'static str) + Send + Sync;

struct Inner {
    base_url: String,
    transport: Arc<dyn Transport>,
    headers: Vec<(String, String)>,
    timeout: Option<Duration>,
    retry: Option<Retry>,
    auth: Option<Box<AuthFn>>,
    on_undocumented_status: Option<Box<UndocumentedFn>>,
    on_result: Option<Box<ResultFn>>,
}

/// A client. Cheap to clone; pass it to generated functions.
#[derive(Clone)]
pub struct Client {
    inner: Arc<Inner>,
}

pub struct ClientBuilder {
    inner: Inner,
}

impl ClientBuilder {
    /// The transport to send with (the default is reqwest).
    pub fn transport(mut self, transport: impl Transport + 'static) -> Self {
        self.inner.transport = Arc::new(transport);
        self
    }

    /// A header for every request.
    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.inner
            .headers
            .push((name.to_ascii_lowercase(), value.to_owned()));
        self
    }

    /// The timeout over all attempts of a call.
    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.inner.timeout = Some(timeout);
        self
    }

    pub fn retry(mut self, retry: Retry) -> Self {
        self.inner.retry = Some(retry);
        self
    }

    /// Credentials for the operations' security schemes (spec/README.md, Authentication).
    pub fn auth(
        mut self,
        f: impl Fn(&SecurityUse, &OperationInfo) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.inner.auth = Some(Box::new(f));
        self
    }

    pub fn on_undocumented_status(
        mut self,
        f: impl Fn(&OperationInfo, u16) + Send + Sync + 'static,
    ) -> Self {
        self.inner.on_undocumented_status = Some(Box::new(f));
        self
    }

    pub fn on_result(
        mut self,
        f: impl Fn(&OperationInfo, &'static str) + Send + Sync + 'static,
    ) -> Self {
        self.inner.on_result = Some(Box::new(f));
        self
    }

    pub fn build(self) -> Client {
        Client {
            inner: Arc::new(self.inner),
        }
    }
}

struct NoTransport;

impl Transport for NoTransport {
    fn send(&self, _: HttpRequest) -> BoxFuture<'_, Result<HttpResponse, TransportError>> {
        Box::pin(async {
            Err(TransportError::Network(
                "tsurara: no transport; enable the `reqwest` feature or set one".into(),
            ))
        })
    }
}

fn default_transport() -> Arc<dyn Transport> {
    #[cfg(feature = "reqwest")]
    {
        Arc::new(ReqwestTransport::default())
    }
    #[cfg(not(feature = "reqwest"))]
    {
        Arc::new(NoTransport)
    }
}

impl Client {
    pub fn builder(base_url: &str) -> ClientBuilder {
        let _ = NoTransport;
        ClientBuilder {
            inner: Inner {
                base_url: base_url.trim_end_matches('/').to_owned(),
                transport: default_transport(),
                headers: Vec::new(),
                timeout: None,
                retry: None,
                auth: None,
                on_undocumented_status: None,
                on_result: None,
            },
        }
    }

    /// Calls an operation and decodes the bodies: `S` for successes, `E` for documented errors.
    pub async fn request<S: DeserializeOwned, E: DeserializeOwned>(
        &self,
        def: &OperationDef,
        input: Input,
    ) -> ApiResult<S, E> {
        match self.observed(def, input, false).await {
            Ok(s) => {
                let decoded = match s.data {
                    Received::Payload(p) => {
                        serde_json::from_value::<S>(p.into_value()).map_err(|e| e.to_string())
                    }
                    Received::Items(..) => Err("the response is read item by item".to_owned()),
                };
                match decoded {
                    Ok(data) => Ok(Success {
                        status: s.status,
                        data,
                        headers: s.headers,
                        operation: s.operation,
                    }),
                    Err(cause) => Err(Failure::Parse {
                        status: s.status,
                        content_type: header(&s.headers, "content-type").map(str::to_owned),
                        cause,
                        operation: s.operation,
                    }),
                }
            }
            Err(f) => Err(decode_failure(f)),
        }
    }

    /// Calls an operation whose success is read item by item (Server-Sent Events, JSON Lines, JSON
    /// text sequences; design.md 8.5): `T` for each item, `E` for documented errors. The success
    /// returns once the headers have arrived; the client's timeout does not cover reading the items.
    pub async fn request_items<T: DeserializeOwned, E: DeserializeOwned>(
        &self,
        def: &OperationDef,
        input: Input,
    ) -> ApiResult<Items<T>, E> {
        match self.observed(def, input, true).await {
            Ok(s) => match s.data {
                Received::Items(response, kind, json_events) => Ok(Success {
                    status: s.status,
                    data: Items::new(response.body, kind, json_events),
                    headers: s.headers,
                    operation: s.operation,
                }),
                Received::Payload(_) => Err(Failure::Parse {
                    status: s.status,
                    content_type: header(&s.headers, "content-type").map(str::to_owned),
                    cause: "the response is not read item by item".into(),
                    operation: s.operation,
                }),
            },
            Err(f) => Err(decode_failure(f)),
        }
    }

    /// Calls an operation and returns the bodies as read, without decoding them. Bodies read item by
    /// item are read whole as bytes.
    pub async fn request_raw(
        &self,
        def: &OperationDef,
        input: Input,
    ) -> ApiResult<Payload, Payload> {
        self.observed(def, input, false).await.map(|s| Success {
            status: s.status,
            data: match s.data {
                Received::Payload(p) => p,
                Received::Items(..) => unreachable!("items are only kept when asked for"),
            },
            headers: s.headers,
            operation: s.operation,
        })
    }

    async fn observed(
        &self,
        def: &OperationDef,
        input: Input,
        items: bool,
    ) -> ApiResult<Received, Payload> {
        let operation = OperationInfo::from(def);
        let result = self.run(def, &operation, input, items).await;
        if let Some(f) = &self.inner.on_result {
            f(
                &operation,
                match &result {
                    Ok(_) => "success",
                    Err(e) => e.kind(),
                },
            );
        }
        result
    }

    async fn run(
        &self,
        def: &OperationDef,
        operation: &OperationInfo,
        input: Input,
        items: bool,
    ) -> ApiResult<Received, Payload> {
        let unexpected = |cause: String| Failure::Unexpected {
            cause,
            operation: operation.clone(),
        };
        if def.v != OPERATION_DEF_VERSION {
            return Err(unexpected(format!(
                "tsurara: OperationDef version {} is not supported by this tsurara-client (expected {OPERATION_DEF_VERSION}); regenerate the code or update tsurara-client",
                def.v
            )));
        }
        if let Some(e) = &input.encode_error {
            return Err(unexpected(format!(
                "tsurara: a value could not be encoded: {e}"
            )));
        }
        let request = self.prepare(def, operation, &input).map_err(unexpected)?;
        let timeout = input.timeout.or(self.inner.timeout);
        let deadline = timeout.map(|t| Instant::now() + t);
        let transport_failure = |e: TransportError| match e {
            TransportError::Timeout => Failure::Timeout {
                operation: operation.clone(),
            },
            TransportError::Network(cause) => Failure::Network {
                cause,
                operation: operation.clone(),
            },
        };
        let mut response = self
            .send(request, deadline)
            .await
            .map_err(transport_failure)?;
        if items {
            if let Some((kind, json_events)) = sequence_of(def, &response) {
                return Ok(Success {
                    status: response.status,
                    headers: response.headers.clone(),
                    operation: operation.clone(),
                    data: Received::Items(response, kind, json_events),
                });
            }
        }
        // Any other body is read whole, within the timeout.
        let read = read_all(response.body.as_mut());
        let body = match deadline {
            Some(d) => tokio::time::timeout(d.saturating_duration_since(Instant::now()), read)
                .await
                .unwrap_or(Err(TransportError::Timeout)),
            None => read.await,
        }
        .map_err(transport_failure)?;
        let response = HttpResponse {
            status: response.status,
            headers: response.headers,
            body,
        };
        let result = to_result(def, operation, response).map(|s| Success {
            status: s.status,
            data: Received::Payload(s.data),
            headers: s.headers,
            operation: s.operation,
        });
        if let (
            Err(Failure::Http {
                status,
                body: ErrorBody::Undocumented(_),
                ..
            }),
            Some(f),
        ) = (&result, &self.inner.on_undocumented_status)
        {
            f(operation, *status);
        }
        result
    }

    fn prepare(
        &self,
        def: &OperationDef,
        operation: &OperationInfo,
        input: &Input,
    ) -> Result<HttpRequest, String> {
        use crate::serialize::*;
        let params = &def.params;
        let path = expand_path(&def.path, &input.path, &params.path)?;
        let mut query = match (&params.querystring, &input.querystring) {
            (Some(qs), Some(v)) => build_query_string(v, &qs.content_type),
            _ => build_query(&input.query, &params.query),
        };
        let mut headers = self.inner.headers.clone();
        let default = crate::def::ParamDef::default();
        for (name, value) in &input.headers {
            let text = match value {
                Value::String(s) => s.clone(),
                v => serialize_header_param(v, params.headers.get(name).unwrap_or(&default)),
            };
            set(&mut headers, name, text);
        }
        let mut cookie = build_cookie(&input.cookies, &params.cookies);
        // Authentication: the first alternative with every credential.
        if let Some(auth) = &self.inner.auth {
            for alternative in &def.security {
                let schemes: Vec<&SecurityUse> = alternative
                    .iter()
                    .filter(|s| s.kind != "mutualTLS")
                    .collect();
                let values: Vec<String> =
                    schemes.iter().map_while(|s| auth(s, operation)).collect();
                if values.len() < schemes.len() {
                    continue;
                }
                for (s, v) in schemes.iter().zip(values) {
                    place(s, &v, &mut headers, &mut query, &mut cookie);
                }
                break;
            }
        }
        if !cookie.is_empty() {
            set(&mut headers, "cookie", cookie);
        }
        let mut body = Vec::new();
        if let (Some(b), Some(d)) = (&input.body, &def.body) {
            let encoded = encode_body(b, d, input.content_type.as_deref())?;
            body = encoded.content;
            if let Some(ct) = encoded.content_type {
                set(&mut headers, "content-type", ct);
            }
        }
        let mut url = format!("{}{path}", self.inner.base_url);
        if !query.is_empty() {
            url.push(if url.contains('?') { '&' } else { '?' });
            url.push_str(&query);
        }
        Ok(HttpRequest {
            method: def.method.to_ascii_uppercase(),
            url,
            headers,
            body,
        })
    }

    /// Sends, with retries when the client has them, and returns once the headers have arrived.
    async fn send(
        &self,
        request: HttpRequest,
        deadline: Option<Instant>,
    ) -> Result<StreamingResponse, TransportError> {
        let retries = self
            .inner
            .retry
            .as_ref()
            .map_or(0, |r| r.retries(&request.method));
        let mut attempt = 0;
        loop {
            let send = self.inner.transport.send_streaming(request.clone());
            let outcome = match deadline {
                Some(d) => {
                    match tokio::time::timeout(d.saturating_duration_since(Instant::now()), send)
                        .await
                    {
                        Ok(r) => r,
                        Err(_) => Err(TransportError::Timeout),
                    }
                }
                None => send.await,
            };
            let Some(retry) = &self.inner.retry else {
                return outcome;
            };
            let fits = |wait: Duration| deadline.is_none_or(|d| Instant::now() + wait < d);
            match &outcome {
                Err(TransportError::Network(_)) if attempt < retries => {
                    let wait = retry.delay(attempt + 1, None);
                    if !fits(wait) {
                        return outcome;
                    }
                    tokio::time::sleep(wait).await;
                }
                Ok(r) if attempt < retries && retry.statuses.contains(&r.status) => {
                    // `Retry-After` is in the headers; the body is not needed.
                    let head = HttpResponse {
                        status: r.status,
                        headers: r.headers.clone(),
                        body: Vec::new(),
                    };
                    let wait = retry.delay(attempt + 1, Some(&head));
                    if !fits(wait) {
                        return outcome;
                    }
                    tokio::time::sleep(wait).await;
                }
                _ => return outcome,
            }
            attempt += 1;
        }
    }
}

/// A success as the client read it: a whole body, or a body to read item by item.
enum Received {
    Payload(Payload),
    Items(
        StreamingResponse,
        crate::sequences::Kind,
        crate::sequences::JsonEvents,
    ),
}

/// The sequence a success response is read as, if its definition says so.
fn sequence_of(
    def: &OperationDef,
    response: &StreamingResponse,
) -> Option<(crate::sequences::Kind, crate::sequences::JsonEvents)> {
    let status = response.status;
    let (key, entry) = find_response(def, status)?;
    if status >= 400 || (key == "default" && has_success_entry(def)) {
        return None;
    }
    let mt = crate::serialize::media_type_of(header(&response.headers, "content-type"));
    let kind = match entry
        .content_types
        .get(&mt)
        .copied()
        .unwrap_or(entry.parse_as)
    {
        ParseAs::Events => crate::sequences::Kind::Events,
        ParseAs::JsonLines => crate::sequences::Kind::JsonLines,
        ParseAs::JsonSeq => crate::sequences::Kind::JsonSeq,
        _ => return None,
    };
    let json_events = match &entry.json_events {
        Some(Value::Bool(true)) => crate::sequences::JsonEvents::All,
        Some(Value::Array(names)) => crate::sequences::JsonEvents::Named(
            names
                .iter()
                .filter_map(|n| n.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => crate::sequences::JsonEvents::None,
    };
    Some((kind, json_events))
}

/// A failure with its documented error body decoded as `E`.
fn decode_failure<E: DeserializeOwned>(f: Failure<Payload>) -> Failure<E> {
    match f {
        Failure::Http {
            status,
            body: ErrorBody::Documented(p),
            headers,
            operation,
        } => match serde_json::from_value::<E>(p.into_value()) {
            Ok(e) => Failure::Http {
                status,
                body: ErrorBody::Documented(e),
                headers,
                operation,
            },
            Err(e) => Failure::Parse {
                status,
                content_type: header(&headers, "content-type").map(str::to_owned),
                cause: e.to_string(),
                operation,
            },
        },
        f => retype(f),
    }
}

fn set(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    let name = name.to_ascii_lowercase();
    headers.retain(|(k, _)| *k != name);
    headers.push((name, value));
}

fn place(
    s: &SecurityUse,
    value: &str,
    headers: &mut Vec<(String, String)>,
    query: &mut String,
    cookie: &mut String,
) {
    let push = |target: &mut String, part: String, sep: &str| {
        if !target.is_empty() {
            target.push_str(sep);
        }
        target.push_str(&part);
    };
    if s.kind == "apiKey" {
        let name = s.name.clone().unwrap_or_else(|| s.scheme.clone());
        match s.location.as_deref() {
            Some("query") => push(
                query,
                format!(
                    "{}={}",
                    crate::serialize::component(&name),
                    crate::serialize::component(value)
                ),
                "&",
            ),
            Some("cookie") => push(
                cookie,
                format!("{name}={}", crate::serialize::component(value)),
                "; ",
            ),
            _ => set(headers, &name, value.to_owned()),
        }
        return;
    }
    let http = if s.kind == "http" {
        s.http_scheme.as_deref().unwrap_or("bearer")
    } else {
        "bearer"
    };
    let value = match http.to_ascii_lowercase().as_str() {
        "basic" => format!("Basic {}", base64(value.as_bytes())),
        "bearer" => format!("Bearer {value}"),
        _ => format!("{http} {value}"),
    };
    set(headers, "authorization", value);
}

fn base64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn retype<A, B>(f: Failure<A>) -> Failure<B> {
    match f {
        Failure::Http {
            status,
            body: ErrorBody::Undocumented(p),
            headers,
            operation,
        } => Failure::Http {
            status,
            body: ErrorBody::Undocumented(p),
            headers,
            operation,
        },
        Failure::Http {
            status,
            headers,
            operation,
            ..
        } => Failure::Parse {
            status,
            content_type: header(&headers, "content-type").map(str::to_owned),
            cause: "a documented body was not decoded".into(),
            operation,
        },
        Failure::Network { cause, operation } => Failure::Network { cause, operation },
        Failure::Timeout { operation } => Failure::Timeout { operation },
        Failure::Parse {
            status,
            content_type,
            cause,
            operation,
        } => Failure::Parse {
            status,
            content_type,
            cause,
            operation,
        },
        Failure::Unexpected { cause, operation } => Failure::Unexpected { cause, operation },
    }
}

fn find_response(def: &OperationDef, status: u16) -> Option<(&str, &ResponseDef)> {
    [
        status.to_string(),
        format!("{}XX", status / 100),
        "default".to_owned(),
    ]
    .into_iter()
    .find_map(|k| {
        def.responses
            .get_key_value(&k)
            .map(|(k, v)| (k.as_str(), v))
    })
}

fn has_success_entry(def: &OperationDef) -> bool {
    def.responses.keys().any(|k| {
        let k = k.to_ascii_uppercase();
        if let Some(d) = k.strip_suffix("XX") {
            d.parse::<u16>().is_ok_and(|d| d < 4)
        } else {
            k.parse::<u16>().is_ok_and(|n| n < 400)
        }
    })
}

fn is_json(mt: &str) -> bool {
    mt == "application/json" || mt.ends_with("+json")
}

fn guess(response: &HttpResponse) -> ParseAs {
    let mt = crate::serialize::media_type_of(response.header("content-type"));
    if matches!(response.status, 204 | 205 | 304) {
        ParseAs::None
    } else if is_json(&mt) {
        ParseAs::Json
    } else if mt.starts_with("text/") || mt.is_empty() {
        ParseAs::Text
    } else {
        ParseAs::Blob
    }
}

fn read(response: &HttpResponse, parse_as: ParseAs) -> Result<Payload, String> {
    match parse_as {
        ParseAs::None => Ok(Payload::None),
        ParseAs::Text => Ok(Payload::Text(
            String::from_utf8_lossy(&response.body).into_owned(),
        )),
        ParseAs::Blob
        | ParseAs::Stream
        | ParseAs::Events
        | ParseAs::JsonLines
        | ParseAs::JsonSeq => Ok(Payload::Bytes {
            content: response.body.clone(),
            content_type: response.header("content-type").map(str::to_owned),
        }),
        ParseAs::Auto => read(response, guess(response)),
        ParseAs::Json => {
            let mt = crate::serialize::media_type_of(response.header("content-type"));
            if !mt.is_empty() && !is_json(&mt) {
                return Err(format!(
                    "expected JSON but the response has Content-Type {mt}"
                ));
            }
            serde_json::from_slice(&response.body)
                .map(Payload::Json)
                .map_err(|e| format!("the response body is not valid JSON: {e}"))
        }
    }
}

fn read_undocumented(response: &HttpResponse) -> Payload {
    match guess(response) {
        ParseAs::Json => serde_json::from_slice(&response.body)
            .map(Payload::Json)
            .unwrap_or_else(|_| {
                Payload::Text(String::from_utf8_lossy(&response.body).into_owned())
            }),
        other => read(response, other).unwrap_or(Payload::None),
    }
}

fn to_result(
    def: &OperationDef,
    operation: &OperationInfo,
    response: HttpResponse,
) -> ApiResult<Payload, Payload> {
    let status = response.status;
    let found = find_response(def, status);
    let content_type = response.header("content-type").map(str::to_owned);
    let mt = crate::serialize::media_type_of(content_type.as_deref());
    let parse = |entry: &ResponseDef| {
        read(
            &response,
            entry
                .content_types
                .get(&mt)
                .copied()
                .unwrap_or(entry.parse_as),
        )
    };
    let parse_failure = |cause: String| Failure::Parse {
        status,
        content_type: content_type.clone(),
        cause,
        operation: operation.clone(),
    };
    let is_default = found.is_some_and(|(k, _)| k == "default");
    let success = status < 400 && found.is_some() && (!is_default || !has_success_entry(def));
    match found {
        Some((_, entry)) if success => Ok(Success {
            status,
            data: parse(entry).map_err(parse_failure)?,
            headers: response.headers.clone(),
            operation: operation.clone(),
        }),
        Some((_, entry)) if status >= 400 => Err(Failure::Http {
            status,
            body: ErrorBody::Documented(parse(entry).map_err(parse_failure)?),
            headers: response.headers.clone(),
            operation: operation.clone(),
        }),
        _ => Err(Failure::Http {
            status,
            body: ErrorBody::Undocumented(read_undocumented(&response)),
            headers: response.headers.clone(),
            operation: operation.clone(),
        }),
    }
}
