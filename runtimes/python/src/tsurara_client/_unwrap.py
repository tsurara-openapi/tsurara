from __future__ import annotations

import inspect
from typing import Any, Generic, TypeVar

from ._info import OperationInfo
from ._result import AnyResult, Success

T = TypeVar("T")
R = TypeVar("R", bound=AnyResult)


class ApiError(Exception, Generic[R]):
    """Raised by `unwrap` for a result that is not a success. `result` is the original result; with
    an operation's Result type (`ApiError[GetPetResult]`) it has that type."""

    def __init__(self, result: R, operation: OperationInfo | None = None) -> None:
        self.result: R = result
        self.operation = operation or result.operation
        self.kind = result.kind
        self.status: int | None = getattr(result, "status", None)
        what = f"HTTP {self.status}" if self.status is not None else result.kind
        name = self.operation.id if self.operation is not None else "the operation"
        super().__init__(f"tsurara: {name} failed with {what}")
        cause = getattr(result, "cause", None)
        if isinstance(cause, BaseException):
            self.__cause__ = cause

    @property
    def operation_id(self) -> str | None:
        return self.operation.id if self.operation is not None else None


def unwrap(result: Success[T] | AnyResult) -> T:
    """Returns `data` of a success, and raises `ApiError` otherwise. A value that is not a result
    (such as the coroutine of an async call that was not awaited) raises `TypeError`."""
    value: Any = result
    if inspect.isawaitable(value):
        raise TypeError("tsurara: unwrap() received an awaitable; await the call first: unwrap(await call(...))")
    if not hasattr(value, "ok") or not hasattr(value, "kind"):
        raise TypeError(f"tsurara: unwrap() received a value that is not a result: {value!r}")
    if isinstance(result, Success):
        return result.data  # pyright: ignore[reportUnknownVariableType, reportUnknownMemberType]
    raise ApiError(result)
