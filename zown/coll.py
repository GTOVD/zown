"""Collections on a fat pointer (SPEC §21).

A collection is not a block and not a raw `buf` the program can `bg`. It has
the same ownership as a `buf`: exclusive, borrow, and free. The element budget
is `BUF_MAX`. Elements are an `int` or a string with no newline. Map, tree,
and trie keys are strings with no whitespace.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .buf import BUF_MAX
from .errors import BOUNDS, TYPE_MISMATCH

KINDS = ("ls", "mp", "sc", "qu", "rg", "te", "ti", "bm", "pq", "bv")
PUSH_KINDS = ("ls", "qu", "rg", "pq", "sc", "bm")
POP_KINDS = ("ls", "qu", "rg", "pq")


@dataclass
class BNode:
    leaf: bool = True
    keys: list = field(default_factory=list)
    kids: list = field(default_factory=list)


@dataclass
class TNode:
    kids: dict = field(default_factory=dict)
    end: bool = False


class Store:
    def __init__(self, kind: str) -> None:
        self.kind = kind
        self.freed = False
        self.items: list = []
        self.order: list = []
        self.pairs: dict = {}
        self.root = BNode()
        self.trie = TNode()
        self.trie_n = 0
        self.cap = 0
        self.bits = bytearray()
        self.nbits = 0
        self.seq = 0
        self.heap: list = []

    def count(self) -> int:
        if self.kind in ("ls", "qu", "rg"):
            return len(self.items)
        if self.kind == "sc":
            return len(self.order)
        if self.kind == "mp":
            return len(self.order)
        if self.kind == "te":
            return _tree_count(self.root)
        if self.kind == "ti":
            return self.trie_n
        if self.kind == "pq":
            return len(self.heap)
        return self.nbits


class Coll:
    def __init__(self, store: Store, exclusive: bool) -> None:
        self.store = store
        self.exclusive = exclusive

    def borrow(self) -> Coll:
        return Coll(self.store, False)

    def text(self) -> str:
        body = "-" if self.store.freed else str(self.store.count())
        mark = "" if self.exclusive else "&"
        return f"{self.store.kind}{mark}({body})"

    def __repr__(self) -> str:
        return self.text()

    def __eq__(self, other: object) -> bool:
        return (
            isinstance(other, Coll)
            and self.store is other.store
            and self.exclusive == other.exclusive
        )


def release_coll(v) -> None:
    if isinstance(v, Coll) and v.exclusive:
        v.store.freed = True


def _err_type(vm, op, pos, msg):
    raise vm.err(TYPE_MISMATCH, msg, op=op, pos=pos)


def _live(vm, coll: Coll, op: str, pos):
    if coll.store.freed:
        raise vm.err(BOUNDS, f"`{op}` collection was freed", op=op, pos=pos,
                     hint="the exclusive owner was dropped with `,`")
    return coll.store


def _need_excl(vm, coll: Coll, op: str, pos) -> None:
    if not coll.exclusive:
        _err_type(vm, op, pos, f"`{op}` needs the exclusive collection, got a borrow")


def _top(vm, op: str, pos) -> Coll:
    if not vm.stack:
        raise vm.err(BOUNDS, f"`{op}` needs a collection under the operand", op=op, pos=pos)
    coll = vm.stack[-1]
    if not isinstance(coll, Coll):
        from .vm import type_name
        _err_type(vm, op, pos, f"`{op}` expected a collection, got {type_name(coll)}")
    return coll


def _elem(vm, op: str, pos, v):
    if isinstance(v, bool) or not isinstance(v, (int, str)):
        from .vm import type_name
        _err_type(vm, op, pos, f"`{op}` expected an int or a string, got {type_name(v)}")
    if isinstance(v, str) and "\n" in v:
        _err_type(vm, op, pos, f"`{op}` expected one line of text")
    return v


def _key(vm, op: str, pos, v) -> str:
    if not isinstance(v, str) or v == "" or any(ch.isspace() for ch in v):
        from .vm import type_name
        got = "a string with whitespace" if isinstance(v, str) else type_name(v)
        _err_type(vm, op, pos, f"`{op}` expected one word, got {got}")
    return v


def _plain(vm, op: str, pos, v) -> int:
    if isinstance(v, bool) or not isinstance(v, int):
        from .vm import type_name
        _err_type(vm, op, pos, f"`{op}` expected an int, got {type_name(v)}")
    return v


def _room(vm, op: str, pos, n: int) -> None:
    if n > BUF_MAX:
        raise vm.err(BOUNDS, f"`{op}` collection is above the limit {BUF_MAX}",
                     op=op, pos=pos)


def _show(v) -> str:
    return str(v)


def _scan_lines(items) -> str:
    return "\n".join(_show(v) for v in items)


# --- list, queue, ring --------------------------------------------------------
def b_ls(vm, pos) -> None:
    vm.push(Coll(Store("ls"), True))


def b_qu(vm, pos) -> None:
    vm.push(Coll(Store("qu"), True))


def b_rg(vm, pos) -> None:
    n = _plain(vm, "rg", pos, vm.pop("rg", pos))
    if n < 1:
        raise vm.err(BOUNDS, "`rg` capacity must be >= 1", op="rg", pos=pos)
    _room(vm, "rg", pos, n)
    store = Store("rg")
    store.cap = n
    vm.push(Coll(store, True))


def b_ps(vm, pos) -> None:
    v = vm.pop("ps", pos)
    coll = _top(vm, "ps", pos)
    if coll.store.kind not in PUSH_KINDS:
        _err_type(vm, "ps", pos, f"`ps` cannot push onto a {coll.store.kind}")
    _need_excl(vm, coll, "ps", pos)
    store = _live(vm, coll, "ps", pos)
    if store.kind == "bm":
        key = _key(vm, "ps", pos, v)
        bit = _fnv(key) % store.nbits
        store.bits[bit // 8] |= 1 << (bit % 8)
        return
    if store.kind == "pq":
        n = _plain(vm, "ps", pos, v)
        _room(vm, "ps", pos, len(store.heap) + 1)
        _heap_push(store, n)
        return
    if store.kind == "sc":
        el = _elem(vm, "ps", pos, v)
        tag = ("i", el) if isinstance(el, int) else ("s", el)
        if tag not in store.pairs:
            _room(vm, "ps", pos, len(store.order) + 1)
            store.pairs[tag] = True
            store.order.append(tag)
        return
    el = _elem(vm, "ps", pos, v)
    if store.kind == "rg":
        store.items.append(el)
        if len(store.items) > store.cap:
            store.items.pop(0)
        return
    _room(vm, "ps", pos, len(store.items) + 1)
    store.items.append(el)


def b_po(vm, pos) -> None:
    coll = _top(vm, "po", pos)
    if coll.store.kind not in POP_KINDS:
        _err_type(vm, "po", pos, f"`po` cannot pop a {coll.store.kind}")
    _need_excl(vm, coll, "po", pos)
    store = _live(vm, coll, "po", pos)
    if store.kind == "pq":
        if not store.heap:
            raise vm.err(BOUNDS, "`po` priority queue is empty", op="po", pos=pos)
        vm.push(_heap_pop(store))
        return
    if not store.items:
        raise vm.err(BOUNDS, f"`po` {store.kind} is empty", op="po", pos=pos)
    if store.kind == "ls":
        vm.push(store.items.pop())
    else:
        vm.push(store.items.pop(0))


def b_gt(vm, pos) -> None:
    key = vm.pop("gt", pos)
    coll = _top(vm, "gt", pos)
    store = _live(vm, coll, "gt", pos)
    kind = store.kind
    if kind == "ls":
        idx = _plain(vm, "gt", pos, key)
        if idx < 0 or idx >= len(store.items):
            span = "empty list" if not store.items else f"list [0..{len(store.items) - 1}]"
            raise vm.err(BOUNDS, f"`gt` index {idx} is outside {span}", op="gt", pos=pos)
        vm.push(store.items[idx])
        return
    if kind == "bv":
        idx = _plain(vm, "gt", pos, key)
        if idx < 0 or idx >= store.nbits:
            raise vm.err(BOUNDS, f"`gt` index {idx} is outside bits [0..{store.nbits - 1}]",
                         op="gt", pos=pos)
        vm.push((store.bits[idx // 8] >> (idx % 8)) & 1)
        return
    if kind in ("mp", "te"):
        name = _key(vm, "gt", pos, key)
        if kind == "mp":
            if name not in store.pairs:
                raise vm.err(BOUNDS, f"`gt` key {name} is absent", op="gt", pos=pos)
            vm.push(store.pairs[name])
            return
        found = _tree_get(store.root, name)
        if found is None:
            raise vm.err(BOUNDS, f"`gt` key {name} is absent", op="gt", pos=pos)
        vm.push(found)
        return
    _err_type(vm, "gt", pos, f"`gt` cannot index a {kind}")


def b_pt(vm, pos) -> None:
    top = vm.pop("pt", pos)
    if not vm.stack:
        raise vm.err(BOUNDS, "`pt` needs a collection under the operand", op="pt", pos=pos)
    if isinstance(vm.stack[-1], Coll) and vm.stack[-1].store.kind == "ti":
        coll = vm.stack[-1]
        _need_excl(vm, coll, "pt", pos)
        store = _live(vm, coll, "pt", pos)
        key = _key(vm, "pt", pos, top)
        if not _trie_has(store.trie, key):
            _room(vm, "pt", pos, store.trie_n + 1)
            _trie_put(store.trie, key)
            store.trie_n += 1
        return
    key = vm.pop("pt", pos)
    coll = _top(vm, "pt", pos)
    _need_excl(vm, coll, "pt", pos)
    store = _live(vm, coll, "pt", pos)
    if store.kind == "bv":
        idx = _plain(vm, "pt", pos, key)
        bit = _plain(vm, "pt", pos, top)
        if bit not in (0, 1):
            raise vm.err(BOUNDS, f"`pt` bit {bit} is not 0 or 1", op="pt", pos=pos)
        if idx < 0 or idx >= store.nbits:
            raise vm.err(BOUNDS, f"`pt` index {idx} is outside bits [0..{store.nbits - 1}]",
                         op="pt", pos=pos)
        mask = 1 << (idx % 8)
        if bit:
            store.bits[idx // 8] |= mask
        else:
            store.bits[idx // 8] &= ~mask
        return
    if store.kind == "mp":
        name = _key(vm, "pt", pos, key)
        val = _elem(vm, "pt", pos, top)
        if name not in store.pairs:
            _room(vm, "pt", pos, len(store.order) + 1)
            store.order.append(name)
        store.pairs[name] = val
        return
    if store.kind == "te":
        name = _key(vm, "pt", pos, key)
        val = _elem(vm, "pt", pos, top)
        if _tree_get(store.root, name) is None:
            _room(vm, "pt", pos, _tree_count(store.root) + 1)
        store.root = _tree_put(store.root, name, val)
        return
    _err_type(vm, "pt", pos, f"`pt` cannot update a {store.kind}")


def b_hs(vm, pos) -> None:
    v = vm.pop("hs", pos)
    coll = _top(vm, "hs", pos)
    store = _live(vm, coll, "hs", pos)
    if store.kind == "sc":
        el = _elem(vm, "hs", pos, v)
        tag = ("i", el) if isinstance(el, int) else ("s", el)
        vm.push(1 if tag in store.pairs else 0)
        return
    if store.kind == "ti":
        vm.push(1 if _trie_has(store.trie, _key(vm, "hs", pos, v)) else 0)
        return
    if store.kind == "bm":
        key = _key(vm, "hs", pos, v)
        bit = _fnv(key) % store.nbits
        on = (store.bits[bit // 8] >> (bit % 8)) & 1
        vm.push(on)
        return
    _err_type(vm, "hs", pos, f"`hs` cannot test a {store.kind}")


def b_sk(vm, pos) -> None:
    coll = _top(vm, "sk", pos)
    store = _live(vm, coll, "sk", pos)
    kind = store.kind
    if kind in ("ls", "qu", "rg"):
        vm.push(_scan_lines(store.items))
        return
    if kind == "sc":
        ints = sorted(v for t, v in store.order if t == "i")
        strs = sorted(v for t, v in store.order if t == "s")
        vm.push(_scan_lines(ints + strs))
        return
    if kind == "mp":
        vm.push("\n".join(f"{k} {_show(store.pairs[k])}" for k in store.order))
        return
    if kind == "te":
        rows = []
        _tree_walk(store.root, rows)
        vm.push("\n".join(f"{k} {_show(v)}" for k, v in rows))
        return
    if kind == "ti":
        rows: list[str] = []
        _trie_walk(store.trie, "", rows)
        vm.push("\n".join(rows))
        return
    if kind == "pq":
        heap = list(store.heap)
        out = []
        while heap:
            out.append(_heap_pop_list(heap))
        vm.push(_scan_lines(out))
        return
    if kind == "bv":
        chars = []
        for i in range(store.nbits):
            chars.append("1" if (store.bits[i // 8] >> (i % 8)) & 1 else "0")
        vm.push("".join(chars))
        return
    _err_type(vm, "sk", pos, f"`sk` cannot scan a {kind}")


def b_px(vm, pos) -> None:
    prefix = vm.pop("px", pos)
    coll = _top(vm, "px", pos)
    store = _live(vm, coll, "px", pos)
    if store.kind != "ti":
        _err_type(vm, "px", pos, f"`px` expected a trie, got {store.kind}")
    if not isinstance(prefix, str) or any(ch.isspace() for ch in prefix):
        from .vm import type_name
        got = "a string with whitespace" if isinstance(prefix, str) else type_name(prefix)
        _err_type(vm, "px", pos, f"`px` expected one word, got {got}")
    node = store.trie
    for ch in prefix:
        node = node.kids.get(ch)
        if node is None:
            vm.push("")
            return
    rows: list[str] = []
    _trie_walk(node, prefix, rows)
    vm.push("\n".join(rows))


def b_mp(vm, pos) -> None:
    vm.push(Coll(Store("mp"), True))


def b_sc(vm, pos) -> None:
    vm.push(Coll(Store("sc"), True))


def b_te(vm, pos) -> None:
    vm.push(Coll(Store("te"), True))


def b_ti(vm, pos) -> None:
    vm.push(Coll(Store("ti"), True))


def b_pq(vm, pos) -> None:
    vm.push(Coll(Store("pq"), True))


def b_bm(vm, pos) -> None:
    n = _plain(vm, "bm", pos, vm.pop("bm", pos))
    if n < 1:
        raise vm.err(BOUNDS, "`bm` bit count must be >= 1", op="bm", pos=pos)
    _room(vm, "bm", pos, n)
    store = Store("bm")
    store.nbits = n
    store.bits = bytearray((n + 7) // 8)
    vm.push(Coll(store, True))


def b_bv(vm, pos) -> None:
    n = _plain(vm, "bv", pos, vm.pop("bv", pos))
    if n < 1:
        raise vm.err(BOUNDS, "`bv` bit count must be >= 1", op="bv", pos=pos)
    _room(vm, "bv", pos, n)
    store = Store("bv")
    store.nbits = n
    store.bits = bytearray((n + 7) // 8)
    vm.push(Coll(store, True))


def coll_len(coll: Coll) -> int:
    if coll.store.freed:
        raise ValueError("freed")
    return coll.store.count()


# --- bloom hash: FNV-1a, 32-bit, one bit --------------------------------------
def _fnv(text: str) -> int:
    h = 2166136261
    for byte in text.encode("utf-8"):
        h ^= byte
        h = (h * 16777619) & 0xFFFFFFFF
    return h


# --- heap (min, then earlier insertion) ---------------------------------------
def _heap_push(store: Store, n: int) -> None:
    store.seq += 1
    store.heap.append((n, store.seq))
    i = len(store.heap) - 1
    while i > 0:
        p = (i - 1) // 2
        if store.heap[p] <= store.heap[i]:
            break
        store.heap[p], store.heap[i] = store.heap[i], store.heap[p]
        i = p


def _heap_pop(store: Store) -> int:
    return _heap_pop_list(store.heap)


def _heap_pop_list(heap: list) -> int:
    top = heap[0][0]
    last = heap.pop()
    if heap:
        heap[0] = last
        i = 0
        n = len(heap)
        while True:
            left = i * 2 + 1
            right = left + 1
            small = i
            if left < n and heap[left] < heap[small]:
                small = left
            if right < n and heap[right] < heap[small]:
                small = right
            if small == i:
                break
            heap[i], heap[small] = heap[small], heap[i]
            i = small
    return top


# --- trie ----------------------------------------------------------------------
def _trie_put(node: TNode, key: str) -> None:
    for ch in key:
        node = node.kids.setdefault(ch, TNode())
    node.end = True


def _trie_has(node: TNode, key: str) -> bool:
    for ch in key:
        nxt = node.kids.get(ch)
        if nxt is None:
            return False
        node = nxt
    return node.end


def _trie_walk(node: TNode, prefix: str, out: list) -> None:
    if node.end:
        out.append(prefix)
    for ch in sorted(node.kids):
        _trie_walk(node.kids[ch], prefix + ch, out)


# --- B-tree, minimum degree 2 (at most 3 keys) --------------------------------
def _tree_count(node: BNode) -> int:
    n = len(node.keys)
    for kid in node.kids:
        n += _tree_count(kid)
    return n


def _tree_get(node: BNode, key: str):
    i = 0
    while i < len(node.keys) and key > node.keys[i][0]:
        i += 1
    if i < len(node.keys) and node.keys[i][0] == key:
        return node.keys[i][1]
    if node.leaf:
        return None
    return _tree_get(node.kids[i], key)


def _tree_walk(node: BNode, out: list) -> None:
    if node.leaf:
        out.extend(node.keys)
        return
    for i, kv in enumerate(node.keys):
        _tree_walk(node.kids[i], out)
        out.append(kv)
    _tree_walk(node.kids[-1], out)


def _split_child(parent: BNode, i: int) -> None:
    y = parent.kids[i]
    z = BNode(leaf=y.leaf)
    median = y.keys[1]
    z.keys = y.keys[2:]
    y.keys = y.keys[:1]
    if not y.leaf:
        z.kids = y.kids[2:]
        y.kids = y.kids[:2]
    parent.kids.insert(i + 1, z)
    parent.keys.insert(i, median)


def _insert_nonfull(node: BNode, key: str, val) -> None:
    if node.leaf:
        for i, (k, _) in enumerate(node.keys):
            if k == key:
                node.keys[i] = (key, val)
                return
        node.keys.append((key, val))
        node.keys.sort(key=lambda kv: kv[0])
        return
    i = len(node.keys) - 1
    while i >= 0 and key < node.keys[i][0]:
        i -= 1
    if i >= 0 and node.keys[i][0] == key:
        node.keys[i] = (key, val)
        return
    i += 1
    if len(node.kids[i].keys) == 3:
        _split_child(node, i)
        if key == node.keys[i][0]:
            node.keys[i] = (key, val)
            return
        if key > node.keys[i][0]:
            i += 1
    _insert_nonfull(node.kids[i], key, val)


def _tree_put(root: BNode, key: str, val) -> BNode:
    if len(root.keys) == 3:
        fresh = BNode(leaf=False)
        fresh.kids.append(root)
        _split_child(fresh, 0)
        _insert_nonfull(fresh, key, val)
        return fresh
    _insert_nonfull(root, key, val)
    return root
