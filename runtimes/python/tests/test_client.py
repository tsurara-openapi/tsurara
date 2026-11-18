"""Client behavior beyond the conformance vectors: hooks, failures, unwrap, typed values."""

from __future__ import annotations

import asyncio
import enum
from dataclasses import dataclass
from typing import Any

import httpx
import pytest

from tsurara_client import (
    MISSING,
    ApiError,
    AsyncClient,
    Client,
    HttpError,
    Missing,
    NetworkFailure,
    OperationDef,
    RequestInfo,
    Success,
    TimeoutFailure,
    TypeDefs,
    UndocumentedStatus,
    UndocumentedStatusInfo,
    UnexpectedFailure,
    unwrap,
)
from tsurara_client._result import AnyResult

GET_PET: OperationDef = {
    "v": 1,
    "id": "getPet",
    "method": "GET",
    "path": "/pets/{petId}",
    "tags": ["pets"],
    "params": {"path": {"petId": {}}},
    "responses": {
        "200": {"parseAs": "json", "types": {"application/json": {"ref": "Pet"}}},
        "404": {"parseAs": "json"},
    },
}


class Status(enum.Enum):
    AVAILABLE = "available"
    SOLD = "sold"


@dataclass(kw_only=True)
class Pet:
    id: int
    pet_name: str | None = None
    status: Status | str | None = None
    note: str | None | Missing = MISSING


TYPE_DEFS = TypeDefs(
    {
        "v": 1,
        "types": {
            "Pet": {
                "kind": "record",
                "fields": [
                    {"wire": "id", "member": "id", "type": {"scalar": "integer"}, "presence": "required"},
                    {"wire": "petName", "member": "pet_name", "type": {"scalar": "string"}, "presence": "optional"},
                    {"wire": "status", "member": "status", "type": {"ref": "Status"}, "presence": "optional"},
                    {"wire": "note", "member": "note", "type": {"scalar": "string"}, "presence": "optionalNullable"},
                ],
            },
            "Status": {"kind": "enum", "values": ["available", "sold"], "open": True},
        },
    },
    classes={"Pet": Pet, "Status": Status},
)


def client(handler: Any, **options: Any) -> Client[Any]:
    return Client("https://api.test", http=httpx.Client(transport=httpx.MockTransport(handler)), **options)


def json_response(status: int, body: Any) -> httpx.Response:
    return httpx.Response(status, json=body)


def test_typed_values_are_decoded_into_classes() -> None:
    api = client(lambda _: json_response(200, {"id": 1, "petName": "Rex", "status": "sold", "extra": 1}))
    r = api.request(GET_PET, path={"petId": 1}, type_defs=TYPE_DEFS)
    assert isinstance(r, Success)
    assert r.data == Pet(id=1, pet_name="Rex", status=Status.SOLD)
    assert r.data.note is MISSING
    # Unlisted values of an open enum stay strings.
    api = client(lambda _: json_response(200, {"id": 1, "status": "lost"}))
    r = api.request(GET_PET, path={"petId": 1}, type_defs=TYPE_DEFS)
    assert isinstance(r, Success) and r.data.status == "lost"


def test_values_that_do_not_match_the_type_are_parse_failures() -> None:
    api = client(lambda _: json_response(200, {"petName": "Rex"}))
    r = api.request(GET_PET, path={"petId": 1}, type_defs=TYPE_DEFS)
    assert r.kind == "parse"


def test_typed_bodies_are_encoded() -> None:
    sent: list[bytes] = []

    def handler(request: httpx.Request) -> httpx.Response:
        sent.append(request.read())
        return httpx.Response(204)

    op: OperationDef = {
        "v": 1,
        "id": "addPet",
        "method": "POST",
        "path": "/pets",
        "body": {"contents": {"application/json": {"type": {"ref": "Pet"}}}},
        "responses": {"204": {"parseAs": "none"}},
    }
    client(handler).request(op, body=Pet(id=1, pet_name="Rex", status=Status.SOLD, note=None), type_defs=TYPE_DEFS)
    assert sent == [b'{"id":1,"petName":"Rex","status":"sold","note":null}']


def test_documented_and_undocumented_errors() -> None:
    r = client(lambda _: json_response(404, {"m": "no"})).request(GET_PET, path={"petId": 1})
    assert isinstance(r, HttpError) and r.error == {"m": "no"} and r.documented
    r = client(lambda _: json_response(201, {})).request(GET_PET, path={"petId": 1})
    assert isinstance(r, UndocumentedStatus) and r.status == 201 and not r.documented


def test_hooks_observe_results_and_undocumented_statuses() -> None:
    results: list[tuple[str, Any]] = []
    undocumented: list[UndocumentedStatusInfo[str]] = []

    def on_result(result: AnyResult, info: RequestInfo[str]) -> None:
        results.append((result.kind, info.context))
        raise RuntimeError("ignored")

    def on_undocumented(info: UndocumentedStatusInfo[str]) -> None:
        undocumented.append(info)
        raise RuntimeError("ignored")

    status = 201
    api: Client[str] = client(
        lambda _: json_response(status, {}), on_result=on_result, on_undocumented_status=on_undocumented
    )
    r = api.request(GET_PET, path={"petId": 1}, context="trace")
    assert r.kind == "http"
    status = 200
    api.request(GET_PET, path={"petId": 1})
    assert results == [("http", "trace"), ("success", None)]
    assert [(i.operation.id, i.status, i.context) for i in undocumented] == [("getPet", 201, "trace")]


def test_transport_failures() -> None:
    def refuse(request: httpx.Request) -> httpx.Response:
        raise httpx.ConnectError("refused", request=request)

    def slow(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("slow", request=request)

    assert isinstance(client(refuse).request(GET_PET, path={"petId": 1}), NetworkFailure)
    assert isinstance(client(slow).request(GET_PET, path={"petId": 1}), TimeoutFailure)


def test_header_functions_and_version_mismatch() -> None:
    seen: list[str | None] = []

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request.headers.get("authorization"))
        return json_response(200, {"id": 1})

    api: Client[str] = client(handler, headers=lambda info: {"Authorization": f"Bearer {info.context}"})
    api.request(GET_PET, path={"petId": 1}, context="t")
    assert seen == ["Bearer t"]
    future: Any = {**GET_PET, "v": 2}
    r = api.request(future, path={"petId": 1})
    assert isinstance(r, UnexpectedFailure) and "version 2" in str(r.cause)
    assert seen == ["Bearer t"]


def test_unwrap() -> None:
    ok = client(lambda _: json_response(200, {"id": 1})).request(GET_PET, path={"petId": 1})
    assert unwrap(ok) == {"id": 1}
    failed = client(lambda _: json_response(404, {"m": "no"})).request(GET_PET, path={"petId": 1})
    with pytest.raises(ApiError) as e:
        unwrap(failed)
    assert e.value.status == 404
    assert e.value.operation_id == "getPet"
    assert "getPet failed with HTTP 404" in str(e.value)
    # ApiError made directly also names the operation.
    assert ApiError(failed).operation_id == "getPet"


def test_unwrap_rejects_coroutines() -> None:
    async def main() -> None:
        async with httpx.AsyncClient(transport=httpx.MockTransport(lambda _: json_response(200, {}))) as http:
            api: AsyncClient[Any] = AsyncClient("https://api.test", http=http)
            pending = api.request(GET_PET, path={"petId": 1})
            with pytest.raises(TypeError, match="await the call first"):
                unwrap(pending)  # pyright: ignore[reportArgumentType]
            await pending

    asyncio.run(main())


def test_async_hooks_and_headers_may_be_coroutines() -> None:
    seen: list[str] = []

    async def headers(_: RequestInfo[Any]) -> dict[str, str]:
        return {"x-token": "async"}

    async def on_result(result: AnyResult, _: RequestInfo[Any]) -> None:
        seen.append(result.kind)

    def handler(request: httpx.Request) -> httpx.Response:
        seen.append(request.headers["x-token"])
        return json_response(200, {"id": 1})

    async def main() -> None:
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
            api: AsyncClient[Any] = AsyncClient("https://api.test", http=http, headers=headers, on_result=on_result)
            await api.request(GET_PET, path={"petId": 1})

    asyncio.run(main())
    assert seen == ["async", "success"]
