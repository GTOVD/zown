"""Resolve a package's dependencies by content hash.

A dependency is `zown:` plus the BLAKE3 of a file's bytes, the same hash
`~f` uses for a published string. This command looks in a directory. It
does not call a registry.
"""

from __future__ import annotations

import json
import os

from .blake3 import blake3_hex
from .errors import BOUNDS, TYPE_MISMATCH, ZownError


def _hash_of(path: str) -> str:
    with open(path, "rb") as fh:
        return blake3_hex(fh.read())


def resolve(pkg_path: str, root: str | None = None) -> list[tuple[str, str]]:
    """Return `(zown:hash, relative path)` for each dependency, in order."""
    directory = root or os.path.dirname(os.path.abspath(pkg_path))
    try:
        with open(pkg_path, encoding="utf-8") as fh:
            data = json.load(fh)
    except (OSError, json.JSONDecodeError) as exc:
        raise ZownError(TYPE_MISMATCH, f"package file is not JSON: {exc}", op="pkg") from exc
    deps = data.get("deps") if isinstance(data, dict) else None
    if not isinstance(deps, list) or any(not isinstance(item, str) for item in deps):
        raise ZownError(TYPE_MISMATCH, "package deps must be a list of zown: hashes", op="pkg")
    found: dict[str, str] = {}
    pkg_abs = os.path.abspath(pkg_path)
    for dirpath, _dirs, files in os.walk(directory):
        for name in sorted(files):
            path = os.path.join(dirpath, name)
            if os.path.abspath(path) == pkg_abs:
                continue
            digest = "zown:" + _hash_of(path)
            rel = os.path.relpath(path, directory).replace(os.sep, "/")
            prior = found.get(digest)
            if prior is None or rel < prior:
                found[digest] = rel
    out: list[tuple[str, str]] = []
    for dep in deps:
        if not dep.startswith("zown:") or len(dep) != 5 + 64:
            raise ZownError(TYPE_MISMATCH, f"dependency {dep} is not a zown: hash", op="pkg")
        path = found.get(dep)
        if path is None:
            raise ZownError(BOUNDS, f"dependency {dep} is not in {directory}", op="pkg")
        out.append((dep, path))
    return out
