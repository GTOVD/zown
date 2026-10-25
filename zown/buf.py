"""Exclusive byte buffers (SPEC §21).

A `buf` has no address the program can read. `bf` allocates. `=` and a bare
name push a borrow. Only the exclusive value may `bp`. `,` on that value
frees the bytes; a later `bg` or `ln` on a borrow is `BOUNDS`.
"""

from __future__ import annotations

from dataclasses import dataclass

from .errors import BOUNDS, TYPE_MISMATCH

# Implementation-defined grant. Larger requests are BOUNDS, not a crash.
BUF_MAX = 1 << 20


class Alloc:
    __slots__ = ("data", "n")

    def __init__(self, n: int) -> None:
        self.data: bytearray | None = bytearray(n)
        self.n = n

    def free(self) -> None:
        self.data = None


@dataclass(eq=False)
class Buf:
    alloc: Alloc
    exclusive: bool

    def borrow(self) -> Buf:
        return Buf(self.alloc, False)

    def __eq__(self, other: object) -> bool:
        return (
            isinstance(other, Buf)
            and self.alloc is other.alloc
            and self.exclusive == other.exclusive
        )

    def text(self) -> str:
        body = "-" if self.alloc.data is None else str(self.alloc.n)
        return f"bf({body})" if self.exclusive else f"bf&({body})"


def release(v) -> None:
    """Destroying an exclusive `buf` frees its bytes. An exclusive stream closes."""
    if isinstance(v, Buf) and v.exclusive:
        v.alloc.free()
        return
    from .stream import Stream
    if isinstance(v, Stream) and v.exclusive:
        v.conn.close()


def _plain_int(vm, op: str, pos, v, what: str):
    if isinstance(v, bool) or not isinstance(v, int):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected {what}, got {type_name(v)}",
                     op=op, pos=pos)
    return v


def _live(vm, buf: Buf, op: str, pos) -> bytearray:
    data = buf.alloc.data
    if data is None:
        raise vm.err(BOUNDS, f"`{op}` buffer was freed", op=op, pos=pos,
                     hint="the exclusive owner was dropped with `,`")
    return data


def _top_buf(vm, op: str, pos) -> Buf:
    if not vm.stack:
        raise vm.err(STACK_UNDERFLOW, f"`{op}` needs a buffer under the index",
                     op=op, pos=pos)
    buf = vm.stack[-1]
    if not isinstance(buf, Buf):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a buf, got {type_name(buf)}",
                     op=op, pos=pos)
    return buf


def b_bf(vm, pos) -> None:
    n = _plain_int(vm, "bf", pos, vm.pop("bf", pos), "an int >= 0")
    if n < 0:
        raise vm.err(TYPE_MISMATCH, f"`bf` expected an int >= 0, got {n}",
                     op="bf", pos=pos)
    if n > BUF_MAX:
        raise vm.err(BOUNDS, f"`bf` length {n} is above the limit {BUF_MAX}",
                     op="bf", pos=pos)
    vm.push(Buf(Alloc(n), True))


def b_bg(vm, pos) -> None:
    idx = _plain_int(vm, "bg", pos, vm.pop("bg", pos), "an int index")
    buf = _top_buf(vm, "bg", pos)
    data = _live(vm, buf, "bg", pos)
    if idx < 0 or idx >= len(data):
        span = "empty buf" if not data else f"buf [0..{len(data) - 1}]"
        raise vm.err(BOUNDS, f"`bg` index {idx} is outside {span}", op="bg", pos=pos)
    vm.push(data[idx])


def b_bp(vm, pos) -> None:
    byte = _plain_int(vm, "bp", pos, vm.pop("bp", pos), "an int byte")
    idx = _plain_int(vm, "bp", pos, vm.pop("bp", pos), "an int index")
    buf = _top_buf(vm, "bp", pos)
    if not buf.exclusive:
        raise vm.err(TYPE_MISMATCH, "`bp` needs the exclusive buf, got a borrow",
                     op="bp", pos=pos, hint="write through the owner, not a borrow from `=` or a bare name")
    data = _live(vm, buf, "bp", pos)
    if isinstance(byte, int) and not 0 <= byte <= 255:
        raise vm.err(BOUNDS, f"`bp` byte {byte} is outside 0..255", op="bp", pos=pos)
    if idx < 0 or idx >= len(data):
        span = "empty buf" if not data else f"buf [0..{len(data) - 1}]"
        raise vm.err(BOUNDS, f"`bp` index {idx} is outside {span}", op="bp", pos=pos)
    data[idx] = byte
