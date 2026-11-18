"""The mock transport of `tsurara_client.testing`: routing vectors and calls through it."""

from __future__ import annotations

import json
from pathlib import Path
from typing import Any

import httpx
import pytest

from tsurara_client import AsyncClient, Client, HttpError, NetworkFailure, OperationDef, Success
from tsurara_client.testing import MockRequest, MockResponse, MockRouter

VECTORS: list[dict[str, Any]] = json.loads(
    (Path(__file__).parents[3] / "spec" / "conformance" / "mock-routing.json").read_text()
)["vectors"]


@pytest.mark.parametrize("vector", VECTORS, ids=[v["name"] for v in VECTORS])
def test_routing_vectors(vector: dict[str, Any]) -> None:
    router = MockRouter(base_path=vector["basePath"])
    for r in vector["routes"]:
        router.route(r["method"], r["path"], lambda _: MockResponse(204))
    match = router.match(vector["request"]["method"], vector["request"]["path"])
    expect = vector["expect"]
    if expect["route"] is None:
        assert match is None
    else:
        assert match is not None
        assert match.index == expect["route"]
        assert match.params == expect["params"]


GET_PET: OperationDef = {
    "v": 1,
    "id": "getPet",
    "method": "GET",
    "path": "/pets/{petId}",
    "params": {"path": {"petId": {}}},
    "responses": {"200": {"parseAs": "json"}, "404": {"parseAs": "json"}},
}

ADD_PET: OperationDef = {
    "v": 1,
    "id": "addPet",
    "method": "POST",
    "path": "/pets",
    "body": {"required": True, "contents": {"application/json": {}}},
    "responses": {"201": {"parseAs": "json"}},
}


def pet(request: MockRequest) -> MockResponse:
    if request.params["petId"] == "1":
        return MockResponse(200, json={"id": 1, "name": "Pochi"})
    return MockResponse(404, json={"message": "no pet"})


def router() -> MockRouter:
    r = MockRouter(base_path="/v1")
    r.add(GET_PET, pet)
    r.add(ADD_PET, lambda req: MockResponse(201, json={"id": 2, **json.loads(req.body)}))
    return r


def test_calls_through_the_mock_get_typed_results() -> None:
    mock = router()
    with Client("https://api.test/v1", http=httpx.Client(transport=httpx.MockTransport(mock))) as client:
        ok = client.request(GET_PET, path={"petId": 1})
        assert isinstance(ok, Success)
        assert ok.data == {"id": 1, "name": "Pochi"}
        missing = client.request(GET_PET, path={"petId": 9})
        assert isinstance(missing, HttpError)
        assert missing.status == 404 and missing.documented and missing.error == {"message": "no pet"}
        added = client.request(ADD_PET, body={"name": "Tama"})
        assert isinstance(added, Success) and added.data == {"id": 2, "name": "Tama"}
    assert [c.route["id"] if c.route else None for c in mock.calls] == ["getPet", "getPet", "addPet"]
    assert mock.calls[0].params == {"petId": "1"}


def test_the_async_client_uses_the_same_router() -> None:
    import asyncio

    async def run() -> None:
        async with AsyncClient(
            "https://api.test/v1", http=httpx.AsyncClient(transport=httpx.MockTransport(router()))
        ) as client:
            ok = await client.request(GET_PET, path={"petId": 1})
            assert isinstance(ok, Success)

    asyncio.run(run())


def test_an_unmatched_request_is_a_transport_failure() -> None:
    mock = MockRouter()
    mock.add(GET_PET, pet)
    with Client("https://api.test", http=httpx.Client(transport=httpx.MockTransport(mock))) as client:
        r = client.request(ADD_PET, body={})
        assert isinstance(r, NetworkFailure)
        assert "POST /pets" in str(r.cause)


def test_unmatched_requests_can_be_recorded() -> None:
    mock = MockRouter(unmatched="record")
    with Client("https://api.test", http=httpx.Client(transport=httpx.MockTransport(mock))) as client:
        r = client.request(GET_PET, path={"petId": 1})
        assert r.kind == "http" and r.status == 501
    assert [c.method for c in mock.unmatched] == ["GET"]


def test_a_failing_handler_is_a_transport_failure() -> None:
    def fail(_: MockRequest) -> MockResponse:
        raise RuntimeError("connection reset")

    mock = MockRouter()
    mock.route("GET", "/pets/{petId}", fail)
    with Client("https://api.test", http=httpx.Client(transport=httpx.MockTransport(mock))) as client:
        assert isinstance(client.request(GET_PET, path={"petId": 1}), NetworkFailure)
