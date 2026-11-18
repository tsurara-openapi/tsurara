"""Testing plugins: ``run_plugin`` runs tsurara with a plugin on a definition and returns the files."""

from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


def run_plugin(module: str, definition: object, *, config: str = "", bin: str | None = None) -> dict[str, str]:
    """Generates with the plugin module at ``module`` and returns the output files by path."""
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        text = definition if isinstance(definition, str) else json.dumps(definition)
        (root / "openapi.yaml").write_text(text)
        runtime = json.dumps([sys.executable, "-m", "tsurara_plugin"])
        (root / "tsurara.config.yaml").write_text(
            f"input: openapi.yaml\noutput: out\nplugins:\n  - {{ module: {json.dumps(str(Path(module).resolve()))}, runtime: {runtime} }}\n{config}"
        )
        program = bin or os.environ.get("TSURARA_BIN", "tsurara")
        subprocess.run([program, "generate"], cwd=root, check=True, capture_output=True)
        out = root / "out"
        return {p.relative_to(out).as_posix(): p.read_text() for p in out.rglob("*") if p.is_file()}
