"""Conversion between JSON and typed values with TypeDefs (design.md 18.7, spec/type-def.schema.json).

Records become the generated dataclasses and enums their `Enum` classes, found by name. Unlisted
values of open enums and open tagged unions stay as their JSON values, as the generated annotations
say (`Status | str`). Dates and other formats stay strings.
"""

from __future__ import annotations

import dataclasses
import enum
import importlib
from collections.abc import Mapping
from typing import Any, Protocol, cast

from ._file import File
from ._missing import MISSING

Json = Any
TypeRef = Any


class ConversionError(ValueError):
    """A JSON value does not match its TypeDef."""


class Converter(Protocol):
    """Converts the values of an external type (`x-tsurara-type`), registered by the name the TypeDef
    gives (`{"external": name}`). `decode` receives the value decoded with the type's schema."""

    def decode(self, json: Json, /) -> Any: ...

    def encode(self, value: Any, /) -> Json: ...


class TypeDefs:
    """A TypeDef document with the classes it names.

    Classes are looked up by name in `classes`, then in the module `module` (the generated
    package). A record without a class decodes to a dict of its members.
    """

    def __init__(
        self,
        document: Mapping[str, Any],
        *,
        classes: Mapping[str, type] | None = None,
        module: str | None = None,
        converters: Mapping[str, Converter] | None = None,
    ) -> None:
        if document.get("v") != 1:
            raise ValueError(f"TypeDef version {document.get('v')!r} is not supported (expected 1)")
        self._types: Mapping[str, Any] = document["types"]
        self._classes = dict(classes or {})
        self._module = module
        self._converters = dict(converters or {})

    def with_converters(self, converters: Mapping[str, Converter]) -> TypeDefs:
        """A copy with more converters for external types."""
        out = TypeDefs({"v": 1, "types": self._types}, classes=self._classes, module=self._module)
        out._converters = {**self._converters, **converters}
        return out

    def _class(self, name: str) -> type | None:
        cls = self._classes.get(name)
        if cls is None and self._module is not None:
            cls = getattr(importlib.import_module(self._module), name, None)
            if cls is not None:
                self._classes[name] = cls
        return cls

    # Decoding

    def decode(self, ty: TypeRef, json: Json) -> Any:
        """Decodes `json` as the type `ty`. Raises `ConversionError` when it does not match."""
        if isinstance(ty, str):
            return self._decode_keyword(ty, json)
        t = cast(Mapping[str, Any], ty)
        if "scalar" in t:
            return _decode_scalar(t["scalar"], json)
        if "list" in t:
            return [self.decode(t["list"], v) for v in _expect(list, json, "a list")]
        if "map" in t:
            return {k: self.decode(t["map"], v) for k, v in _expect(dict, json, "an object").items()}
        if "tuple" in t:
            return self._decode_tuple(t, json)
        if "ref" in t:
            return self._decode_decl(t["ref"], json)
        if "nullable" in t:
            return None if json is None else self.decode(t["nullable"], json)
        if "external" in t:
            value = self.decode(t["schema"], json)
            converter = self._converters.get(t["external"])
            return value if converter is None else converter.decode(value)
        if "literal" in t:
            if not _same_json(t["literal"], json):
                raise ConversionError(f"{json!r} is not {t['literal']!r}")
            return json
        if "union" in t:
            return self._first(t["union"], json)
        raise ConversionError(f"unknown type {ty!r}")

    def _decode_keyword(self, ty: str, json: Json) -> Any:
        if ty == "any":
            return json
        if ty == "null" and json is None:
            return None
        if ty == "bool" and isinstance(json, bool):
            return json
        raise ConversionError(f"{json!r} is not {ty}")

    def _decode_tuple(self, t: Mapping[str, Any], json: Json) -> tuple[Any, ...]:
        items: list[Any] = t["tuple"]
        values = _expect(list, json, "a tuple")
        if len(values) < t.get("required", 0):
            raise ConversionError("too few tuple items")
        out: list[Any] = []
        for i, v in enumerate(values):
            if i < len(items):
                out.append(self.decode(items[i], v))
            elif "rest" in t:
                out.append(self.decode(t["rest"], v))
            else:
                raise ConversionError("too many tuple items")
        return tuple(out)

    def _first(self, members: list[Any], json: Json) -> Any:
        for m in members:
            try:
                return self.decode(m, json)
            except ConversionError:
                continue
        raise ConversionError(f"{json!r} matches no member")

    def _decl(self, name: str) -> Mapping[str, Any]:
        try:
            return self._types[name]
        except KeyError:
            raise ConversionError(f"unknown declaration {name}") from None

    def _decode_decl(self, name: str, json: Json) -> Any:
        d = self._decl(name)
        kind = d["kind"]
        if kind == "record":
            return self._decode_record(name, d, _expect(dict, json, "an object"))
        if kind == "enum":
            values: list[Any] = d["values"]
            if any(_same_json(v, json) for v in values):
                cls = self._class(name)
                return cls(json) if cls is not None and issubclass(cls, enum.Enum) else json
            if d["open"]:
                return json
            raise ConversionError(f"{json!r} is not a value of {name}")
        if kind == "taggedUnion":
            obj = _expect(dict, json, "an object")
            tag = obj.get(d["property"])
            for variant in d["variants"]:
                if tag in variant["values"]:
                    return self.decode(variant["type"], json)
            if "default" in d:
                return self.decode(d["default"], json)
            if d["open"]:
                return json
            raise ConversionError(f"{tag!r} is not a discriminator value of {name}")
        if kind == "untaggedUnion":
            return self._first(d["members"], json)
        if kind == "alias":
            return self.decode(d["type"], json)
        raise ConversionError(f"unknown declaration kind {kind!r}")

    def _fields(self, d: Mapping[str, Any]) -> list[Mapping[str, Any]]:
        own: list[Mapping[str, Any]] = d["fields"]
        return [*self._fields(self._decl(d["base"])), *own] if "base" in d else own

    def _decode_record(self, name: str, d: Mapping[str, Any], obj: dict[str, Any]) -> Any:
        members: dict[str, Any] = {}
        fields = self._fields(d)
        for f in fields:
            wire, member, presence = f["wire"], f["member"], f["presence"]
            if wire not in obj:
                if presence in ("required", "nullable"):
                    raise ConversionError(f"`{wire}` is missing from {name}")
                continue
            v = obj[wire]
            if v is None and presence in ("nullable", "optionalNullable"):
                members[member] = None
            else:
                try:
                    members[member] = self.decode(f["type"], v)
                except ConversionError as e:
                    raise ConversionError(f"{name}.{wire}: {e}") from e
        if "additional" in d:
            known = {f["wire"] for f in fields}
            members["additional_properties"] = {
                k: self.decode(d["additional"], v) for k, v in obj.items() if k not in known
            }
        cls = self._class(name)
        return members if cls is None else cls(**members)

    # Encoding

    def encode(self, ty: TypeRef, value: Any) -> Json:
        """Encodes a typed value as the type `ty`. A missing optional field is omitted; `None` is
        written as `null` only where the field is nullable."""
        if isinstance(ty, Mapping):
            t = cast(Mapping[str, Any], ty)
            if "external" in t:
                converter = self._converters.get(t["external"])
                inner = value if converter is None else converter.encode(value)
                return self.encode(t["schema"], inner)
            if "list" in t and isinstance(value, list | tuple):
                return [self.encode(t["list"], v) for v in cast(list[Any], value)]
            if "map" in t and isinstance(value, Mapping):
                return {k: self.encode(t["map"], v) for k, v in cast(Mapping[str, Any], value).items()}
            if "tuple" in t and isinstance(value, list | tuple):
                items: list[Any] = t["tuple"]
                rest = t.get("rest", "any")
                return [self.encode(items[i] if i < len(items) else rest, v) for i, v in enumerate(cast(list[Any], value))]
            if "nullable" in t:
                return None if value is None else self.encode(t["nullable"], value)
            if "ref" in t:
                return self._encode_decl(t["ref"], value)
        return self._encode_any(value)

    def _encode_decl(self, name: str, value: Any) -> Json:
        d = self._types.get(name)
        if d is None:
            return self._encode_any(value)
        if d["kind"] == "record":
            return self._encode_record(d, value)
        if d["kind"] == "alias":
            return self.encode(d["type"], value)
        return self._encode_any(value)

    def _encode_record(self, d: Mapping[str, Any], value: Any) -> Json:
        if isinstance(value, Mapping):
            def get(member: str) -> Any:
                return cast(Mapping[str, Any], value).get(member, MISSING)
        else:
            def get(member: str) -> Any:
                return getattr(value, member, MISSING)
        out: dict[str, Any] = {}
        for f in self._fields(d):
            v = get(f["member"])
            if v is MISSING or (v is None and f["presence"] == "optional"):
                continue
            out[f["wire"]] = self.encode(f["type"], v)
        extra = get("additional_properties")
        if isinstance(extra, Mapping):
            additional = d.get("additional", "any")
            for k, v in cast(Mapping[str, Any], extra).items():
                out.setdefault(k, self.encode(additional, v))
        return out

    def _encode_any(self, value: Any) -> Json:
        """Values whose type the TypeDef does not pin down (union members, `any`): records by their
        class name, enums by their value, containers item by item."""
        if isinstance(value, enum.Enum):
            return value.value
        if isinstance(value, bytes | File):
            # Binary values (multipart parts) are sent as they are.
            return value
        if dataclasses.is_dataclass(value) and not isinstance(value, type):
            d = self._types.get(type(value).__name__)
            if d is not None and d["kind"] == "record":
                return self._encode_record(d, value)
            return {f.name: self._encode_any(getattr(value, f.name)) for f in dataclasses.fields(value)}
        if isinstance(value, list | tuple):
            return [self._encode_any(v) for v in cast(list[Any], value)]
        if isinstance(value, Mapping):
            return {k: self._encode_any(v) for k, v in cast(Mapping[str, Any], value).items()}
        return value


def _expect(kind: type, json: Json, what: str) -> Any:
    if not isinstance(json, kind):
        raise ConversionError(f"{json!r} is not {what}")
    return json


def _decode_scalar(kind: str, json: Json) -> Any:
    if isinstance(json, bool):
        raise ConversionError(f"{json!r} is not {kind}")
    if kind == "string" and isinstance(json, str):
        return json
    if kind == "number" and isinstance(json, int | float):
        return json
    if kind == "integer":
        if isinstance(json, int):
            return json
        # An integer that arrives as a float with no fraction.
        if isinstance(json, float) and json.is_integer():
            return int(json)
    raise ConversionError(f"{json!r} is not {'an' if kind == 'integer' else 'a'} {kind}")


def _same_json(a: Json, b: Json) -> bool:
    """JSON equality: `true` is not `1`."""
    return a == b and isinstance(a, bool) == isinstance(b, bool)


