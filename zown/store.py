"""Append-only graph log and a BLAKE3-sealed bundle (SPEC §26).

The durable form is the bundle text, not a memory map. `sb` seals the
snapshot; `so` refuses it when the digest does not match. Ed25519 signatures
and CRDTs are not this log.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .blake3 import blake3_hex
from .errors import INTEGRITY_FAIL, TYPE_MISMATCH

HEADER = "zown-sync 1\n"


@dataclass
class Cursor:
    names: list[str] = field(default_factory=list)


@dataclass
class Graph:
    verts: list[str] = field(default_factory=list)
    edges: list[tuple[str, str, str]] = field(default_factory=list)

    def add_vert(self, name: str) -> None:
        if name not in self.verts:
            self.verts.append(name)

    def add_edge(self, src: str, label: str, dst: str) -> None:
        self.add_vert(src)
        self.add_vert(dst)
        edge = (src, label, dst)
        if edge not in self.edges:
            self.edges.append(edge)

    def snapshot(self) -> str:
        lines = [f"v {name}" for name in self.verts]
        lines.extend(f"e {src} {label} {dst}" for src, label, dst in self.edges)
        if not lines:
            return ""
        return "\n".join(lines) + "\n"

    def head(self) -> str:
        return blake3_hex(self.snapshot().encode("utf-8"))

    def follow(self, names: list[str], label: str) -> list[str]:
        out: list[str] = []
        seen: set[str] = set()
        for src in names:
            for frm, lab, dst in self.edges:
                if frm == src and lab == label and dst not in seen:
                    seen.add(dst)
                    out.append(dst)
        return out


def _name(vm, op: str, pos, v) -> str:
    from .vm import type_name
    if not isinstance(v, str) or v == "" or any(ch.isspace() for ch in v):
        got = type_name(v) if not isinstance(v, str) else "a name with whitespace"
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a single-word name, got {got}",
                     op=op, pos=pos, hint="names in the graph are one word, with no spaces")
    return v


def b_gv(vm, pos) -> None:
    name = _name(vm, "gv", pos, vm.pop("gv", pos))
    vm.graph.add_vert(name)


def b_ge(vm, pos) -> None:
    dst = _name(vm, "ge", pos, vm.pop("ge", pos))
    label = _name(vm, "ge", pos, vm.pop("ge", pos))
    src = _name(vm, "ge", pos, vm.pop("ge", pos))
    vm.graph.add_edge(src, label, dst)


def b_gs(vm, pos) -> None:
    vm.push(vm.graph.snapshot())


def b_gl(vm, pos) -> None:
    text = vm.pop("gl", pos)
    if not isinstance(text, str):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`gl` expected a snapshot string, got {type_name(text)}",
                     op="gl", pos=pos)
    vm.graph = _parse_snapshot(vm, "gl", pos, text)


def b_gh(vm, pos) -> None:
    vm.push(vm.graph.head())


def b_sb(vm, pos) -> None:
    vm.push(seal(vm.graph))


def b_so(vm, pos) -> None:
    text = vm.pop("so", pos)
    if not isinstance(text, str):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`so` expected a bundle string, got {type_name(text)}",
                     op="so", pos=pos)
    vm.graph = open_bundle(vm, pos, text)
    vm.push(vm.graph.head())


def op_open(vm, pos) -> None:
    name = _name(vm, "»", pos, vm.pop("»", pos))
    names = [name] if name in vm.graph.verts else []
    vm.push(Cursor(names))


def op_follow(vm, pos) -> None:
    label = _name(vm, "→", pos, vm.pop("→", pos))
    cur = vm.pop("→", pos)
    if not isinstance(cur, Cursor):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`→` expected a cursor, got {type_name(cur)}",
                     op="→", pos=pos, hint="open a query with »")
    vm.push(Cursor(vm.graph.follow(cur.names, label)))


def op_close(vm, pos) -> None:
    cur = vm.pop("«", pos)
    if not isinstance(cur, Cursor):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`«` expected a cursor, got {type_name(cur)}",
                     op="«", pos=pos)
    vm.push(" ".join(cur.names))


def seal(graph: Graph) -> str:
    body = HEADER + graph.snapshot()
    return body + "blake3 " + blake3_hex(body.encode("utf-8")) + "\n"


def open_bundle(vm, pos, text: str) -> Graph:
    marker = "\nblake3 "
    idx = text.rfind(marker)
    if idx < 0 or not text.endswith("\n"):
        raise vm.err(TYPE_MISMATCH, "`so` expected a zown-sync bundle",
                     op="so", pos=pos, hint="seal the graph with sb")
    body = text[:idx + 1]
    claimed = text[idx + len(marker):-1]
    if len(claimed) != 64 or any(c not in "0123456789abcdef" for c in claimed.lower()):
        raise vm.err(TYPE_MISMATCH, "`so` expected a blake3 trailer",
                     op="so", pos=pos)
    if blake3_hex(body.encode("utf-8")) != claimed.lower():
        raise vm.err(INTEGRITY_FAIL, "`so` bundle hash does not match its bytes",
                     op="so", pos=pos, kind="sec",
                     hint="the bundle was changed after it was sealed")
    if not body.startswith(HEADER):
        raise vm.err(TYPE_MISMATCH, "`so` expected a zown-sync 1 header",
                     op="so", pos=pos)
    return _parse_snapshot(vm, "so", pos, body[len(HEADER):])


def _parse_snapshot(vm, op: str, pos, text: str) -> Graph:
    graph = Graph()
    if text == "":
        return graph
    if not text.endswith("\n"):
        raise vm.err(TYPE_MISMATCH, f"`{op}` snapshot must end with a newline",
                     op=op, pos=pos)
    for line in text.splitlines():
        parts = line.split(" ")
        if len(parts) == 2 and parts[0] == "v":
            graph.add_vert(_name(vm, op, pos, parts[1]))
        elif len(parts) == 4 and parts[0] == "e":
            graph.add_edge(_name(vm, op, pos, parts[1]),
                           _name(vm, op, pos, parts[2]),
                           _name(vm, op, pos, parts[3]))
        else:
            raise vm.err(TYPE_MISMATCH, f"`{op}` cannot read snapshot line {line!r}",
                         op=op, pos=pos)
    return graph


def write_fsync(path: str, text: str) -> None:
    """Write `text`, then fsync the file and its directory.

    A crash after this returns still has the bytes. A crash during the write
    can leave a short file, which `so` rejects.
    """
    import os
    data = text.encode("utf-8")
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    try:
        view = memoryview(data)
        while view:
            n = os.write(fd, view)
            if n <= 0:
                raise OSError(f"write to {path} made no progress")
            view = view[n:]
        os.fsync(fd)
    finally:
        os.close(fd)
    parent = os.path.dirname(os.path.abspath(path)) or "."
    dir_fd = os.open(parent, os.O_RDONLY)
    try:
        os.fsync(dir_fd)
    finally:
        os.close(dir_fd)
