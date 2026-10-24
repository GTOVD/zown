"""Typed layout tree (SPEC §25).

A color is three channels. A text or button is a leaf. `rw` and `cm` run a
block on a fresh stack and lay the nodes it leaves out, start-aligned, with
padding and a gap. `ly` pushes a preorder dump. The picture (`zown draw`)
fills those rectangles; it is not a GPU.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .errors import BOUNDS, TYPE_MISMATCH


@dataclass
class Color:
    r: int
    g: int
    b: int


@dataclass
class UiNode:
    kind: str
    text: str
    color: Color
    x: int = 0
    y: int = 0
    w: int = 0
    h: int = 0
    children: list["UiNode"] = field(default_factory=list)


def _int(vm, op: str, pos) -> int:
    from .vm import type_name
    v = vm.pop(op, pos)
    if type(v) is not int:
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected an int, got {type_name(v)}",
                     op=op, pos=pos, hint="the count is a plain int")
    return v


def _color(vm, op: str, pos) -> Color:
    from .vm import type_name
    v = vm.pop(op, pos)
    if not isinstance(v, Color):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a color, got {type_name(v)}",
                     op=op, pos=pos, hint="build one with r g b co")
    return v


def _channel(vm, op: str, pos) -> int:
    n = _int(vm, op, pos)
    if n < 0 or n > 255:
        raise vm.err(BOUNDS, f"`{op}` channel {n} is outside 0..255",
                     op=op, pos=pos)
    return n


def b_co(vm, pos) -> None:
    b = _channel(vm, "co", pos)
    g = _channel(vm, "co", pos)
    r = _channel(vm, "co", pos)
    vm.push(Color(r, g, b))


def _text_node(vm, kind: str, pos) -> None:
    color = _color(vm, kind, pos)
    size = _int(vm, kind, pos)
    if size < 1:
        raise vm.err(BOUNDS, f"`{kind}` size must be >= 1", op=kind, pos=pos)
    text = vm.pop(kind, pos)
    if not isinstance(text, str) or "\n" in text:
        from .vm import type_name
        got = "a string with a newline" if isinstance(text, str) else type_name(text)
        raise vm.err(TYPE_MISMATCH, f"`{kind}` expected one line of text, got {got}",
                     op=kind, pos=pos)
    n = len(text)
    if kind == "bt":
        w = n * size + size * 2
        h = size * 2
    else:
        w = n * size
        h = size
    vm.push(UiNode(kind, text, color, 0, 0, w, h))


def b_tx(vm, pos) -> None:
    _text_node(vm, "tx", pos)


def b_bt(vm, pos) -> None:
    _text_node(vm, "bt", pos)


def _fresh(vm, blk, op: str, pos) -> list:
    saved = vm.stack
    vm.stack = []
    try:
        vm.invoke(blk, op, pos)
        return list(vm.stack)
    finally:
        vm.stack = saved


def _place(kind: str, children: list[UiNode], pad: int, gap: int) -> tuple[int, int]:
    if kind == "rw":
        x = pad
        max_h = 0
        for ch in children:
            ch.x = x
            ch.y = pad
            x += ch.w + gap
            if ch.h > max_h:
                max_h = ch.h
        if children:
            x -= gap
        else:
            x = pad
        return x + pad, (max_h if children else 0) + pad * 2
    y = pad
    max_w = 0
    for ch in children:
        ch.x = pad
        ch.y = y
        y += ch.h + gap
        if ch.w > max_w:
            max_w = ch.w
    if children:
        y -= gap
    else:
        y = pad
    return (max_w if children else 0) + pad * 2, y + pad


def finish_box(vm, op: str, pos, kind: str, color: Color, gap: int, pad: int, kids: list) -> None:
    from .vm import type_name
    nodes: list[UiNode] = []
    for kid in kids:
        if not isinstance(kid, UiNode):
            raise vm.err(TYPE_MISMATCH, f"`{op}` child is a {type_name(kid)}, not a node",
                         op=op, pos=pos, hint="the block must leave text, buttons, or boxes")
        nodes.append(kid)
    w, h = _place(kind, nodes, pad, gap)
    vm.push(UiNode(kind, "", color, 0, 0, w, h, nodes))


def b_box(vm, row: bool, pos) -> None:
    op = "rw" if row else "cm"
    blk = vm.pop_block(op, pos)
    pad = _int(vm, op, pos)
    gap = _int(vm, op, pos)
    if pad < 0 or gap < 0:
        raise vm.err(BOUNDS, f"`{op}` pad and gap must be >= 0", op=op, pos=pos)
    color = _color(vm, op, pos)
    kids = _fresh(vm, blk, op, pos)
    finish_box(vm, op, pos, op, color, gap, pad, kids)


def b_rw(vm, pos) -> None:
    b_box(vm, True, pos)


def b_cm(vm, pos) -> None:
    b_box(vm, False, pos)


def lum(r: int, g: int, b: int) -> int:
    """Linear Rec. 709 luma, thousandths, then divided back. Range 0..2550."""
    return (2126 * r + 7152 * g + 722 * b) // 1000


def contrast_ok(fg: Color, bg: Color) -> bool:
    hi = max(lum(fg.r, fg.g, fg.b), lum(bg.r, bg.g, bg.b))
    lo = min(lum(fg.r, fg.g, fg.b), lum(bg.r, bg.g, bg.b))
    return (hi + 128) * 10 >= (lo + 128) * 45


def dump(node: UiNode, ox: int = 0, oy: int = 0, parent: Color | None = None, depth: int = 0) -> list[str]:
    bg = parent if parent is not None else Color(0, 0, 0)
    indent = "  " * depth
    if node.kind in ("rw", "cm"):
        lines = [f"{indent}{node.kind} {ox} {oy} {node.w} {node.h}"]
        for ch in node.children:
            lines.extend(dump(ch, ox + ch.x, oy + ch.y, node.color, depth + 1))
        return lines
    mark = "ok" if contrast_ok(node.color, bg) else "low"
    return [f"{indent}{node.kind} {ox} {oy} {node.w} {node.h} {mark} {node.text}"]


def b_ly(vm, pos) -> None:
    from .vm import type_name
    v = vm.pop("ly", pos)
    if not isinstance(v, UiNode):
        raise vm.err(TYPE_MISMATCH, f"`ly` expected a node, got {type_name(v)}",
                     op="ly", pos=pos, hint="build a tree with tx, bt, rw, or cm")
    vm.push("\n".join(dump(v)))


def raster(node: UiNode) -> bytes:
    """P6 PPM of the laid-out rectangles. A zero-size root is refused by the caller."""
    w, h = node.w, node.h
    pix = bytearray(w * h * 3)

    def fill(x: int, y: int, rw: int, rh: int, c: Color) -> None:
        for yy in range(max(y, 0), min(y + rh, h)):
            row = yy * w
            for xx in range(max(x, 0), min(x + rw, w)):
                i = (row + xx) * 3
                pix[i] = c.r
                pix[i + 1] = c.g
                pix[i + 2] = c.b

    def paint(n: UiNode, ox: int, oy: int) -> None:
        if n.kind in ("rw", "cm"):
            fill(ox, oy, n.w, n.h, n.color)
            for ch in n.children:
                paint(ch, ox + ch.x, oy + ch.y)
        else:
            fill(ox, oy, n.w, n.h, n.color)

    paint(node, 0, 0)
    header = f"P6\n{w} {h}\n255\n".encode("ascii")
    return header + bytes(pix)
