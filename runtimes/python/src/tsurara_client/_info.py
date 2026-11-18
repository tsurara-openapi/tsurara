from __future__ import annotations

from dataclasses import dataclass
from typing import Any, Generic, TypeVar

import httpx

Ctx = TypeVar("Ctx")


@dataclass(frozen=True)
class OperationInfo:
    """The operation of a request: for hooks, logs and `ApiError`."""

    id: str
    method: str
    path: str
    tags: tuple[str, ...] = ()


@dataclass(frozen=True)
class RequestInfo(Generic[Ctx]):
    """Passed to `on_result` and to header functions."""

    operation: OperationInfo
    context: Ctx | None


@dataclass(frozen=True)
class UndocumentedStatusInfo(Generic[Ctx]):
    """Passed to `on_undocumented_status`."""

    operation: OperationInfo
    context: Ctx | None
    status: int
    response: httpx.Response


def operation_info(op: Any) -> OperationInfo:
    return OperationInfo(op["id"], op["method"], op["path"], tuple(op.get("tags", ())))
