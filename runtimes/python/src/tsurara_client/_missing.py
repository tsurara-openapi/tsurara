from __future__ import annotations

from typing import Final


class Missing:
    """A field that is absent from the JSON object, as opposed to `null` (`None`). Only fields that are
    both optional and nullable use it (design.md 18.15, Q3)."""

    _instance: Missing | None = None

    def __new__(cls) -> Missing:
        if cls._instance is None:
            cls._instance = super().__new__(cls)
        return cls._instance

    def __repr__(self) -> str:
        return "MISSING"

    def __bool__(self) -> bool:
        return False


MISSING: Final = Missing()
