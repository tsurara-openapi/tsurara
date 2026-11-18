"""The TypeDef vectors of spec/conformance/typedefs.json. They exercise the types layer only: this module imports neither httpx nor pytest, and runs as a script in an environment without
them (`crates/tsurara-lang-python/tests/types_only.rs`).
"""

from __future__ import annotations

import dataclasses
import enum
import json
from pathlib import Path
from typing import Any

from tsurara_client import TypeDefs

SPEC = Path(__file__).resolve().parents[3] / "spec" / "conformance"


def load(name: str) -> Any:
    return json.loads((SPEC / name).read_text(encoding="utf-8"))


# TypeDef vectors: values are compared in the neutral form of spec/conformance/README.md, so the
# runner builds classes for the TypeDefs and converts decoded values back to that form.

TYPEDEFS = load("typedefs.json")


def default(field: dict[str, Any]) -> Any:
    """Defaults as generated code has them: `MISSING` only for optional and nullable fields."""
    from tsurara_client import MISSING

    return dataclasses.field(default=MISSING if field["presence"] == "optionalNullable" else None)


def build_classes(doc: dict[str, Any]) -> dict[str, type]:
    classes: dict[str, type] = {}
    for name, d in doc["types"].items():
        if d["kind"] == "record":
            fields: list[Any] = [(f["member"], Any, default(f)) for f in d["fields"]]
            if "additional" in d:
                fields.append(("additional_properties", Any, dataclasses.field(default_factory=dict)))
            classes[name] = dataclasses.make_dataclass(name, fields, kw_only=True)
        elif d["kind"] == "enum":
            members = {f"V{i}": v for i, v in enumerate(d["values"])}
            classes[name] = enum.Enum(name, members)  # type: ignore[misc]
    # Bases: a record with a base gets the base's fields too.
    for name, d in doc["types"].items():
        if d["kind"] == "record" and "base" in d:
            base = classes[d["base"]]
            own = [(f.name, Any, dataclasses.field(default=f.default)) for f in dataclasses.fields(classes[name])]
            classes[name] = dataclasses.make_dataclass(name, own, bases=(base,), kw_only=True)
    return classes


CLASSES = build_classes(TYPEDEFS["typeDefs"])


class Neutral:
    """Converts decoded values to the neutral form, walking the TypeDefs: the runtime keeps unlisted
    enum and union values as their JSON values, which only the type tells apart."""

    def __init__(self, doc: dict[str, Any]) -> None:
        self.types: dict[str, Any] = doc["types"]

    def value(self, ty: Any, v: Any) -> Any:
        if isinstance(ty, str):
            return v
        if "list" in ty:
            return [self.value(ty["list"], x) for x in v]
        if "map" in ty:
            return {k: self.value(ty["map"], x) for k, x in v.items()}
        if "tuple" in ty:
            items = ty["tuple"]
            return [self.value(items[i] if i < len(items) else ty["rest"], x) for i, x in enumerate(v)]
        if "nullable" in ty:
            return None if v is None else self.value(ty["nullable"], v)
        if "union" in ty:
            return v
        if "ref" in ty:
            return self.decl(ty["ref"], v)
        return v

    def decl(self, name: str, v: Any) -> Any:
        d = self.types[name]
        kind = d["kind"]
        if kind == "record":
            return self.record(name, v)
        if kind == "enum":
            return v.value if isinstance(v, enum.Enum) else {"$unknown": v}
        if kind == "alias":
            return self.value(d["type"], v)
        if kind == "taggedUnion":
            if not (dataclasses.is_dataclass(v) and not isinstance(v, type)):
                return {"$unknown": v}
            return self.record(type(v).__name__, v)
        # Untagged unions: the member is whatever decoded.
        if dataclasses.is_dataclass(v) and not isinstance(v, type):
            return self.record(type(v).__name__, v)
        return v

    def fields(self, name: str) -> list[Any]:
        d = self.types[name]
        own: list[Any] = d["fields"]
        return (self.fields(d["base"]) if "base" in d else []) + own

    def record(self, name: str, v: Any) -> Any:
        from tsurara_client import MISSING

        out: dict[str, Any] = {"$type": name}
        for f in self.fields(name):
            x = getattr(v, f["member"])
            if x is MISSING or (x is None and f["presence"] == "optional"):
                continue
            out[f["member"]] = self.value(f["type"], x)
        extra = getattr(v, "additional_properties", None)
        if extra:
            additional = self.types[name]["additional"]
            out["$additional"] = {k: self.value(additional, x) for k, x in extra.items()}
        return out


def test_typedef_vectors() -> None:
    from tsurara_client import ConversionError

    doc = TYPEDEFS["typeDefs"]
    defs = TypeDefs(doc, classes=CLASSES, converters={"Uuid": UuidConverter()})
    neutral = Neutral(doc)
    for vector in TYPEDEFS["vectors"]:
        ty = vector["type"]
        for case in vector.get("decode", []):
            if case.get("error"):
                try:
                    defs.decode(ty, case["json"])
                except ConversionError:
                    continue
                raise AssertionError(f"no ConversionError: {ty} {case}")
            got = defs.decode(ty, case["json"])
            assert neutral.value(ty, got) == case["value"], (ty, case)
        for case in vector.get("encode", []):
            value = from_neutral(case["value"])
            assert defs.encode(ty, value) == case["json"], (ty, case)


class UuidConverter:
    def decode(self, json: Any) -> Any:
        return {"$external": "Uuid", "value": json}

    def encode(self, value: Any) -> Any:
        return value["value"]


def from_neutral(v: Any) -> Any:
    if isinstance(v, list):
        return [from_neutral(x) for x in v]  # pyright: ignore[reportUnknownVariableType]
    if isinstance(v, dict):
        if "$type" in v:
            cls = CLASSES[v["$type"]]
            args = {k: from_neutral(x) for k, x in v.items() if not k.startswith("$")}  # pyright: ignore[reportUnknownVariableType]
            if "$additional" in v:
                args["additional_properties"] = from_neutral(v["$additional"])
            return cls(**args)
        if "$unknown" in v:
            return v["$unknown"]
        if "$external" in v:
            return v
        return {k: from_neutral(x) for k, x in v.items()}  # pyright: ignore[reportUnknownVariableType]
    return v


if __name__ == "__main__":
    import sys

    test_typedef_vectors()
    assert "httpx" not in sys.modules or sys.argv[1:] != ["--without-httpx"], "the types layer imported httpx"
    print("ok")
