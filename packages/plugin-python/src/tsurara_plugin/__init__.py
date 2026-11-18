"""Write tsurara plugins in Python.

A plugin module defines ``plugin = define_plugin(...)`` with any of the stages. tsurara runs it once per
run (``python -m tsurara_plugin <module>``) and calls the stages it declares over the session protocol
(design.md 11.2). ``extend`` and ``transform_files`` receive and return the generated dataclasses of
``tsurara_plugin.protocol``; ``transform_document`` and ``transform_model`` work on JSON values, so that
nothing a newer tsurara adds is lost.
"""

from __future__ import annotations

from collections.abc import Callable
from dataclasses import dataclass
from typing import Any

from .protocol import ExtendInput, Manifest, Model, OutputFile
from .write import PyFile

__all__ = [
    "SESSION_PROTOCOL",
    "ExtendInput",
    "Manifest",
    "Model",
    "OutputFile",
    "Plugin",
    "PyFile",
    "define_plugin",
]

SESSION_PROTOCOL = 1
"""The session protocol this SDK speaks (``tsurara_backend::session::SESSION_PROTOCOL``)."""


@dataclass(frozen=True, kw_only=True)
class Plugin:
    """A plugin: a name, and any of the stages."""

    name: str
    transform_document: Callable[[Any], Any] | None = None
    transform_model: Callable[[dict[str, Any]], dict[str, Any]] | None = None
    extend: Callable[[ExtendInput], list[OutputFile]] | None = None
    transform_files: Callable[[list[OutputFile]], list[OutputFile]] | None = None


def define_plugin(
    *,
    name: str,
    transform_document: Callable[[Any], Any] | None = None,
    transform_model: Callable[[dict[str, Any]], dict[str, Any]] | None = None,
    extend: Callable[[ExtendInput], list[OutputFile]] | None = None,
    transform_files: Callable[[list[OutputFile]], list[OutputFile]] | None = None,
) -> Plugin:
    """Declares a plugin; the module assigns it to ``plugin``."""
    return Plugin(
        name=name,
        transform_document=transform_document,
        transform_model=transform_model,
        extend=extend,
        transform_files=transform_files,
    )
