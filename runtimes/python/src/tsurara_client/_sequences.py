"""Responses read item by item: Server-Sent Events, JSON Lines and JSON text sequences (design.md
8.5). The readers take the body in chunks and hand over each complete item."""

from __future__ import annotations

import codecs
import json
import re
from collections.abc import Iterator, Sequence
from typing import Any, Literal

SequenceKind = Literal["events", "jsonLines", "jsonSeq"]
SEQUENCES: tuple[SequenceKind, ...] = ("events", "jsonLines", "jsonSeq")

_LINE_END = re.compile(r"\r\n|\r|\n")


class _Text:
    """Decodes UTF-8 chunks (without a leading byte order mark) and splits them at `separator`,
    keeping the text after the last separator for the next chunk."""

    def __init__(self, separator: re.Pattern[str] | str) -> None:
        self._decoder = codecs.getincrementaldecoder("utf-8-sig")("replace")
        self._separator = separator
        self._buffer = ""

    def feed(self, chunk: bytes, final: bool) -> list[str]:
        self._buffer += self._decoder.decode(chunk, final)
        # A CR at the end may be the first half of a CRLF, so it waits for what follows.
        held = "\r" if not final and self._separator is _LINE_END and self._buffer.endswith("\r") else ""
        text = self._buffer[: len(self._buffer) - len(held)]
        if isinstance(self._separator, str):
            parts = text.split(self._separator)
        else:
            parts = self._separator.split(text)
        self._buffer = parts.pop() + held
        if final:
            parts.append(self._buffer)
            self._buffer = ""
        return parts


class Reader:
    """Turns the chunks of a body into items: event objects (`{"event", "data", "id"?, "retry"?}`)
    or JSON values. Invalid JSON raises `ValueError` when its item is reached."""

    def __init__(self, kind: SequenceKind, json_events: bool | Sequence[str] | None = None) -> None:
        self._kind = kind
        self._json_events = json_events
        self._text = _Text("\x1e" if kind == "jsonSeq" else _LINE_END)
        # Text before the first record separator is not a record.
        self._started = kind != "jsonSeq"
        self._count = 0
        self._event = ""
        self._data: list[str] = []
        self._id: str | None = None
        self._retry: int | None = None

    def feed(self, chunk: bytes, final: bool = False) -> Iterator[Any]:
        parts = self._text.feed(chunk, final)
        if not self._started and (len(parts) > 1 or (parts and not final)):
            parts = parts[1:]
            self._started = True
        elif not self._started:
            parts = []
        if self._kind == "events":
            # The last part of the final chunk is an event the stream ended in the middle of.
            for line in parts[:-1] if final else parts:
                event = self._line(line)
                if event is not None:
                    yield event
            return
        for text in parts:
            self._count += 1
            if text.strip() == "":
                continue
            try:
                yield json.loads(text)
            except ValueError as e:
                what = "line" if self._kind == "jsonLines" else "record"
                raise ValueError(f"{what} {self._count} is not valid JSON") from e

    def _line(self, line: str) -> dict[str, Any] | None:
        if line == "":
            event: dict[str, Any] | None = None
            if self._data:
                name = self._event or "message"
                data: Any = "\n".join(self._data)
                if self._json_events is True or (
                    isinstance(self._json_events, Sequence) and name in self._json_events
                ):
                    try:
                        data = json.loads(data)
                    except ValueError as e:
                        raise ValueError(f'the data of the event "{name}" is not valid JSON') from e
                event = {"event": name, "data": data}
                if self._id is not None:
                    event["id"] = self._id
                if self._retry is not None:
                    event["retry"] = self._retry
            self._event, self._data, self._id, self._retry = "", [], None, None
            return event
        if line.startswith(":"):
            return None
        field, _, value = line.partition(":")
        if value.startswith(" "):
            value = value[1:]
        if field == "event":
            self._event = value
        elif field == "data":
            self._data.append(value)
        elif field == "id" and "\0" not in value:
            self._id = value
        elif field == "retry" and value.isascii() and value.isdigit():
            self._retry = int(value)
        return None
