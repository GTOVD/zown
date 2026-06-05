"""Shadow manifest (.zn.json).

The manifest bridges Zown's 1-2 char tokens to human/AI-readable intent. Raw Zown
stays microscopic; the manifest carries the architectural index: alias, a prose
description, and an `ai_hint` that tells an agent how to safely modify the symbol.

`generate` scans a source file, discovers every binding and every builtin word it
uses, and merges that into an existing manifest WITHOUT clobbering descriptions a
human or AI has already written.
"""

from __future__ import annotations

import json
import os
from typing import Any

from .builtins import WORDS
from .parser import parse

MANIFEST_VERSION = "0.1"


def manifest_path(src_file: str) -> str:
    return src_file + ".json"


def _collect(nodes: list, binds: dict[str, str], used: set[str]) -> None:
    """Walk the AST; record bind names (+ inferred type) and used builtin words."""
    prev_tag: str | None = None
    for node in nodes:
        tag = node[0]
        if tag == "blk":
            _collect(node[1], binds, used)
        elif tag == "bind":
            name = node[1]
            # infer the bound type from the node that produced the value
            binds[name] = "block" if prev_tag == "blk" else "value"
        elif tag == "name":
            if node[1] in WORDS:
                used.add(node[1])
        prev_tag = tag


def generate(src_file: str) -> dict[str, Any]:
    with open(src_file, "r", encoding="utf-8") as fh:
        src = fh.read()
    nodes = parse(src, src_file)

    binds: dict[str, str] = {}
    used: set[str] = set()
    _collect(nodes, binds, used)

    out_path = manifest_path(src_file)
    existing: dict[str, Any] = {}
    if os.path.exists(out_path):
        try:
            with open(out_path, "r", encoding="utf-8") as fh:
                existing = json.load(fh)
        except (json.JSONDecodeError, OSError):
            existing = {}
    prev_syms: dict[str, Any] = existing.get("symbols", {}) or {}

    symbols: dict[str, Any] = {}

    # user bindings: keep any previously authored prose, scaffold the rest
    for name, kind in sorted(binds.items()):
        prior = prev_syms.get(name, {})
        symbols[name] = {
            "type": prior.get("type", kind),
            "alias": prior.get("alias", name),
            "desc": prior.get("desc", "TODO: describe what this symbol holds/does"),
            "ai_hint": prior.get("ai_hint", ""),
        }

    # stdlib words actually used, documented from the source of truth
    for name in sorted(used):
        _, alias, desc = WORDS[name]
        prior = prev_syms.get(name, {})
        symbols[name] = {
            "type": "builtin",
            "alias": alias,
            "desc": desc,
            "ai_hint": prior.get("ai_hint", ""),
        }

    return {
        "language": f"Zown v{MANIFEST_VERSION}",
        "source": os.path.basename(src_file),
        "symbols": symbols,
    }


def write(src_file: str) -> str:
    data = generate(src_file)
    out_path = manifest_path(src_file)
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2)
        fh.write("\n")
    return out_path
