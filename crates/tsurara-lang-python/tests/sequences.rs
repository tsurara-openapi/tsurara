//! Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences
//! are iterators of typed items with `Client`, and async iterators with `AsyncClient`. Needs the
//! runtime's virtual environment; skipped without it unless the Python checker is required.

use std::path::Path;
use std::process::Command;
use tsurara_lang_python::{PythonOptions, generate};
use tsurara_loader::{DocumentSet, parse_document};
use tsurara_sem::Options;
use tsurara_testkit::{Checker, PythonChecker, python_or_skip, repo_root};

fn set() -> DocumentSet {
    let text = std::fs::read(repo_root().join("corpus/edge-cases/event-streams.yaml")).unwrap();
    DocumentSet::from_value(parse_document(&text).unwrap())
}

fn files() -> Vec<(String, String)> {
    let out = generate(&set(), &Options::default(), &PythonOptions::default()).unwrap();
    out.files
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
fn sequences_are_iterators_of_items() {
    let files = files();
    let stream = file(&files, "operations/stream.py");
    assert!(
        // Events the definition does not list arrive as dicts.
        stream.contains(
            "StreamResult: TypeAlias = \"Success[Iterator[StreamEvent | dict[str, Any]]] | "
        ),
        "{stream}"
    );
    assert!(
        stream.contains(
            "StreamAsyncResult: TypeAlias = \"Success[AsyncIterator[StreamEvent | dict[str, Any]]] | "
        ),
        "{stream}"
    );
    assert!(
        stream.contains("-> Awaitable[StreamAsyncResult]: ..."),
        "{stream}"
    );
    assert!(stream.contains("\"parseAs\": \"events\""), "{stream}");
    assert!(
        stream.contains("\"jsonEvents\": [\n                \"note\",\n                \"noteDeleted\",\n            ],"),
        "{stream}"
    );
    assert!(
        stream.contains("\"text/event-stream\": {\n                    \"ref\": \"StreamEvent\","),
        "{stream}"
    );
    let lines = file(&files, "operations/lines.py");
    assert!(lines.contains("Success[Iterator[Note]]"), "{lines}");
    assert!(lines.contains("\"parseAs\": \"jsonLines\""), "{lines}");
    let seq = file(&files, "operations/seq.py");
    assert!(seq.contains("\"parseAs\": \"jsonSeq\""), "{seq}");
}

const USE: &str = r#"
from __future__ import annotations

from typing import assert_type

from tsurara_client import AsyncClient, Client, Success

from . import Note, StreamNoteEvent
from .operations import lines, stream


def read(client: Client[None]) -> list[str]:
    out: list[str] = []
    r = stream(client)
    if isinstance(r, Success):
        for e in r.data:
            if isinstance(e, StreamNoteEvent):
                assert_type(e.data, Note)
                out.append(e.data.id)
    r2 = lines(client)
    if isinstance(r2, Success):
        out.extend(n.id for n in r2.data)
    return out


async def read_async(client: AsyncClient[None]) -> list[str]:
    out: list[str] = []
    r = await lines(client)
    if isinstance(r, Success):
        async for n in r.data:
            out.append(n.id)
    return out
"#;

#[test]
fn generated_sequences_pass_pyright() {
    let mut files = files();
    files.push(("_use.py".to_owned(), USE.to_owned()));
    PythonChecker {
        package: "api".into(),
    }
    .check(&files)
    .unwrap_or_else(|e| panic!("{e}"));
}

const CALLS: &str = r#"
import asyncio, json
import httpx
from tsurara_client import AsyncClient, Client, Success
from api import Note, StreamNoteEvent, StreamNoteDeletedEvent
from api.operations import lines, mixed, seq, stream

def handler(request):
    path = request.url.path
    if path == "/stream":
        body = b'event: note\ndata: {"id":"1","text":"hi"}\nid: 5\n\nevent: noteDeleted\ndata: {"id":"1"}\n\n'
        return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=iter([body[:20], body[20:]]))
    if path == "/mixed":
        body = b'event: note\ndata: {"id":"2"}\n\nevent: ping\ndata: x\n\n'
        return httpx.Response(200, headers={"content-type": "text/event-stream"}, content=iter([body]))
    if path == "/lines":
        return httpx.Response(200, headers={"content-type": "application/x-ndjson"}, content=b'{"id":"3"}\n{"id":"4"}\n')
    return httpx.Response(200, headers={"content-type": "application/json-seq"}, content=b'\x1e{"count":1}\n')

out = []
with httpx.Client(transport=httpx.MockTransport(handler)) as http:
    client = Client("https://api.test", http=http)
    r = stream(client)
    assert isinstance(r, Success), r
    for e in r.data:
        out.append([type(e).__name__, e.event, type(e.data).__name__, e.data.id, e.id])
    r = mixed(client)
    for e in r.data:
        out.append([type(e).__name__, getattr(e.event, "value", e.event), e.data if isinstance(e.data, str) else e.data.id])
    out.append([n.id for n in lines(client).data])
    out.append([type(s).__name__ + ":" + str(s.count) for s in seq(client).data])

async def go():
    async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
        r = await lines(AsyncClient("https://api.test", http=http))
        return [n.id async for n in r.data]

out.append(asyncio.run(go()))
print(json.dumps(out))
"#;

fn run(python: &Path, dir: &Path, script: &str) -> String {
    std::fs::write(dir.join("script.py"), script).unwrap();
    let out = Command::new(python)
        .arg("script.py")
        .current_dir(dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn generated_functions_read_typed_items() {
    let Some(python) = python_or_skip() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    for (path, contents) in files() {
        let full = dir.path().join("api").join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(full, contents).unwrap();
    }
    let out: serde_json::Value = serde_json::from_str(&run(&python, dir.path(), CALLS)).unwrap();
    assert_eq!(
        out,
        serde_json::json!([
            ["StreamNoteEvent", "note", "Note", "1", "5"],
            [
                "StreamNoteDeletedEvent",
                "noteDeleted",
                "StreamNoteDeletedEventData",
                "1",
                null
            ],
            ["MixedNoteEvent", "note", "2"],
            ["MixedPingEvent", "ping", "x"],
            ["3", "4"],
            ["SeqItem:1"],
            ["3", "4"]
        ])
    );
}
