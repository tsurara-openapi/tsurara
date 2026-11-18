//! Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences (design.md
//! 8.5). [`Items`] reads the body a chunk at a time and decodes each complete item.

use crate::{BodyReader, TransportError};
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};
use std::collections::VecDeque;
use std::fmt;
use std::marker::PhantomData;

/// How the items of a body are separated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    Events,
    JsonLines,
    JsonSeq,
}

/// Which events carry JSON in `data`.
#[derive(Debug, Clone, Default)]
pub(crate) enum JsonEvents {
    #[default]
    None,
    All,
    Named(Vec<String>),
}

/// Why reading an item failed. The response itself was a success, so this comes from
/// [`Items::next`] instead of the request.
#[derive(Debug)]
pub enum ItemError {
    /// The connection broke while reading.
    Transport(TransportError),
    /// The item is not valid JSON, or does not match its type.
    Parse(String),
}

impl fmt::Display for ItemError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ItemError::Transport(e) => write!(f, "tsurara: reading the stream failed: {e}"),
            ItemError::Parse(e) => write!(f, "tsurara: {e}"),
        }
    }
}

impl std::error::Error for ItemError {}

/// The items of a response, read as they arrive: events (`{ "event", "data", "id"?, "retry"? }`
/// with JSON data parsed) or JSON values, each decoded as `T`.
///
/// Reading stops at the first error. Dropping it closes the response.
pub struct Items<T> {
    body: Box<dyn BodyReader>,
    parser: Parser,
    pending: VecDeque<Result<Value, String>>,
    done: bool,
    _item: PhantomData<fn() -> T>,
}

impl<T> fmt::Debug for Items<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Items")
            .field("kind", &self.parser.kind)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl<T: DeserializeOwned> Items<T> {
    pub(crate) fn new(body: Box<dyn BodyReader>, kind: Kind, json_events: JsonEvents) -> Self {
        Items {
            body,
            parser: Parser::new(kind, json_events),
            pending: VecDeque::new(),
            done: false,
            _item: PhantomData,
        }
    }

    /// The next item, or `None` at the end of the body or after an error.
    pub async fn next(&mut self) -> Option<Result<T, ItemError>> {
        loop {
            if let Some(item) = self.pending.pop_front() {
                let decoded = item.and_then(|v| {
                    serde_json::from_value::<T>(v)
                        .map_err(|e| format!("an item does not match its type: {e}"))
                });
                if decoded.is_err() {
                    self.pending.clear();
                    self.done = true;
                }
                return Some(decoded.map_err(ItemError::Parse));
            }
            if self.done {
                return None;
            }
            match self.body.next_chunk().await {
                Ok(Some(chunk)) => self.parser.feed(&chunk, false, &mut self.pending),
                Ok(None) => {
                    self.parser.feed(&[], true, &mut self.pending);
                    self.done = true;
                }
                Err(e) => {
                    self.done = true;
                    return Some(Err(ItemError::Transport(e)));
                }
            }
        }
    }
}

/// Splits bytes into items. Lines and records are split on bytes (CR, LF and U+001E never occur
/// inside a multi-byte UTF-8 character), then decoded.
struct Parser {
    kind: Kind,
    json_events: JsonEvents,
    buffer: Vec<u8>,
    /// Whether the start of the body was seen (to drop a byte order mark) and, for JSON text
    /// sequences, the first record separator.
    started: bool,
    bom_checked: bool,
    failed: bool,
    count: usize,
    event: String,
    data: Vec<String>,
    id: Option<String>,
    retry: Option<u64>,
}

impl Parser {
    fn new(kind: Kind, json_events: JsonEvents) -> Self {
        Parser {
            kind,
            json_events,
            buffer: Vec::new(),
            started: kind != Kind::JsonSeq,
            bom_checked: false,
            failed: false,
            count: 0,
            event: String::new(),
            data: Vec::new(),
            id: None,
            retry: None,
        }
    }

    fn feed(&mut self, chunk: &[u8], last: bool, out: &mut VecDeque<Result<Value, String>>) {
        if self.failed {
            return;
        }
        self.buffer.extend_from_slice(chunk);
        if !self.bom_checked {
            if self.buffer.len() < 3 && !last && b"\xEF\xBB\xBF".starts_with(&self.buffer) {
                return;
            }
            if self.buffer.starts_with(b"\xEF\xBB\xBF") {
                self.buffer.drain(..3);
            }
            self.bom_checked = true;
        }
        let mut parts: Vec<Vec<u8>> = Vec::new();
        let mut start = 0;
        let mut i = 0;
        while i < self.buffer.len() {
            let b = self.buffer[i];
            let end = match self.kind {
                Kind::JsonSeq => (b == 0x1E).then_some(i + 1),
                _ if b == b'\n' => Some(i + 1),
                // A CR at the end may be the first half of a CRLF, so it waits for what follows.
                _ if b == b'\r' && i + 1 == self.buffer.len() && !last => break,
                _ if b == b'\r' => Some(if self.buffer.get(i + 1) == Some(&b'\n') {
                    i + 2
                } else {
                    i + 1
                }),
                _ => None,
            };
            if let Some(end) = end {
                parts.push(self.buffer[start..i].to_vec());
                start = end;
                i = end;
            } else {
                i += 1;
            }
        }
        self.buffer.drain(..start);
        if last {
            parts.push(std::mem::take(&mut self.buffer));
        }
        if !self.started {
            // Text before the first record separator is not a record.
            let separated = if last {
                parts.len() > 1
            } else {
                !parts.is_empty()
            };
            if !separated {
                return;
            }
            parts.remove(0);
            self.started = true;
        }
        if self.kind == Kind::Events {
            // The last part of the body is an event the stream ended in the middle of.
            if last {
                parts.pop();
            }
            for line in parts {
                if let Some(event) = self.line(&String::from_utf8_lossy(&line)) {
                    let failed = event.is_err();
                    out.push_back(event);
                    if failed {
                        self.failed = true;
                        return;
                    }
                }
            }
            return;
        }
        for part in parts {
            self.count += 1;
            let text = String::from_utf8_lossy(&part);
            if text.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Value>(&text) {
                Ok(v) => out.push_back(Ok(v)),
                Err(e) => {
                    let what = if self.kind == Kind::JsonLines {
                        "line"
                    } else {
                        "record"
                    };
                    out.push_back(Err(format!("{what} {} is not valid JSON: {e}", self.count)));
                    self.failed = true;
                    return;
                }
            }
        }
    }

    fn line(&mut self, line: &str) -> Option<Result<Value, String>> {
        if line.is_empty() {
            let event = (!self.data.is_empty()).then(|| {
                let name = if self.event.is_empty() {
                    "message".to_owned()
                } else {
                    std::mem::take(&mut self.event)
                };
                let text = self.data.join("\n");
                let json = match &self.json_events {
                    JsonEvents::None => false,
                    JsonEvents::All => true,
                    JsonEvents::Named(names) => names.contains(&name),
                };
                let data = if json {
                    serde_json::from_str(&text).map_err(|e| {
                        format!("the data of the event \"{name}\" is not valid JSON: {e}")
                    })?
                } else {
                    Value::String(text)
                };
                let mut m = Map::new();
                m.insert("event".into(), Value::String(name));
                m.insert("data".into(), data);
                if let Some(id) = self.id.take() {
                    m.insert("id".into(), Value::String(id));
                }
                if let Some(retry) = self.retry.take() {
                    m.insert("retry".into(), Value::from(retry));
                }
                Ok(Value::Object(m))
            });
            self.event.clear();
            self.data.clear();
            self.id = None;
            self.retry = None;
            return event;
        }
        if line.starts_with(':') {
            return None;
        }
        let (field, value) = line.split_once(':').unwrap_or((line, ""));
        let value = value.strip_prefix(' ').unwrap_or(value);
        match field {
            "event" => self.event = value.to_owned(),
            "data" => self.data.push(value.to_owned()),
            "id" if !value.contains('\0') => self.id = Some(value.to_owned()),
            "retry" if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) => {
                self.retry = value.parse().ok();
            }
            _ => {}
        }
        None
    }
}
