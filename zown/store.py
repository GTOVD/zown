"""Graph, BLAKE3 bundle, Ed25519 trailer, and transparency log (SPEC §26).

`sb` seals a snapshot. `sl` adds an Ed25519 trailer. `la` appends the sealed
body's digest to this VM's log, and `lp` writes an inclusion proof for it.
`so` checks the hash, then the signature if one is present, then the proof
if one is present. A BLAKE3-only bundle still opens. CRDTs are not this log.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .blake3 import blake3, blake3_hex
from .errors import BOUNDS, INTEGRITY_FAIL, SIG_INVALID, TYPE_MISMATCH

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


def body_of(graph: Graph) -> str:
    return HEADER + graph.snapshot()


def seal(graph: Graph) -> str:
    body = body_of(graph)
    return body + "blake3 " + blake3_hex(body.encode("utf-8")) + "\n"


def _digest(body: str) -> bytes:
    return blake3(body.encode("utf-8"))


def _k_below(n: int) -> int:
    """Largest power of two strictly less than n. n is at least 2."""
    k = 1
    while (k << 1) < n:
        k <<= 1
    return k


def _leaf(digest: bytes) -> bytes:
    return blake3(b"\x00" + digest)


def _node(left: bytes, right: bytes) -> bytes:
    return blake3(b"\x01" + left + right)


def _mth(leaves: list[bytes]) -> bytes:
    n = len(leaves)
    if n == 1:
        return _leaf(leaves[0])
    k = _k_below(n)
    return _node(_mth(leaves[:k]), _mth(leaves[k:]))


def log_root(leaves: list[bytes]) -> bytes | None:
    if not leaves:
        return None
    return _mth(leaves)


def _path(leaves: list[bytes], index: int) -> list[bytes]:
    n = len(leaves)
    if n == 1:
        return []
    k = _k_below(n)
    if index < k:
        return _path(leaves[:k], index) + [_mth(leaves[k:])]
    return _path(leaves[k:], index - k) + [_mth(leaves[:k])]


def _fold(digest: bytes, index: int, size: int, path: list[bytes]) -> tuple[bytes, list[bytes]] | None:
    if size == 1:
        if index != 0:
            return None
        return _leaf(digest), path
    k = _k_below(size)
    if index < k:
        inner = _fold(digest, index, k, path)
        if inner is None or not inner[1]:
            return None
        acc, path = inner
        return _node(acc, path[0]), path[1:]
    inner = _fold(digest, index - k, size - k, path)
    if inner is None or not inner[1]:
        return None
    acc, path = inner
    return _node(path[0], acc), path[1:]


def proof_ok(digest: bytes, index: int, size: int, path: list[bytes], root: bytes) -> bool:
    if size <= 0 or index < 0 or index >= size:
        return False
    folded = _fold(digest, index, size, path)
    return folded is not None and folded[1] == [] and folded[0] == root


def _hex64(text: str) -> bool:
    return len(text) == 64 and all(c in "0123456789abcdefABCDEF" for c in text)


def _hex_n(text: str, n: int) -> bool:
    return len(text) == n and all(c in "0123456789abcdefABCDEF" for c in text)


def _dec(text: str) -> int | None:
    if not text.isdigit() or (len(text) > 1 and text[0] == "0"):
        return None
    return int(text)


def _shape(vm, pos):
    raise vm.err(TYPE_MISMATCH, "`so` expected a zown-sync bundle",
                 op="so", pos=pos, hint="seal the graph with sb")


def open_bundle(vm, pos, text: str) -> Graph:
    if not text.endswith("\n"):
        _shape(vm, pos)
    lines = text[:-1].split("\n")
    idx = None
    for i, line in enumerate(lines):
        if line.startswith("blake3 "):
            idx = i
    if idx is None:
        _shape(vm, pos)
    body = "\n".join(lines[:idx]) + "\n"
    claimed = lines[idx][len("blake3 "):]
    if not _hex64(claimed):
        raise vm.err(TYPE_MISMATCH, "`so` expected a blake3 trailer",
                     op="so", pos=pos)
    digest = _digest(body)
    if blake3_hex(body.encode("utf-8")) != claimed.lower():
        raise vm.err(INTEGRITY_FAIL, "`so` bundle hash does not match its bytes",
                     op="so", pos=pos, kind="sec",
                     hint="the bundle was changed after it was sealed")
    if not body.startswith(HEADER):
        raise vm.err(TYPE_MISMATCH, "`so` expected a zown-sync 1 header",
                     op="so", pos=pos)
    rest = lines[idx + 1:]
    signed = None
    included = None
    if rest and rest[0].startswith("ed25519 "):
        signed = rest.pop(0)
    if rest and rest[0].startswith("include "):
        included = rest.pop(0)
    if rest:
        raise vm.err(TYPE_MISMATCH, "`so` expected only an ed25519 line and an include line after blake3",
                     op="so", pos=pos)
    if signed is not None:
        _check_sig(vm, pos, body, signed)
    if included is not None:
        _check_proof(vm, pos, digest, included)
    return _parse_snapshot(vm, "so", pos, body[len(HEADER):])


def _check_sig(vm, pos, body: str, line: str) -> None:
    parts = line.split(" ")
    if len(parts) != 3 or not _hex_n(parts[1], 64) or not _hex_n(parts[2], 128):
        raise vm.err(TYPE_MISMATCH, "`so` expected an ed25519 public key and signature",
                     op="so", pos=pos)
    from .edwards import verify
    pub = bytes.fromhex(parts[1])
    sig = bytes.fromhex(parts[2])
    if not verify(pub, body.encode("utf-8"), sig):
        raise vm.err(SIG_INVALID, "`so` signature is not valid",
                     op="so", pos=pos, kind="sec",
                     hint="the signature, bundle, or public key does not match")


def _check_proof(vm, pos, digest: bytes, line: str) -> None:
    parts = line.split(" ")
    if len(parts) < 3 or parts[0] != "include":
        raise vm.err(TYPE_MISMATCH, "`so` expected an include proof",
                     op="so", pos=pos)
    index = _dec(parts[1])
    size = _dec(parts[2])
    if index is None or size is None:
        raise vm.err(TYPE_MISMATCH, "`so` expected an include index and tree size",
                     op="so", pos=pos)
    path: list[bytes] = []
    for token in parts[3:]:
        if not _hex64(token):
            raise vm.err(TYPE_MISMATCH, "`so` expected include siblings as 64 hex digits",
                         op="so", pos=pos)
        path.append(bytes.fromhex(token))
    if size < 1 or index >= size:
        raise vm.err(TYPE_MISMATCH, "`so` expected an include index inside the tree",
                     op="so", pos=pos)
    root = log_root(vm.log)
    if root is None or size != len(vm.log) or not proof_ok(digest, index, size, path, root):
        raise vm.err(INTEGRITY_FAIL, "`so` inclusion proof does not match the log",
                     op="so", pos=pos, kind="sec",
                     hint="the proof, the leaf, or this VM's log does not agree")


def b_sl(vm, pos) -> None:
    from .buf import Buf, release
    from .edwards import public_key, sign
    from .vm import type_name
    seed_v = vm.pop("sl", pos)
    if not isinstance(seed_v, Buf):
        raise vm.err(TYPE_MISMATCH, f"`sl` expected a buf, got {type_name(seed_v)}",
                     op="sl", pos=pos, hint="the seed is a buf of 32 bytes")
    if seed_v.alloc.data is None:
        raise vm.err(BOUNDS, "`sl` buffer was freed",
                     op="sl", pos=pos, hint="the exclusive owner was dropped with `,`")
    seed = bytes(seed_v.alloc.data)
    release(seed_v)
    if len(seed) != 32:
        raise vm.err(TYPE_MISMATCH, "`sl` expected a seed of 32 bytes",
                     op="sl", pos=pos)
    body = body_of(vm.graph)
    raw = body.encode("utf-8")
    sig = sign(seed, raw)
    pub = public_key(seed)
    vm.push(seal(vm.graph) + f"ed25519 {pub.hex()} {sig.hex()}\n")


def b_la(vm, pos) -> None:
    vm.log.append(_digest(body_of(vm.graph)))
    vm.push(len(vm.log))


def b_lp(vm, pos) -> None:
    digest = _digest(body_of(vm.graph))
    try:
        index = vm.log.index(digest)
    except ValueError:
        raise vm.err(BOUNDS, "`lp` has no log entry for this bundle",
                     op="lp", pos=pos, hint="append it with la first") from None
    siblings = _path(vm.log, index)
    line = f"include {index} {len(vm.log)}"
    if siblings:
        line += " " + " ".join(item.hex() for item in siblings)
    vm.push(line + "\n")


def b_lr(vm, pos) -> None:
    root = log_root(vm.log)
    vm.push("" if root is None else root.hex())


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
