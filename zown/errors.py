"""Structured Zown diagnostics (.zerr packets).

In Zown an error is not a passive human-readable tombstone. It is an actionable,
machine-first payload designed to be fed straight into an AI agent so the code can
self-heal. Every failure carries:

  * a deterministic recovery `code` (what kind of thinking is required),
  * the source position,
  * a snapshot of the VM stack at the moment of failure,
  * a short, dense `hint`.

This is the v0.1 seed of the full self-healing loop described in the design notes.
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from typing import Any

ZERR_VERSION = "0.1"

# Deterministic recovery codes. An AI agent keys its repair strategy off these
# rather than parsing free-form text.
REPAIR_SYNTAX = "REPAIR_SYNTAX"        # structural / token mismatch
STACK_UNDERFLOW = "STACK_UNDERFLOW"    # an op needed more values than present
TYPE_MISMATCH = "TYPE_MISMATCH"        # an op got the wrong kind of value
NAME_UNRESOLVED = "NAME_UNRESOLVED"    # referenced a name that is not bound
DIV_ZERO = "DIV_ZERO"                  # division / modulo by zero
BOUNDS = "BOUNDS"                      # index past an allocated bound
NOT_CALLABLE = "NOT_CALLABLE"          # tried to invoke a non-block
UNSUPPORTED = "UNSUPPORTED"            # feature not implemented in this build
OVERFLOW = "OVERFLOW"                  # a value did not fit a checked fixed-width type
NO_MATCH = "NO_MATCH"                  # no `??` arm matched the subject
BAD_PATTERN = "BAD_PATTERN"            # a `??` arm or pattern is malformed

# v0.2 security recovery codes (SPEC.md Part II §18). Security failures ride the
# same structured channel as every other diagnostic so the AI control plane and
# the self-healing loop can act on them uniformly.
CAP_DENIED = "CAP_DENIED"              # an op required a capability that was not granted
AUTH_FAIL = "AUTH_FAIL"                # peer/module authentication failed
INTEGRITY_FAIL = "INTEGRITY_FAIL"      # content hash did not match its claimed hash
SIG_INVALID = "SIG_INVALID"            # a signature did not verify
RATE_LIMITED = "RATE_LIMITED"          # a capability's rate budget was exceeded
UB_TRAP = "UB_TRAP"                    # a would-be-undefined operation was trapped


@dataclass
class Pos:
    line: int = 0
    col: int = 0
    offset: int = 0

    def as_dict(self) -> dict[str, int]:
        return {"line": self.line, "col": self.col, "offset": self.offset}


@dataclass
class ZownError(Exception):
    """A structured Zown error. Renders to a .zerr JSON packet."""

    code: str
    msg: str
    kind: str = "run"  # lex | parse | run | sec
    op: str | None = None
    pos: Pos | None = None
    stack: list[Any] = field(default_factory=list)
    hint: str = ""
    file: str | None = None
    cap: str | None = None  # the capability involved (security packets only)

    def __post_init__(self) -> None:
        Exception.__init__(self, self.msg)

    def packet(self) -> dict[str, Any]:
        pkt = {
            "zerr": ZERR_VERSION,
            "kind": self.kind,
            "code": self.code,
            "msg": self.msg,
            "op": self.op,
            "pos": self.pos.as_dict() if self.pos else None,
            "stack": [_snap(v) for v in self.stack],
            "hint": self.hint,
            "file": self.file,
        }
        # Only present on security packets, so v0.1 packet shapes are unchanged.
        if self.cap is not None:
            pkt["cap"] = self.cap
        return pkt

    def to_json(self, indent: int | None = 2) -> str:
        return json.dumps(self.packet(), indent=indent)

    def render_human(self) -> str:
        loc = ""
        if self.file:
            loc = self.file
        if self.pos:
            loc = f"{loc}:{self.pos.line}:{self.pos.col}" if loc else f"{self.pos.line}:{self.pos.col}"
        head = f"zerr[{self.code}]"
        if loc:
            head += f" {loc}"
        op = f" (op `{self.op}`)" if self.op else ""
        lines = [f"{head}{op}: {self.msg}"]
        if self.hint:
            lines.append(f"  hint: {self.hint}")
        if self.stack:
            snap = ", ".join(str(_snap(v)) for v in self.stack[-6:])
            lines.append(f"  stack: [{snap}]")
        return "\n".join(lines)


def _snap(v: Any) -> Any:
    """Compact, JSON-safe snapshot of a stack value."""
    from .vm import Block, Cap, WidthTag, Vec  # local import to avoid a cycle

    if isinstance(v, Block):
        return f"[blk:{len(v.nodes)}]"
    if isinstance(v, Cap):
        return f"`{v.name}"
    if isinstance(v, WidthTag):
        return v.name
    if isinstance(v, Vec):
        return f"{v.name}[{len(v.lanes)}]"
    if isinstance(v, (int, float, str, bool)) or v is None:
        return v
    return repr(v)
