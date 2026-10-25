"""Typed layout tree (SPEC §25).

A color is three channels. A text or button is a leaf. `rw` and `cm` run a
block on a fresh stack and lay the nodes it leaves out, start-aligned, with
padding and a gap. `ly` pushes a preorder dump. `zown draw` paints boxes as
rectangles and leaves as 5×7 glyphs. `pk` and `ky` run a button handler.
`an` adds a tick count to a node's width. `gp` fills a rectangle from a
shader into a `buf`.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .errors import BOUNDS, OVERFLOW, TYPE_MISMATCH


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
    handler: object | None = None


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
    button = kind in ("bt", "bh")
    if button:
        w = n * size + size * 2
        h = size * 2
    else:
        w = n * size
        h = size
    node = UiNode("bt" if button else "tx", text, color, 0, 0, w, h)
    vm.push(node)


def b_tx(vm, pos) -> None:
    _text_node(vm, "tx", pos)


def b_bt(vm, pos) -> None:
    _text_node(vm, "bt", pos)


def b_bh(vm, pos) -> None:
    blk = vm.pop_block("bh", pos)
    _text_node(vm, "bh", pos)
    vm.stack[-1].handler = blk


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


# 5×7 glyphs. Bit 4 is the left column. A missing glyph is blank.
_GLYPHS = {
    " ": (0, 0, 0, 0, 0, 0, 0),
    "H": (0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001),
    "i": (0b00100, 0, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100),
    "G": (0b01111, 0b10000, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111),
    "o": (0, 0, 0b01110, 0b10001, 0b10001, 0b10001, 0b01110),
}


def glyph_on(ch: str, x: int, y: int, size: int) -> bool:
    rows = _GLYPHS.get(ch)
    if rows is None or size < 1 or x < 0 or y < 0 or x >= size or y >= size:
        return False
    col = x * 5 // size
    row = y * 7 // size
    if col > 4 or row > 6:
        return False
    return (rows[row] >> (4 - col)) & 1 == 1


def _ink(c: Color) -> Color:
    if lum(c.r, c.g, c.b) >= 1280:
        return Color(0, 0, 0)
    return Color(255, 255, 255)


def _hit(node: UiNode, ox: int, oy: int, x: int, y: int) -> UiNode | None:
    """Deepest button whose rectangle contains the point."""
    for ch in node.children:
        found = _hit(ch, ox + ch.x, oy + ch.y, x, y)
        if found is not None:
            return found
    if node.kind == "bt" and ox <= x < ox + node.w and oy <= y < oy + node.h:
        return node
    return None


def _labeled(node: UiNode, key: str) -> UiNode | None:
    for ch in node.children:
        found = _labeled(ch, key)
        if found is not None:
            return found
    if node.kind == "bt" and node.text == key:
        return node
    return None


def b_pk(vm, pos) -> None:
    y = _int(vm, "pk", pos)
    x = _int(vm, "pk", pos)
    node = vm.pop("pk", pos)
    from .vm import type_name
    if not isinstance(node, UiNode):
        raise vm.err(TYPE_MISMATCH, f"`pk` expected a node, got {type_name(node)}",
                     op="pk", pos=pos)
    hit = _hit(node, 0, 0, x, y)
    if hit is not None and hit.handler is not None:
        vm.invoke(hit.handler, "pk", pos)


def b_ky(vm, pos) -> None:
    key = vm.pop("ky", pos)
    node = vm.pop("ky", pos)
    from .vm import type_name
    if not isinstance(key, str) or "\n" in key:
        got = "a string with a newline" if isinstance(key, str) else type_name(key)
        raise vm.err(TYPE_MISMATCH, f"`ky` expected one line of text, got {got}",
                     op="ky", pos=pos)
    if not isinstance(node, UiNode):
        raise vm.err(TYPE_MISMATCH, f"`ky` expected a node, got {type_name(node)}",
                     op="ky", pos=pos)
    hit = _labeled(node, key)
    if hit is not None and hit.handler is not None:
        vm.invoke(hit.handler, "ky", pos)


def b_an(vm, pos) -> None:
    n = _int(vm, "an", pos)
    if n < 0:
        raise vm.err(BOUNDS, "`an` tick count must be >= 0", op="an", pos=pos)
    node = vm.pop("an", pos)
    from .vm import type_name
    if not isinstance(node, UiNode):
        raise vm.err(TYPE_MISMATCH, f"`an` expected a node, got {type_name(node)}",
                     op="an", pos=pos)
    from .lexer import INT_MAX
    if node.w > INT_MAX - n:
        raise vm.err(OVERFLOW, "`an` width does not fit int", op="an", pos=pos)
    node.w += n
    vm.push(node)


def b_gp(vm, pos) -> None:
    from .buf import BUF_MAX, Alloc, Buf
    blk = vm.pop_block("gp", pos)
    h = _int(vm, "gp", pos)
    w = _int(vm, "gp", pos)
    if w < 1 or h < 1:
        raise vm.err(BOUNDS, "`gp` width and height must be >= 1", op="gp", pos=pos)
    if w * h > BUF_MAX // 3:
        raise vm.err(BOUNDS, "`gp` picture is larger than the buffer budget",
                     op="gp", pos=pos)
    saved = vm.stack
    vm.stack = []
    try:
        vm.invoke(blk, "gp", pos)
        left = list(vm.stack)
    finally:
        vm.stack = saved
    if len(left) != 1 or not isinstance(left[0], Color):
        raise vm.err(TYPE_MISMATCH, "`gp` shader must leave one color",
                     op="gp", pos=pos, hint="the block runs on a fresh stack and leaves r g b co")
    color = left[0]
    alloc = Alloc(w * h * 3)
    data = alloc.data
    assert data is not None
    for i in range(0, len(data), 3):
        data[i] = color.r
        data[i + 1] = color.g
        data[i + 2] = color.b
    vm.push(Buf(alloc, True))


def _glyphs(fill, n: UiNode, ox: int, oy: int, color: Color) -> None:
    size = n.h if n.kind == "tx" else n.h // 2
    if size < 1:
        return
    for i, ch in enumerate(n.text):
        gx = ox + i * size
        for y in range(size):
            for x in range(size):
                if glyph_on(ch, x, y, size):
                    fill(gx + x, oy + y, 1, 1, color)


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
        elif n.kind == "bt":
            fill(ox, oy, n.w, n.h, n.color)
            _glyphs(fill, n, ox + n.h // 2, oy + n.h // 4, _ink(n.color))
        else:
            _glyphs(fill, n, ox, oy, n.color)

    paint(node, 0, 0)
    header = f"P6\n{w} {h}\n255\n".encode("ascii")
    return header + bytes(pix)
