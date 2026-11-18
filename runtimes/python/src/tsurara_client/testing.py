"""A mock transport for tests: an httpx handler that answers from handlers routed by
operation, for ``httpx.MockTransport`` with ``Client`` and ``AsyncClient``. Routing follows
spec/README.md (Mock transports), shared with the runtimes of the other languages.

.. code-block:: python

    router = MockRouter(base_path="/v1")
    router.add(GET_PET, lambda req: MockResponse(200, json={"id": req.params["petId"]}))
    client = Client("https://api.test/v1", http=httpx.Client(transport=httpx.MockTransport(router)))
"""

from __future__ import annotations

import json as jsonlib
import re
from collections.abc import Callable, Iterable, Mapping
from dataclasses import dataclass, field
from typing import Any, Literal
from urllib.parse import unquote

import httpx

from ._def import OperationDef

__all__ = ["MockCall", "MockMatch", "MockRequest", "MockResponse", "MockRoute", "MockRouter"]


@dataclass(frozen=True)
class MockRequest:
    """What a handler receives."""

    method: str
    url: httpx.URL
    params: Mapping[str, str]
    """Path parameters, percent-decoded."""
    query: list[tuple[str, str]]
    headers: httpx.Headers
    body: bytes


@dataclass(frozen=True)
class MockResponse:
    """What a handler returns: a status with a JSON value, text or bytes."""

    status: int
    json: Any = None
    """Encoded as JSON with ``Content-Type: application/json`` when not ``None``."""
    text: str | None = None
    content: bytes | None = None
    headers: Mapping[str, str] = field(default_factory=lambda: {})
    content_type: str | None = None


Handler = Callable[[MockRequest], MockResponse]

MockRoute = tuple[OperationDef, Handler]
"""An OperationDef with its handler, as generated mock routes return them (``router.extend(routes())``)."""


@dataclass(frozen=True)
class MockCall:
    method: str
    url: str
    route: OperationDef | None
    """The OperationDef of the route, when the route was given one."""
    params: Mapping[str, str]
    body: bytes


@dataclass(frozen=True)
class MockMatch:
    index: int
    params: dict[str, str]


@dataclass(frozen=True)
class _Route:
    method: str
    segments: list[str | re.Pattern[str]]
    names: list[list[str]]
    literals: int
    handler: Handler
    operation: OperationDef | None


_PART = re.compile(r"\{([^}]+)\}|([^{]+)")


def _compile(method: str, path: str, handler: Handler, operation: OperationDef | None) -> _Route:
    segments: list[str | re.Pattern[str]] = []
    names: list[list[str]] = []
    literals = 0
    for segment in path.split("/"):
        if "{" not in segment:
            segments.append(segment)
            names.append([])
            literals += 1
            continue
        found: list[str] = []
        source = ""
        for m in _PART.finditer(segment):
            if m.group(1) is not None:
                found.append(m.group(1))
                source += "(.+)"
            else:
                source += re.escape(m.group(2))
        segments.append(re.compile(f"^{source}$"))
        names.append(found)
    return _Route(method.upper(), segments, names, literals, handler, operation)


class MockRouter:
    """Routes requests to handlers; call it with an ``httpx.Request`` (an httpx mock handler)."""

    def __init__(self, *, base_path: str = "", unmatched: Literal["error", "record"] = "error") -> None:
        self.base_path = base_path
        self.unmatched_mode = unmatched
        self._routes: list[_Route] = []
        self.calls: list[MockCall] = []
        """The calls that matched a route, in order."""
        self.unmatched: list[MockCall] = []
        """With ``unmatched="record"``, the requests no route matched."""

    def add(self, operation: OperationDef, handler: Handler) -> MockRouter:
        """Answers the operation's method and path template with ``handler``."""
        self._routes.append(_compile(operation["method"], operation["path"], handler, operation))
        return self

    def extend(self, routes: Iterable[MockRoute]) -> MockRouter:
        """Adds routes as generated mocks return them, in order."""
        for operation, handler in routes:
            self.add(operation, handler)
        return self

    def route(self, method: str, path: str, handler: Handler) -> MockRouter:
        """Answers ``method`` and the path template ``path`` with ``handler``."""
        self._routes.append(_compile(method, path, handler, None))
        return self

    def match(self, method: str, path: str) -> MockMatch | None:
        """The route for a request path (spec/README.md, Mock transports)."""
        path = path.split("?", 1)[0]
        if not path.startswith(self.base_path):
            return None
        parts = path[len(self.base_path) :].split("/")
        best: MockMatch | None = None
        best_literals = -1
        for index, r in enumerate(self._routes):
            if r.method != method.upper() or len(r.segments) != len(parts):
                continue
            params: dict[str, str] = {}
            ok = True
            for segment, names, part in zip(r.segments, r.names, parts, strict=True):
                if isinstance(segment, str):
                    if segment != part:
                        ok = False
                        break
                    continue
                m = segment.match(part)
                if m is None:
                    ok = False
                    break
                for i, name in enumerate(names):
                    params[name] = unquote(m.group(i + 1))
            if ok and r.literals > best_literals:
                best = MockMatch(index, params)
                best_literals = r.literals
        return best

    def __call__(self, request: httpx.Request) -> httpx.Response:
        raw_path = request.url.raw_path.decode("ascii")
        match = self.match(request.method, raw_path)
        body = request.read()
        route = self._routes[match.index] if match is not None else None
        call = MockCall(
            request.method,
            str(request.url),
            route.operation if route is not None else None,
            match.params if match is not None else {},
            body,
        )
        if match is None or route is None:
            path = raw_path.split("?", 1)[0]
            if self.unmatched_mode == "record":
                self.unmatched.append(call)
                return httpx.Response(501, request=request)
            raise httpx.ConnectError(f"tsurara mock: no route for {request.method} {path}", request=request)
        self.calls.append(call)
        try:
            r = route.handler(
                MockRequest(
                    request.method,
                    request.url,
                    match.params,
                    list(request.url.params.multi_items()),
                    request.headers,
                    body,
                )
            )
        except Exception as e:
            raise httpx.ConnectError(f"tsurara mock: the handler failed: {e}", request=request) from e
        headers = dict(r.headers)
        if r.json is not None:
            content = jsonlib.dumps(r.json).encode()
            headers.setdefault("content-type", "application/json")
        elif r.text is not None:
            content = r.text.encode()
        else:
            content = r.content or b""
        if r.content_type is not None:
            headers.setdefault("content-type", r.content_type)
        return httpx.Response(r.status, headers=headers, content=content, request=request)
