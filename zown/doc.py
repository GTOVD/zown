"""Print the manifest fields a file actually has.

`sig` is printed only as it is stored. A null signature stays null. The
generator does not invent one.
"""

from __future__ import annotations

from typing import Any

from .manifest import generate


def _show(value: Any) -> str:
    if value is None:
        return "null"
    if isinstance(value, list):
        return "[]" if not value else " ".join(str(item) for item in value)
    if isinstance(value, dict):
        parts = [f"{key}={_show(item)}" for key, item in value.items()]
        return " ".join(parts) if parts else "{}"
    return str(value)


def render(src_file: str) -> str:
    data = generate(src_file)
    lines = [f"source: {data.get('source', '')}"]
    module = data.get("module") or {}
    for key in ("content", "parent", "author", "sig", "ver", "ts", "log", "deps", "patch_of", "tier"):
        if key in module:
            lines.append(f"{key}: {_show(module[key])}")
    lines.append("")
    symbols = data.get("symbols") or {}
    for name in symbols:
        sym = symbols[name]
        lines.append(f"symbol: {name}")
        for key, value in sym.items():
            lines.append(f"  {key}: {_show(value)}")
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"
