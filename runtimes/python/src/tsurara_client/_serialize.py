"""Serialization of parameters and request bodies (design.md 8.5), as in @tsurara/client. Values here are
JSON values: generated code encodes typed values with TypeDefs first."""

from __future__ import annotations

import json
import re
import secrets
from collections.abc import Mapping
from dataclasses import dataclass
from typing import Any, cast
from urllib.parse import quote

from ._def import BodyDef, EncodingDef, ParamDef, ParamStyle
from ._file import File

# `encodeURIComponent` keeps these; `encodeURI` (allowReserved) also keeps the reserved characters
# except the ones that would end a query value.
_COMPONENT_SAFE = "-_.!~*'()"
_RESERVED_SAFE = _COMPONENT_SAFE + ";,/:@$"


def _encode(text: str, allow_reserved: bool | None = None) -> str:
    return quote(text, safe=_RESERVED_SAFE if allow_reserved else _COMPONENT_SAFE)


def _component(text: str) -> str:
    return quote(text, safe=_COMPONENT_SAFE)


def json_text(value: Any) -> str:
    """`JSON.stringify`: no spaces, non-ASCII kept."""
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False, allow_nan=False)


def to_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, str):
        return value
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int):
        return str(value)
    if isinstance(value, float):
        # As JavaScript prints numbers: 1.0 is "1".
        return str(int(value)) if value.is_integer() and abs(value) < 1e21 else repr(value)
    return json_text(value)


def _is_object(value: Any) -> bool:
    return isinstance(value, Mapping)


def _entries(value: Any) -> list[tuple[str, Any]]:
    return [(str(k), v) for k, v in cast(Mapping[Any, Any], value).items()]


def _content_value(value: Any, content_type: str) -> str:
    return json_text(value) if "json" in content_type.lower() else to_text(value)


def serialize_path_param(name: str, value: Any, d: ParamDef) -> str:
    style: ParamStyle = d.get("style", "simple")
    explode = d.get("explode", False)

    def enc(s: str) -> str:
        return _encode(s, d.get("allowReserved"))

    if "contentType" in d:
        return enc(_content_value(value, d["contentType"]))
    prefix = "." if style == "label" else ";" if style == "matrix" else ""
    if isinstance(value, list | tuple):
        items = [enc(to_text(v)) for v in cast(list[Any], value)]
        if style == "matrix":
            return "".join(f";{name}={i}" for i in items) if explode else f";{name}={','.join(items)}"
        sep = "." if explode and style == "label" else ","
        return prefix + sep.join(items)
    if _is_object(value):
        kv = _entries(value)
        if explode:
            sep = "." if style == "label" else ";" if style == "matrix" else ","
            return prefix + sep.join(f"{enc(k)}={enc(to_text(v))}" for k, v in kv)
        flat = ",".join(x for k, v in kv for x in (enc(k), enc(to_text(v))))
        return f";{name}={flat}" if style == "matrix" else prefix + flat
    text = enc(to_text(value))
    return f";{name}={text}" if style == "matrix" else prefix + text


_PLACEHOLDER = re.compile(r"\{([^}]+)\}")
_ENCODED_DOT = re.compile("%2e", re.IGNORECASE)


def _is_dot_or_empty(segment: str) -> bool:
    """A segment that URL parsing would remove or merge: empty, `.` or `..` (also percent-encoded)."""
    return _ENCODED_DOT.sub(".", segment) in ("", ".", "..")


def expand_path(template: str, values: Mapping[str, Any] | None, defs: Mapping[str, ParamDef]) -> str:
    """Embeds path parameters into the path template. Raises `TypeError` when a parameter is missing,
    or when its value would make a whole segment empty, `.` or `..`: URL parsing removes dot segments,
    so such a value would call another endpoint (spec/README.md). A template that does not start with
    `/` raises too: appended to a base URL, it could change the host or port."""
    if not template.startswith("/"):
        raise TypeError(f"tsurara: the path {template!r} does not start with /")

    def replace(m: re.Match[str]) -> str:
        name = m.group(1)
        if values is None or name not in values:
            raise TypeError(f"tsurara: the path parameter {name} is missing")
        return serialize_path_param(name, values[name], defs.get(name, {}))

    segments: list[str] = []
    for segment in template.split("/"):
        if "{" not in segment:
            segments.append(segment)
            continue
        expanded = _PLACEHOLDER.sub(replace, segment)
        if any(_is_dot_or_empty(s) for s in expanded.split("/")):
            raise TypeError(f"tsurara: a path parameter value makes the segment {expanded!r}, which would change the path")
        segments.append(expanded)
    return "/".join(segments)


def serialize_query_param(name: str, value: Any, d: ParamDef) -> list[str]:
    style: ParamStyle = d.get("style", "form")
    explode = d.get("explode", style == "form")

    def enc(s: str) -> str:
        return _encode(s, d.get("allowReserved"))

    key = _component(name)
    if "contentType" in d:
        return [f"{key}={enc(_content_value(value, d['contentType']))}"]
    if style == "deepObject" and _is_object(value):
        out: list[str] = []

        def walk(prefix: str, v: Any) -> None:
            if _is_object(v):
                for k, x in _entries(v):
                    walk(f"{prefix}[{k}]", x)
            elif isinstance(v, list | tuple):
                for x in cast(list[Any], v):
                    out.append(f"{_component(prefix)}={enc(to_text(x))}")
            else:
                out.append(f"{_component(prefix)}={enc(to_text(v))}")

        walk(name, value)
        return out
    sep = "%20" if style == "spaceDelimited" else "|" if style == "pipeDelimited" else ","
    if isinstance(value, list | tuple):
        items = [enc(to_text(v)) for v in cast(list[Any], value)]
        if explode:
            return [f"{key}={i}" for i in items]
        return [f"{key}={sep.join(items)}"]
    if _is_object(value):
        kv = _entries(value)
        if explode:
            return [f"{_component(k)}={enc(to_text(v))}" for k, v in kv]
        return [f"{key}={sep.join(x for k, v in kv for x in (enc(k), enc(to_text(v))))}"]
    return [f"{key}={enc(to_text(value))}"]


def build_query(values: Mapping[str, Any] | None, defs: Mapping[str, ParamDef]) -> str:
    """The query string. `None` values are left out: RFC 6570, which the styles come from, treats
    null as undefined."""
    if not values:
        return ""
    return "&".join(
        p for name, v in values.items() if v is not None for p in serialize_query_param(name, v, defs.get(name, {}))
    )


def build_query_string(value: Any, content_type: str) -> str:
    """`in: querystring` (3.2): the whole query string as one value."""
    if "x-www-form-urlencoded" in content_type.lower() and _is_object(value):
        return "&".join(
            p for k, v in _entries(value) if v is not None for p in serialize_query_param(k, v, {"style": "form", "explode": True})
        )
    return _component(_content_value(value, content_type))


def serialize_header_param(value: Any, d: ParamDef) -> str:
    if "contentType" in d:
        return _content_value(value, d["contentType"])
    if isinstance(value, list | tuple):
        return ",".join(to_text(v) for v in cast(list[Any], value))
    if _is_object(value):
        kv = _entries(value)
        if d.get("explode", False):
            return ",".join(f"{k}={to_text(v)}" for k, v in kv)
        return ",".join(x for k, v in kv for x in (k, to_text(v)))
    return to_text(value)


def build_cookie(values: Mapping[str, Any] | None, defs: Mapping[str, ParamDef]) -> str:
    if not values:
        return ""
    parts: list[str] = []
    for name, v in values.items():
        d = defs.get(name, {})
        explode = d.get("explode", True)
        if "contentType" in d:
            parts.append(f"{name}={_component(_content_value(v, d['contentType']))}")
        elif isinstance(v, list | tuple):
            items = [_component(to_text(x)) for x in cast(list[Any], v)]
            if explode:
                parts.extend(f"{name}={i}" for i in items)
            else:
                parts.append(f"{name}={','.join(items)}")
        elif _is_object(v):
            if explode:
                parts.extend(f"{k}={_component(to_text(x))}" for k, x in _entries(v))
            else:
                flat = ",".join(y for k, x in _entries(v) for y in (k, _component(to_text(x))))
                parts.append(f"{name}={flat}")
        else:
            parts.append(f"{name}={_component(to_text(v))}")
    return "; ".join(parts)


@dataclass
class EncodedBody:
    content: bytes
    content_type: str | None


def media_type_of(content_type: str | None) -> str:
    return (content_type or "").split(";")[0].strip().lower()


def _is_binary(value: Any) -> bool:
    return isinstance(value, bytes | bytearray | memoryview | File)


def _binary(value: Any) -> tuple[bytes, str | None]:
    if isinstance(value, File):
        return value.content, value.content_type
    return bytes(cast(bytes, value)), None


def _multipart(value: Any, encoding: Mapping[str, EncodingDef]) -> EncodedBody:
    boundary = secrets.token_hex(16)
    chunks: list[bytes] = []

    def part(name: str, content: bytes, content_type: str | None = None, filename: str | None = None) -> None:
        disposition = f'form-data; name="{_quote_param(name)}"'
        if filename is not None:
            disposition += f'; filename="{_quote_param(filename)}"'
        head = f"--{boundary}\r\nContent-Disposition: {disposition}\r\n"
        if content_type is not None:
            head += f"Content-Type: {content_type}\r\n"
        chunks.append(head.encode() + b"\r\n" + content + b"\r\n")

    def append(name: str, v: Any, enc: EncodingDef | None) -> None:
        if isinstance(v, File):
            part(name, v.content, v.content_type or "application/octet-stream", v.filename)
        elif _is_binary(v):
            part(name, _binary(v)[0], "application/octet-stream")
        elif enc and "contentType" in enc and (_is_object(v) or isinstance(v, list | tuple)):
            part(name, json_text(v).encode(), enc["contentType"])
        elif _is_object(v):
            # A field with the JSON text, as the TypeScript runtime sends it (FormData can only give
            # a part a Content-Type by making it a file).
            part(name, json_text(v).encode())
        elif enc and "contentType" in enc and "json" in enc["contentType"].lower():
            part(name, json_text(v).encode(), enc["contentType"])
        else:
            part(name, to_text(v).encode())

    if _is_object(value):
        for name, v in _entries(value):
            enc = encoding.get(name)
            if isinstance(v, list | tuple) and not (enc and "contentType" in enc):
                for item in cast(list[Any], v):
                    append(name, item, enc)
            else:
                append(name, v, enc)
    chunks.append(f"--{boundary}--\r\n".encode())
    return EncodedBody(b"".join(chunks), f"multipart/form-data; boundary={boundary}")


def _quote_param(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("\r", "%0D").replace("\n", "%0A")


def encode_body(value: Any, d: BodyDef, requested: str | None) -> EncodedBody:
    """Builds the request body for the requested media type, or the first one of the definition.
    Raises `TypeError` for values that cannot be sent as that media type."""
    contents = d["contents"]
    content_type = requested or next(iter(contents), "application/json")
    mt = media_type_of(content_type)
    media = contents.get(content_type) or contents.get(mt) or {}
    encoding = media.get("encoding", {})
    if mt.startswith("multipart/"):
        return _multipart(value, encoding)
    if mt == "application/x-www-form-urlencoded":
        parts: list[str] = []
        if _is_object(value):
            for name, v in _entries(value):
                e = encoding.get(name, {})
                pd: ParamDef = {"style": e.get("style", "form"), "explode": e.get("explode", True)}
                if "contentType" in e:
                    pd["contentType"] = e["contentType"]
                parts.extend(serialize_query_param(name, v, pd))
        return EncodedBody("&".join(parts).replace("%20", "+").encode(), content_type)
    if mt.startswith("text/"):
        return EncodedBody(to_text(value).encode(), content_type)
    if "*" in mt:
        # A wildcard: binary values are sent as is, with a File's type; anything else as JSON.
        if _is_binary(value):
            content, ctype = _binary(value)
            return EncodedBody(content, ctype)
        return EncodedBody(json_text(value).encode(), "application/json")
    if mt == "application/json" or mt.endswith("+json"):
        if _is_binary(value):
            raise TypeError(f"{type(value).__name__} cannot be sent as a {mt} body")
        return EncodedBody(json_text(value).encode(), content_type)
    if not _is_binary(value):
        raise TypeError(f"{type(value).__name__} cannot be sent as a {mt} body; pass bytes or File")
    return EncodedBody(_binary(value)[0], content_type)
