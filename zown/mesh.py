"""Content-addressed bundles and a two-peer IPv6 UDP exchange.

`~p` / `~f` in the VM use the local store. `exchange` is the LAN path: two
datagram sockets on `::1`, no internet. Announce packets are also offered to
`ff02::5a77`; this host does not deliver that group, so the rendezvous at
`[::1]:port` is the path the tests run.
"""

from __future__ import annotations

import socket
import struct
import threading

from .blake3 import blake3, blake3_hex
from .errors import BOUNDS, TYPE_MISMATCH

MAGIC = b"ZN"
VERSION = 1
ANN = 1
HELLO = 2
FETCH = 3
DATA = 4
MISS = 5
GROUP = "ff02::5a77"
DEFAULT_PORT = 59770
MAX_BUNDLE = 1100


def node_id_from_material(material: str | None) -> bytes:
    if material:
        return blake3(material.encode("utf-8"))
    import secrets
    return secrets.token_bytes(32)


def ula(node_id: bytes) -> str:
    """IPv6 unique-local address: `fd00::/8` with the next 15 bytes from the id."""
    raw = bytearray(node_id[:16])
    raw[0] = 0xFD
    return ":".join(f"{(raw[i] << 8) | raw[i + 1]:04x}" for i in range(0, 16, 2))


def _pack(kind: int, node: bytes, rest: bytes = b"") -> bytes:
    return MAGIC + bytes([VERSION, kind]) + node + rest


def _parse(data: bytes) -> tuple[int, bytes, bytes] | None:
    if len(data) < 4 + 32 or data[:2] != MAGIC or data[2] != VERSION:
        return None
    return data[3], data[4:36], data[36:]


def _offer_multicast(sock: socket.socket, packet: bytes, port: int) -> None:
    """Best-effort LAN announce. A host that cannot route ff02:: is ignored."""
    try:
        hop = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
        hop.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_MULTICAST_HOPS, 1)
        hop.sendto(packet, (f"{GROUP}%lo", port))
        hop.close()
    except OSError:
        return
    del sock


def exchange(payload: str, port: int = DEFAULT_PORT) -> tuple[str, str, str]:
    """Publish `payload` on one peer and fetch it from another.

    Returns `(hash_hex, fetched_text, "ula-a ula-b")`. Raises `RuntimeError`
    when the datagrams do not arrive.
    """
    body = payload.encode("utf-8")
    if len(body) > MAX_BUNDLE:
        raise RuntimeError(f"bundle is {len(body)} bytes; the datagram path holds {MAX_BUNDLE}")
    digest = blake3(body)
    node_a = blake3(b"zown-peer-a")
    node_b = blake3(b"zown-peer-b")
    ready = threading.Event()
    err: dict[str, str] = {}
    got: dict[str, str] = {}

    def serve() -> None:
        sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
        try:
            sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            sock.bind(("::1", port))
            sock.settimeout(3)
            ready.set()
            _offer_multicast(sock, _pack(ANN, node_a), port)
            deadline_loops = 8
            while deadline_loops:
                deadline_loops -= 1
                try:
                    data, addr = sock.recvfrom(1500)
                except socket.timeout:
                    err.setdefault("e", "the serving peer heard nothing")
                    return
                parsed = _parse(data)
                if parsed is None:
                    continue
                kind, _node, rest = parsed
                if kind == ANN:
                    # The hello carries this node's content hash. The peer
                    # checks it before the fetch (the gossip step).
                    sock.sendto(_pack(HELLO, node_a, digest), addr)
                elif kind == FETCH and len(rest) >= 32:
                    want = rest[:32]
                    if want == digest:
                        header = want + len(body).to_bytes(2, "big")
                        sock.sendto(_pack(DATA, node_a, header + body), addr)
                        return
                    sock.sendto(_pack(MISS, node_a, want), addr)
        finally:
            sock.close()

    def client() -> None:
        if not ready.wait(2):
            err["e"] = "the serving peer did not bind"
            return
        sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
        try:
            sock.bind(("::1", 0))
            sock.settimeout(3)
            sock.sendto(_pack(ANN, node_b), ("::1", port))
            data, _addr = sock.recvfrom(1500)
            parsed = _parse(data)
            if parsed is None or parsed[0] != HELLO or parsed[1] != node_a or parsed[2] != digest:
                err["e"] = "discovery did not announce the bundle"
                return
            sock.sendto(_pack(FETCH, node_b, digest), ("::1", port))
            data, _addr = sock.recvfrom(1500)
            parsed = _parse(data)
            if parsed is None or parsed[0] != DATA or len(parsed[2]) < 34:
                err["e"] = "the bundle did not come back"
                return
            rest = parsed[2]
            n = int.from_bytes(rest[32:34], "big")
            blob = rest[34:34 + n]
            if blake3(blob) != digest:
                err["e"] = "the bundle hash did not match"
                return
            got["text"] = blob.decode("utf-8")
        except (socket.timeout, OSError) as exc:
            err["e"] = str(exc)
        finally:
            sock.close()

    server = threading.Thread(target=serve)
    fetcher = threading.Thread(target=client)
    server.start()
    fetcher.start()
    fetcher.join()
    server.join()
    if "text" not in got:
        raise RuntimeError(err.get("e", "the two peers did not exchange the bundle"))
    return digest.hex(), got["text"], f"{ula(node_a)} {ula(node_b)}"


def _hex_hash(text: str) -> str | None:
    if len(text) != 64:
        return None
    try:
        raw = bytes.fromhex(text)
    except ValueError:
        return None
    if len(raw) != 32:
        return None
    return raw.hex()


def publish(vm, pos) -> None:
    text = vm.pop("~p", pos)
    if not isinstance(text, str):
        from .vm import type_name
        raise vm.err(TYPE_MISMATCH, f"`~p` expected a string, got {type_name(text)}",
                     op="~p", pos=pos, hint="publish the bundle's bytes as a string")
    digest = blake3_hex(text.encode("utf-8"))
    vm.bundles[digest] = text
    vm.push(digest)


def fetch(vm, pos) -> None:
    text = vm.pop("~f", pos)
    if not isinstance(text, str) or _hex_hash(text) is None:
        raise vm.err(TYPE_MISMATCH, "`~f` expected a 64-digit BLAKE3 hex hash",
                     op="~f", pos=pos, hint="publish first; ~p pushes the hash")
    digest = _hex_hash(text)
    assert digest is not None
    got = vm.bundles.get(digest)
    if got is None:
        raise vm.err(BOUNDS, "`~f` has no bundle for that hash",
                     op="~f", pos=pos, hint="the hash is the address; publish it with ~p")
    vm.push(got)


def demo(port: int = DEFAULT_PORT) -> str:
    digest, text, addrs = exchange("hello bundle", port)
    a, b = addrs.split()
    return f"peer {a}\npeer {b}\nhash {digest}\ngot {text}\n"
