"""Results (spec/README.md, "Result kinds"). A request returns one of these instead of raising.

Every class has `ok` and `kind`, so results can be narrowed by either or matched by class:

    match result:
        case Success(data=pet):
            ...
        case HttpError(status=404):
            ...
"""

from __future__ import annotations

from dataclasses import dataclass, field
from typing import Any, Generic, Literal, TypeVar

import httpx

from ._info import OperationInfo

T = TypeVar("T", covariant=True)
E = TypeVar("E", covariant=True)


@dataclass(frozen=True)
class Success(Generic[T]):
    """A success status of the definition was returned."""

    status: int
    data: T
    response: httpx.Response = field(repr=False)
    ok: Literal[True] = field(default=True, init=False)
    kind: Literal["success"] = field(default="success", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class HttpError(Generic[E]):
    """An error status that the definition documents; `error` has its type."""

    status: int
    error: E
    response: httpx.Response = field(repr=False)
    documented: Literal[True] = field(default=True, init=False)
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["http"] = field(default="http", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class UndocumentedStatus:
    """A status the definition does not document, also a 2xx when other success statuses are
    defined. `error` is the body read as JSON if possible, otherwise as text or bytes."""

    status: int
    error: Any
    response: httpx.Response = field(repr=False)
    documented: Literal[False] = field(default=False, init=False)
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["http"] = field(default="http", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class NetworkFailure:
    """The transport failed (DNS, connection, TLS, and so on)."""

    cause: BaseException
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["network"] = field(default="network", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class TimeoutFailure:
    """The client's timeout was exceeded."""

    cause: BaseException
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["timeout"] = field(default="timeout", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class ParseFailure:
    """The body could not be read as the definition says, including a Content-Type that does not
    match."""

    cause: BaseException
    response: httpx.Response = field(repr=False)
    content_type: str | None
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["parse"] = field(default="parse", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


@dataclass(frozen=True)
class UnexpectedFailure:
    """User code (hooks, converters) failed, or the input cannot be sent."""

    cause: BaseException
    ok: Literal[False] = field(default=False, init=False)
    kind: Literal["unexpected"] = field(default="unexpected", init=False)
    operation: OperationInfo | None = field(default=None, kw_only=True, repr=False, compare=False)
    """The operation that returned the result."""


TransportFailure = NetworkFailure | TimeoutFailure | ParseFailure | UnexpectedFailure
"""Failures that every operation's Result includes."""

AnyResult = Success[Any] | HttpError[Any] | UndocumentedStatus | TransportFailure
"""A type that every operation's Result is assignable to."""
