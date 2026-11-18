from __future__ import annotations

from dataclasses import dataclass, field


@dataclass(frozen=True)
class File:
    """A binary value with an optional file name and media type, for multipart parts and binary
    bodies. Plain `bytes` work wherever no name or type is needed."""

    content: bytes = field(repr=False)
    filename: str | None = None
    content_type: str | None = None
