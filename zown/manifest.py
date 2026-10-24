"""Shadow manifest (.zn.json).

The manifest bridges Zown's 1-2 char tokens to human/AI-readable intent. Raw Zown
stays microscopic; the manifest carries the architectural index: alias, a prose
description, and an `ai_hint` that tells an agent how to safely modify the symbol.

As of v0.2 (PLAN M7b) the manifest also carries *authority*, *provenance*, and
*telemetry* metadata without inflating the source (MANIFEST.md "Manifest v2"):

  * per user symbol: `caps` (capabilities the code touches, partly discovered
    from the source), `sec` (constant-time / secret flags), `tele` (telemetry the
    runtime emits), `i18n` (translation keys);
  * a module-level `module` provenance block (content hash, author, signature,
    version chain) — scaffolded with nulls until the M14 distribution layer fills
    it in with real signed values.

`generate` scans a source file, discovers every binding and every builtin word it
uses (plus the capabilities each bound block references), and merges that into an
existing manifest WITHOUT clobbering descriptions or metadata a human or AI has
already written.
"""

from __future__ import annotations

import json
import os
from typing import Any

from .builtins import WORDS
from .parser import parse

MANIFEST_VERSION = "0.2"


def manifest_path(src_file: str) -> str:
    return src_file + ".json"


def _scan_caps(nodes: list) -> set[str]:
    """Recursively collect capability names (`name -> "name") used in a block."""
    found: set[str] = set()
    for node in nodes:
        tag = node[0]
        if tag == "cap":
            found.add(node[1])
        elif tag == "blk":
            found |= _scan_caps(node[1])
    return found


def _collect(nodes: list, binds: dict[str, str], used: set[str],
             bind_caps: dict[str, set[str]]) -> None:
    """Walk the AST; record bind names (+ inferred type + referenced caps) and
    used builtin words."""
    prev: tuple | None = None
    for node in nodes:
        tag = node[0]
        if tag == "blk":
            _collect(node[1], binds, used, bind_caps)
        elif tag == "bind":
            name = node[1]
            if prev is not None and prev[0] == "blk":
                binds[name] = "block"
                # a block-bound symbol's caps = every capability its body touches
                bind_caps[name] = _scan_caps(prev[1])
            else:
                binds[name] = "value"
                bind_caps.setdefault(name, set())
        elif tag == "name":
            if node[1] in WORDS:
                used.add(node[1])
        prev = node


def _merge_caps(prior: list, discovered: set[str]) -> list[str]:
    """Union authored caps with freshly discovered ones (never lose hand-added)."""
    disc = {f"`{c}" for c in discovered}
    return sorted(set(prior) | disc)


def _module_block(prior: dict[str, Any]) -> dict[str, Any]:
    """Scaffold the signed provenance block, preserving any authored values.

    Crypto fields stay null until the M14 distribution layer computes a real
    BLAKE3 content hash and Ed25519 signature — we never fabricate provenance.
    """
    return {
        "content": prior.get("content"),       # zown:<blake3>  (filled by M14)
        "parent": prior.get("parent"),          # previous version (null = genesis)
        "author": prior.get("author"),          # zown:node:<pubkey>
        "sig": prior.get("sig"),                # ed25519:<sig>
        "ver": prior.get("ver", "0.0.0"),
        "ts": prior.get("ts"),
        "log": prior.get("log", ""),
        "deps": prior.get("deps", []),
        "patch_of": prior.get("patch_of"),      # set if this version is a SemanticPatch
        "tier": prior.get("tier"),              # trust tier that applied it
    }


def generate(src_file: str) -> dict[str, Any]:
    with open(src_file, "r", encoding="utf-8") as fh:
        src = fh.read()
    nodes = parse(src, src_file)

    binds: dict[str, str] = {}
    used: set[str] = set()
    bind_caps: dict[str, set[str]] = {}
    _collect(nodes, binds, used, bind_caps)

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

    # user bindings: keep any previously authored prose/metadata, scaffold the
    # rest, and merge discovered capabilities (the v2 authority/telemetry fields).
    for name, kind in sorted(binds.items()):
        prior = prev_syms.get(name, {})
        symbols[name] = {
            "type": prior.get("type", kind),
            "alias": prior.get("alias", name),
            "desc": prior.get("desc", "TODO: describe what this symbol holds/does"),
            "ai_hint": prior.get("ai_hint", ""),
            "caps": _merge_caps(prior.get("caps", []), bind_caps.get(name, set())),
            "sec": prior.get("sec", {"ct": False, "secret": False}),
            "tele": prior.get("tele", {"latency": False, "errors": []}),
            "i18n": prior.get("i18n", {"keys": []}),
        }

    # stdlib words actually used, documented from the source of truth. Builtins
    # keep the lean v1 shape — their semantics live in WORDS, not per-program.
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
        "module": _module_block(existing.get("module", {}) or {}),
        "symbols": symbols,
    }


def write(src_file: str) -> str:
    data = generate(src_file)
    out_path = manifest_path(src_file)
    with open(out_path, "w", encoding="utf-8") as fh:
        json.dump(data, fh, indent=2)
        fh.write("\n")
    return out_path
