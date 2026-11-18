"""Responses read item by item, beyond the conformance vectors: typed items, closing early, and
characters split across chunks."""

from __future__ import annotations

import asyncio
from collections.abc import AsyncIterator, Iterator
from dataclasses import dataclass
from typing import Any

import httpx

from tsurara_client import AsyncClient, Client, OperationDef, Success, TypeDefs

LINES: OperationDef = {
    "v": 1,
    "id": "lines",
    "method": "GET",
    "path": "/lines",
    "responses": {"200": {"parseAs": "jsonLines", "types": {"application/jsonl": {"ref": "Item"}}}},
}


@dataclass(kw_only=True)
class Item:
    id: int


TYPE_DEFS = TypeDefs(
    {
        "v": 1,
        "types": {
            "Item": {
                "kind": "record",
                "fields": [{"wire": "id", "member": "id", "type": {"scalar": "integer"}, "presence": "required"}],
            }
        },
    },
    classes={"Item": Item},
)


class Body(httpx.SyncByteStream):
    """A body that records whether it was closed."""

    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks
        self.closed = False

    def __iter__(self) -> Iterator[bytes]:
        yield from self.chunks

    def close(self) -> None:
        self.closed = True


class AsyncBody(httpx.AsyncByteStream):
    def __init__(self, chunks: list[bytes]) -> None:
        self.chunks = chunks
        self.closed = False

    async def __aiter__(self) -> AsyncIterator[bytes]:
        for c in self.chunks:
            yield c

    async def aclose(self) -> None:
        self.closed = True


def respond(body: Any) -> httpx.MockTransport:
    return httpx.MockTransport(
        lambda _: httpx.Response(200, headers={"content-type": "application/jsonl"}, stream=body)
    )


def test_items_are_decoded_with_their_type_def() -> None:
    with httpx.Client(transport=respond(Body([b'{"id":1}\n{"id"', b":2}\n"]))) as http:
        r = Client("https://api.test", http=http).request(LINES, type_defs=TYPE_DEFS)
        assert isinstance(r, Success)
        assert list(r.data) == [Item(id=1), Item(id=2)]


def test_closing_the_iterator_early_closes_the_response() -> None:
    body = Body([b"1\n", b"2\n", b"3\n"])
    with httpx.Client(transport=respond(body)) as http:
        r = Client("https://api.test", http=http).request(LINES)
        assert isinstance(r, Success)
        for item in r.data:
            assert item == 1
            break
        r.data.close()
        assert body.closed


def test_closing_the_async_iterator_early_closes_the_response() -> None:
    body = AsyncBody([b"1\n", b"2\n"])

    async def go() -> None:
        async with httpx.AsyncClient(transport=respond(body)) as http:
            r = await AsyncClient("https://api.test", http=http).request(LINES)
            assert isinstance(r, Success)
            async for item in r.data:
                assert item == 1
                break
            await r.data.aclose()
            assert body.closed

    asyncio.run(go())


def test_characters_split_across_chunks_and_a_byte_order_mark() -> None:
    raw = '﻿"日本語"\n'.encode()
    with httpx.Client(transport=respond(Body([raw[:5], raw[5:]]))) as http:
        r = Client("https://api.test", http=http).request(LINES)
        assert isinstance(r, Success)
        assert list(r.data) == ["日本語"]
