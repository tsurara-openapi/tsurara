"""The OperationDef format (spec/operation-def.schema.json), as generated code writes it."""

from __future__ import annotations

from typing import Any, Literal, NotRequired, TypedDict

OPERATION_DEF_VERSION = 1
"""Version of the OperationDef format. Must match the `v` emitted by the generator."""

ParseAs = Literal["json", "text", "blob", "stream", "events", "jsonLines", "jsonSeq", "none", "auto"]
"""How a response body is read. `events`, `jsonLines` and `jsonSeq` read it item by item: the data
is an iterator (an async iterator with `AsyncClient`) of events or JSON values."""
ParamStyle = Literal["simple", "label", "matrix", "form", "spaceDelimited", "pipeDelimited", "deepObject"]


class ParamDef(TypedDict, total=False):
    style: ParamStyle
    explode: bool
    allowReserved: bool
    contentType: str
    type: Any


class ParamsDef(TypedDict, total=False):
    path: dict[str, ParamDef]
    query: dict[str, ParamDef]
    headers: dict[str, ParamDef]
    cookies: dict[str, ParamDef]
    querystring: QueryStringDef


class QueryStringDef(TypedDict):
    contentType: str


class EncodingDef(TypedDict, total=False):
    contentType: str
    style: ParamStyle
    explode: bool


class MediaDef(TypedDict, total=False):
    encoding: dict[str, EncodingDef]
    type: Any


class BodyDef(TypedDict):
    contents: dict[str, MediaDef]
    required: NotRequired[bool]


class ResponseDef(TypedDict):
    parseAs: ParseAs
    contentTypes: NotRequired[dict[str, ParseAs]]
    jsonEvents: NotRequired[bool | list[str]]
    types: NotRequired[dict[str, Any]]


class OperationDef(TypedDict):
    v: Literal[1]
    id: str
    method: str
    path: str
    responses: dict[str, ResponseDef]
    tags: NotRequired[list[str]]
    params: NotRequired[ParamsDef]
    body: NotRequired[BodyDef]
    security: NotRequired[list[list[dict[str, Any]]]]
