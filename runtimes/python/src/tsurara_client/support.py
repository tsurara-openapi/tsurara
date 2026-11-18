"""Helpers that generated code uses."""

from __future__ import annotations

from collections.abc import Mapping
from typing import Any


def present(values: Mapping[str, Any]) -> dict[str, Any]:
    """The parameters that were given: generated functions default optional parameters to `None`,
    which means not sent."""
    return {k: v for k, v in values.items() if v is not None}
