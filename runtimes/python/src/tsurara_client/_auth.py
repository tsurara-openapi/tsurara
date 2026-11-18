"""Authentication (spec/README.md): the credentials of the first alternative of an operation's
`security` for which the auth hook gives every one, placed by scheme."""

from __future__ import annotations

import base64
import inspect
from collections.abc import Awaitable, Callable, Mapping, Sequence
from dataclasses import dataclass, field
from typing import Any
from urllib.parse import quote


@dataclass(frozen=True)
class SecurityScheme:
    """A security scheme an operation uses, as the auth hook receives it."""

    scheme: str
    """The name in `components.securitySchemes`."""
    type: str
    """`apiKey`, `http`, `oauth2`, `openIdConnect` or `mutualTLS`."""
    in_: str | None = None
    """`apiKey`: `header`, `query` or `cookie`."""
    name: str | None = None
    """`apiKey`: the header, query or cookie name."""
    http_scheme: str | None = None
    """`http`: the Authorization scheme, such as `bearer` or `basic`."""
    scopes: tuple[str, ...] = ()

    @staticmethod
    def of(use: Mapping[str, Any]) -> SecurityScheme:
        return SecurityScheme(
            scheme=use["scheme"],
            type=use["type"],
            in_=use.get("in"),
            name=use.get("name"),
            http_scheme=use.get("httpScheme"),
            scopes=tuple(use.get("scopes", ())),
        )


@dataclass
class Credentials:
    headers: dict[str, str] = field(default_factory=lambda: {})
    query: list[str] = field(default_factory=lambda: [])
    cookies: list[str] = field(default_factory=lambda: [])


Get = Callable[[SecurityScheme], "str | None | Awaitable[str | None]"]


def _place(out: Credentials, s: SecurityScheme, value: str) -> None:
    if s.type == "apiKey":
        name = s.name or s.scheme
        if s.in_ == "query":
            out.query.append(f"{quote(name, safe='')}={quote(value, safe='')}")
        elif s.in_ == "cookie":
            out.cookies.append(f"{name}={quote(value, safe='')}")
        else:
            out.headers[name.lower()] = value
        return
    http = (s.http_scheme or "bearer") if s.type == "http" else "bearer"
    if http.lower() == "basic":
        out.headers["authorization"] = "Basic " + base64.b64encode(value.encode()).decode()
    elif http.lower() == "bearer":
        out.headers["authorization"] = f"Bearer {value}"
    else:
        out.headers["authorization"] = f"{http} {value}"


def _alternatives(security: Sequence[Sequence[Mapping[str, Any]]] | None) -> list[list[SecurityScheme]]:
    # mutualTLS is handled by the transport.
    return [[SecurityScheme.of(u) for u in alt if u["type"] != "mutualTLS"] for alt in security or []]


def authenticate(security: Sequence[Sequence[Mapping[str, Any]]] | None, get: Get) -> Credentials:
    out = Credentials()
    for alternative in _alternatives(security):
        values: list[tuple[SecurityScheme, str]] = []
        for s in alternative:
            value = get(s)
            if inspect.isawaitable(value):
                raise TypeError("an async auth function needs AsyncClient")
            if value is None:
                break
            values.append((s, value))
        if len(values) == len(alternative):
            for s, v in values:
                _place(out, s, v)
            return out
    return out


async def authenticate_async(security: Sequence[Sequence[Mapping[str, Any]]] | None, get: Get) -> Credentials:
    out = Credentials()
    for alternative in _alternatives(security):
        values: list[tuple[SecurityScheme, str]] = []
        for s in alternative:
            value = get(s)
            if inspect.isawaitable(value):
                value = await value
            if value is None:
                break
            values.append((s, value))
        if len(values) == len(alternative):
            for s, v in values:
                _place(out, s, v)
            return out
    return out
