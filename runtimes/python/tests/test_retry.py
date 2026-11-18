"""Retry (design.md 8.4): the same defaults as @tsurara/client, for Client and AsyncClient."""

from __future__ import annotations

import asyncio
from email.utils import format_datetime
from datetime import datetime, timedelta, timezone
from typing import Any

import httpx

from tsurara_client import AsyncClient, Client, OperationDef, Retry

GET: OperationDef = {"v": 1, "id": "get", "method": "GET", "path": "/x", "responses": {"200": {"parseAs": "json"}}}
POST: OperationDef = {**GET, "id": "post", "method": "POST"}


def run(statuses: list[tuple[int, dict[str, str]]], retry: Retry, op: OperationDef = GET, **options: Any) -> tuple[Any, int]:
    calls = 0

    def handler(_: httpx.Request) -> httpx.Response:
        nonlocal calls
        calls += 1
        status, headers = statuses[min(calls - 1, len(statuses) - 1)]
        return httpx.Response(status, headers=headers, json={})

    client = Client("https://x", http=httpx.Client(transport=httpx.MockTransport(handler)), retry=retry, **options)
    return client.request(op), calls


def recording() -> tuple[list[float], Retry]:
    waits: list[float] = []
    return waits, Retry(sleep=waits.append, random=lambda: 0.5)


def test_retries_until_success_with_jittered_backoff() -> None:
    waits, retry = recording()
    r, calls = run([(503, {}), (502, {}), (200, {})], retry)
    assert r.kind == "success" and calls == 3
    # Full jitter: half of 0.1 s, then half of 0.2 s.
    assert waits == [0.05, 0.1]


def test_returns_the_last_response_after_max_retries() -> None:
    _, retry = recording()
    r, calls = run([(503, {})], retry)
    assert r.kind == "http" and r.status == 503 and calls == 3


def test_retry_after_seconds_and_dates_are_honored_and_capped() -> None:
    waits, retry = recording()
    run([(429, {"retry-after": "2"}), (200, {})], retry)
    assert waits == [2.0]
    waits.clear()
    at = format_datetime(datetime.now(timezone.utc) + timedelta(seconds=3), usegmt=True)
    run([(503, {"retry-after": at}), (200, {})], retry)
    assert len(waits) == 1 and 1.0 < waits[0] <= 3.0
    waits.clear()
    capped = Retry(sleep=waits.append, max_delay=5.0)
    run([(429, {"retry-after": "120"}), (200, {})], capped)
    assert waits == [5.0]


def test_non_idempotent_methods_are_not_retried() -> None:
    _, retry = recording()
    r, calls = run([(503, {})], retry, op=POST)
    assert r.kind == "http" and calls == 1


def test_a_wait_beyond_the_timeout_stops_retrying() -> None:
    waits, retry = recording()
    r, calls = run([(429, {"retry-after": "10"})], retry, timeout=1.0)
    assert r.kind == "http" and r.status == 429 and calls == 1 and waits == []


def test_network_failures_are_retried_and_timeouts_are_not() -> None:
    waits, retry = recording()
    calls = 0

    def flaky(request: httpx.Request) -> httpx.Response:
        nonlocal calls
        calls += 1
        if calls == 1:
            raise httpx.ConnectError("refused", request=request)
        return httpx.Response(200, json={})

    client = Client("https://x", http=httpx.Client(transport=httpx.MockTransport(flaky)), retry=retry)
    assert client.request(GET).kind == "success" and calls == 2

    def slow(request: httpx.Request) -> httpx.Response:
        raise httpx.ReadTimeout("slow", request=request)

    client = Client("https://x", http=httpx.Client(transport=httpx.MockTransport(slow)), retry=retry)
    assert client.request(GET).kind == "timeout"


def test_async_client_retries_too() -> None:
    waits: list[float] = []

    async def sleep(seconds: float) -> None:
        waits.append(seconds)

    calls = 0

    def handler(_: httpx.Request) -> httpx.Response:
        nonlocal calls
        calls += 1
        return httpx.Response(503 if calls == 1 else 200, json={})

    async def main() -> str:
        async with httpx.AsyncClient(transport=httpx.MockTransport(handler)) as http:
            api: AsyncClient[Any] = AsyncClient("https://x", http=http, retry=Retry(async_sleep=sleep, random=lambda: 0))
            return (await api.request(GET)).kind

    assert asyncio.run(main()) == "success"
    assert calls == 2 and waits == [0.0]
