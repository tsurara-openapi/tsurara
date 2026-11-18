"""``python -m tsurara_plugin <module>``: serves the plugin the module defines as ``plugin`` over the
session protocol (one JSON message per line on stdin and stdout)."""

from __future__ import annotations

import importlib.util
import json
import sys
from typing import Any

from . import SESSION_PROTOCOL, Plugin
from .protocol._typedefs import CODEC

STAGES = {
    "transformDocument": "transform_document",
    "transformModel": "transform_model",
    "extend": "extend",
    "transformFiles": "transform_files",
}

FILES = {"list": {"ref": "OutputFile"}}


def load(path: str) -> Plugin:
    spec = importlib.util.spec_from_file_location("tsurara_plugin_module", path)
    if spec is None or spec.loader is None:
        raise SystemExit(f"cannot load {path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    plugin = getattr(module, "plugin", None)
    if not isinstance(plugin, Plugin):
        raise SystemExit(f"{path} does not define `plugin = define_plugin(...)`")
    return plugin


def call(plugin: Plugin, stage: str, value: Any) -> Any:
    function = getattr(plugin, STAGES[stage])
    if function is None:
        raise ValueError(f"the plugin has no {stage} stage")
    if stage == "extend":
        return CODEC.encode(FILES, function(CODEC.decode({"ref": "ExtendInput"}, value)))
    if stage == "transformFiles":
        return CODEC.encode(FILES, function(CODEC.decode(FILES, value)))
    return function(value)


def main() -> None:
    plugin = load(sys.argv[1])
    for line in sys.stdin:
        if not line.strip():
            continue
        request = json.loads(line)
        if request["type"] == "hello":
            stages = [s for s, attr in STAGES.items() if getattr(plugin, attr) is not None]
            answer: dict[str, Any] = {
                "type": "hello",
                "protocol": SESSION_PROTOCOL,
                "name": plugin.name,
                "stages": stages,
            }
        else:
            try:
                answer = {"type": "result", "output": call(plugin, request["stage"], request["input"])}
            except Exception as e:  # noqa: BLE001 - every failure is reported to tsurara
                answer = {"type": "error", "message": str(e)}
        sys.stdout.write(json.dumps(answer) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
