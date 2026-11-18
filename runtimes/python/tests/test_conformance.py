"""Runs the language-neutral conformance vectors in spec/conformance (design.md 18.9), with the
sync and the async client."""

from __future__ import annotations

import asyncio
import json
from collections.abc import AsyncIterator
from pathlib import Path
from typing import Any

import httpx
import pytest

from tsurara_client import (
    AsyncClient,
    Client,
    File,
    HttpError,
    Success,
    UndocumentedStatus,
)
from tsurara_client._result import AnyResult

SPEC = Path(__file__).resolve().parents[3] / "spec" / "conformance"


def load(name: str) -> Any:
    return json.loads((SPEC / name).read_text(encoding="utf-8"))


def tagged(v: Any) -> Any:
    """Turns tagged values (`$blob`, `$file`) into runtime values."""
    if isinstance(v, list):
        return [tagged(x) for x in v]  # pyright: ignore[reportUnknownVariableType]
    if isinstance(v, dict):
        if "$blob" in v:
            b = v["$blob"]
            return File(b["text"].encode(), content_type=b["type"])
        if "$file" in v:
            f = v["$file"]
            return File(f["text"].encode(), filename=f["name"], content_type=f["type"])
        return {k: tagged(x) for k, x in v.items()}  # pyright: ignore[reportUnknownVariableType]
    return v


Call = tuple[httpx.Request, bytes]


def run(
    op: Any, input: dict[str, Any], respond: httpx.Response, use_async: bool, credentials: dict[str, str] | None = None
) -> tuple[AnyResult, list[Call]]:
    calls: list[Call] = []

    def handler(request: httpx.Request) -> httpx.Response:
        calls.append((request, request.read()))
        return respond

    args = {k: tagged(v) for k, v in input.items()}
    if "contentType" in args:
        args["content_type"] = args.pop("contentType")
    if use_async:

        async def go() -> AnyResult:
            async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
                client = AsyncClient(base_url="https://api.test", http=http, auth=lambda scheme, _: (credentials or {}).get(scheme.scheme))
                return await client.request(op, **args)

        return asyncio.run(go()), calls
    with httpx.Client(transport=httpx.MockTransport(handler)) as http:
        client = Client(base_url="https://api.test", http=http, auth=lambda scheme, _: (credentials or {}).get(scheme.scheme))
        return client.request(op, **args), calls


MODES = [pytest.param(False, id="sync"), pytest.param(True, id="async")]


def parse_multipart(request: httpx.Request, body: bytes) -> list[tuple[str, Any]]:
    from email.parser import BytesParser
    from email.policy import HTTP

    head = f"Content-Type: {request.headers['content-type']}\r\n\r\n".encode()
    message = BytesParser(policy=HTTP).parsebytes(head + body)
    parts: list[tuple[str, Any]] = []
    for part in message.iter_parts():  # pyright: ignore[reportAttributeAccessIssue, reportUnknownMemberType, reportUnknownVariableType]
        name = part.get_param("name", header="content-disposition")  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        filename = part.get_filename()  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        payload: Any = part.get_payload(decode=True)  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        ctype = part.get("content-type")  # pyright: ignore[reportUnknownMemberType, reportUnknownVariableType]
        if ctype is None and filename is None:
            parts.append((str(name), payload.decode()))  # pyright: ignore[reportUnknownArgumentType]
        else:
            parts.append((str(name), (filename, part.get_content_type(), payload.decode())))  # pyright: ignore[reportUnknownMemberType, reportUnknownArgumentType]
    return parts


@pytest.mark.parametrize("use_async", MODES)
@pytest.mark.parametrize("vector", load("requests.json"), ids=lambda v: v["name"])
def test_request_vectors(vector: dict[str, Any], use_async: bool) -> None:
    result, calls = run(vector["operation"], vector["input"], httpx.Response(204), use_async, vector.get("credentials"))
    expect = vector["expect"]
    if "kind" in expect:
        assert result.kind == expect["kind"]
        assert calls == []
        return
    assert len(calls) == 1, result
    request, body = calls[0]
    assert str(request.url) == expect["url"]
    assert request.method == expect["method"]
    for name, want in (expect.get("headers") or {}).items():
        assert request.headers.get(name) == want, name
    want = expect["body"]
    if want is None:
        assert body == b""
    elif "text" in want:
        assert body.decode() == want["text"]
    elif "bytes" in want:
        assert body.decode() == want["bytes"]["text"]
        assert request.headers.get("content-type") == expect["headers"]["content-type"]
    else:
        got = parse_multipart(request, body)
        assert len(got) == len(want["form"])
        for (name, value), (want_name, want_value) in zip(got, want["form"]):
            assert name == want_name
            if isinstance(want_value, dict) and ("$blob" in want_value or "$file" in want_value):
                assert isinstance(value, tuple)
                filename, ctype, text = value  # pyright: ignore[reportUnknownVariableType]
                if "$file" in want_value:
                    f = want_value["$file"]
                    assert (filename, ctype, text) == (f["name"], f["type"], f["text"])
                else:
                    b = want_value["$blob"]
                    assert (ctype, text) == (b["type"], b["text"])
            else:
                assert value == want_value


def neutral_body(value: Any, response: httpx.Response) -> Any:
    if isinstance(value, bytes):
        return {"$blob": {"type": response.headers.get("content-type"), "text": value.decode()}}
    return value


@pytest.mark.parametrize("use_async", MODES)
@pytest.mark.parametrize("vector", load("responses.json"), ids=lambda v: v["name"])
def test_response_vectors(vector: dict[str, Any], use_async: bool) -> None:
    r = vector["response"]
    body = r["body"]["text"].encode() if r["body"] else b""
    response = httpx.Response(r["status"], headers=r["headers"], content=body)
    result, _ = run(vector["operation"], {}, response, use_async)
    expect = vector["expect"]
    assert result.kind == expect["kind"], result
    if isinstance(result, Success):
        assert result.status == expect["status"]
        assert neutral_body(result.data, result.response) == expect["data"]
    elif isinstance(result, HttpError | UndocumentedStatus):
        assert result.status == expect["status"]
        assert result.documented == expect["documented"]
        assert neutral_body(result.error, result.response) == expect["error"]


# Sequence vectors: the body arrives in chunks, and the items are read inside the client's lifetime.


def read_items(items: Any) -> tuple[list[Any], bool]:
    out: list[Any] = []
    try:
        for item in items:
            out.append(item)
    except Exception:
        return out, True
    return out, False


async def read_items_async(items: Any) -> tuple[list[Any], bool]:
    out: list[Any] = []
    try:
        async for item in items:
            out.append(item)
    except Exception:
        return out, True
    return out, False


def check_sequence(vector: dict[str, Any], result: AnyResult, items: tuple[list[Any], bool] | None) -> None:
    expect = vector["expect"]
    assert result.kind == expect["kind"], result
    if isinstance(result, Success):
        assert result.status == expect["status"]
        assert items == (expect["items"], expect.get("error", False))
    elif isinstance(result, HttpError | UndocumentedStatus):
        assert result.status == expect["status"]
        assert result.documented == expect["documented"]
        assert result.error == expect["error"]


@pytest.mark.parametrize("use_async", MODES)
@pytest.mark.parametrize("vector", load("sequences.json"), ids=lambda v: v["name"])
def test_sequence_vectors(vector: dict[str, Any], use_async: bool) -> None:
    r = vector["response"]
    chunks = [c.encode() for c in r["chunks"]]

    if use_async:

        async def body() -> AsyncIterator[bytes]:
            for c in chunks:
                yield c

        async def go() -> None:
            def handler(_: httpx.Request) -> httpx.Response:
                return httpx.Response(r["status"], headers=r["headers"], content=body())

            async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
                result = await AsyncClient(base_url="https://api.test", http=http).request(vector["operation"])
                items = await read_items_async(result.data) if isinstance(result, Success) else None
                check_sequence(vector, result, items)

        asyncio.run(go())
        return

    def handler(_: httpx.Request) -> httpx.Response:
        return httpx.Response(r["status"], headers=r["headers"], content=iter(chunks))

    with httpx.Client(transport=httpx.MockTransport(handler)) as http:
        result = Client(base_url="https://api.test", http=http).request(vector["operation"])
        items = read_items(result.data) if isinstance(result, Success) else None
        check_sequence(vector, result, items)
