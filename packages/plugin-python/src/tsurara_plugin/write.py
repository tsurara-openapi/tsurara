"""Writing Python files from a plugin, with their imports."""

from __future__ import annotations


class PyFile:
    """A Python file at ``path`` (relative to the output root); ``render`` writes the marker first."""

    def __init__(self, path: str) -> None:
        self.path = path
        self._imports: dict[str, set[str]] = {}
        self._lines: list[str] = []

    def import_from(self, module: str, name: str) -> str:
        """Imports ``name`` from ``module`` (``..operations.get_pet``) and returns it."""
        self._imports.setdefault(module, set()).add(name)
        return name

    def line(self, code: str) -> PyFile:
        self._lines.append(code)
        return self

    def render(self, marker: str) -> str:
        imports = [f"from {m} import {', '.join(sorted(n))}" for m, n in sorted(self._imports.items())]
        head = "\n".join(imports)
        return f"{marker}\n\n{head}{chr(10) * 3 if head else ''}{chr(10).join(self._lines)}\n"
