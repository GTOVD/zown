"""TCP streams on `[::1]` (SPEC §14).

`~o` connects. `~l` accepts one local connection and registers a name.
`~c` connects to that name. `~s` and `~r` move one UTF-8 frame. `~w`
forwards one frame each way between two accepted connections. Nothing
here dials off the machine. QUIC is not this transport.
"""

from __future__ import annotations

import os
import socket
import time

from .errors import BOUNDS, CAP_DENIED, TYPE_MISMATCH

BUF_MAX = 1 << 20
WAIT = 1.0


class Conn:
    def __init__(self, sock: socket.socket) -> None:
        sock.settimeout(WAIT)
        self.sock: socket.socket | None = sock

    def close(self) -> None:
        sock = self.sock
        self.sock = None
        if sock is not None:
            try:
                sock.close()
            except OSError:
                pass


class Stream:
    def __init__(self, conn: Conn, exclusive: bool = True) -> None:
        self.conn = conn
        self.exclusive = exclusive

    def borrow(self) -> Stream:
        return Stream(self.conn, False)


def _deny(vm, op: str, cap: str, pos) -> None:
    raise vm.err(
        CAP_DENIED,
        f"`{op}` needs `{cap}",
        op=op,
        pos=pos,
        kind="sec",
        cap=cap,
        hint=f"grant it: `{cap} [ ... ] gr",
    )


def _need(vm, op: str, cap: str, pos) -> None:
    if cap not in vm.caps:
        _deny(vm, op, cap, pos)


def _port(vm, op: str, pos, v) -> int:
    from .vm import type_name
    if isinstance(v, bool) or not isinstance(v, int):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a port, got {type_name(v)}",
                     op=op, pos=pos)
    if v < 1 or v > 65535:
        raise vm.err(BOUNDS, f"`{op}` port {v} is outside 1..65535", op=op, pos=pos)
    return v


def _word(vm, op: str, pos, v) -> str:
    from .vm import type_name
    if not isinstance(v, str) or v == "" or any(ch.isspace() for ch in v):
        got = type_name(v) if not isinstance(v, str) else "a name with whitespace"
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a single-word name, got {got}",
                     op=op, pos=pos)
    return v


def _stream(vm, op: str, pos, v) -> Stream:
    from .vm import type_name
    if not isinstance(v, Stream):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a stream, got {type_name(v)}",
                     op=op, pos=pos, hint="open one with ~o, ~l, or ~c")
    if v.conn.sock is None:
        raise vm.err(BOUNDS, f"`{op}` stream is closed", op=op, pos=pos,
                     hint="the exclusive stream was dropped with `,`")
    return v


def _mesh_dir() -> str:
    path = os.environ.get("ZOWN_MESH") or os.path.join(
        os.environ.get("TMPDIR", "/tmp"), "zown-mesh"
    )
    os.makedirs(path, exist_ok=True)
    return path


def _with_names(edit):
    directory = _mesh_dir()
    path = os.path.join(directory, "names")
    lock = path + ".lock"
    deadline = time.time() + WAIT
    while True:
        try:
            fd = os.open(lock, os.O_CREAT | os.O_EXCL | os.O_WRONLY)
            break
        except FileExistsError:
            if time.time() >= deadline:
                raise TimeoutError(lock)
            time.sleep(0.02)
    try:
        lines: list[str] = []
        if os.path.exists(path):
            with open(path, encoding="utf-8") as fh:
                lines = [ln for ln in fh.read().splitlines() if ln]
        out = edit(lines)
        tmp = path + ".tmp"
        text = ("\n".join(lines) + "\n") if lines else ""
        with open(tmp, "w", encoding="utf-8") as fh:
            fh.write(text)
        os.replace(tmp, path)
        return out
    finally:
        os.close(fd)
        try:
            os.remove(lock)
        except OSError:
            pass


def _register(name: str, proto: str, port: int) -> None:
    def edit(lines: list[str]) -> None:
        lines[:] = [ln for ln in lines if ln.split(" ", 1)[0] != name]
        lines.append(f"{name} {proto} {port}")
        return None
    _with_names(edit)


def _lookup(name: str) -> tuple[str, int] | None:
    deadline = time.time() + WAIT
    path = os.path.join(_mesh_dir(), "names")
    while True:
        try:
            text = open(path, encoding="utf-8").read()
        except OSError:
            text = ""
        for ln in text.splitlines():
            parts = ln.split(" ")
            if len(parts) == 3 and parts[0] == name and parts[2].isdigit():
                return parts[1], int(parts[2])
        if time.time() >= deadline:
            return None
        time.sleep(0.02)


def _tcp() -> socket.socket:
    return socket.socket(socket.AF_INET6, socket.SOCK_STREAM)


def _connect(port: int) -> Conn:
    sock = _tcp()
    sock.settimeout(WAIT)
    sock.connect(("::1", port, 0, 0))
    return Conn(sock)


def _listener(port: int, backlog: int) -> socket.socket:
    listener = _tcp()
    listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    listener.bind(("::1", port, 0, 0))
    listener.listen(backlog)
    listener.settimeout(WAIT)
    return listener


def _io(vm, op: str, pos, fn):
    try:
        return fn()
    except (TimeoutError, socket.timeout):
        raise vm.err(BOUNDS, f"`{op}` timed out", op=op, pos=pos,
                     hint="the peer did not answer on [::1]") from None
    except OSError as exc:
        raise vm.err(BOUNDS, f"`{op}` {exc.strerror or exc}", op=op, pos=pos) from None
    except ValueError:
        raise vm.err(BOUNDS, f"`{op}` frame is too large", op=op, pos=pos) from None


def _readn(sock: socket.socket, n: int) -> bytes:
    buf = bytearray()
    while len(buf) < n:
        chunk = sock.recv(n - len(buf))
        if not chunk:
            raise ConnectionError("closed")
        buf += chunk
    return bytes(buf)


def _recv_frame(sock: socket.socket) -> bytes:
    n = int.from_bytes(_readn(sock, 4), "big")
    if n > BUF_MAX:
        raise ValueError("frame")
    return _readn(sock, n)


def _send_frame(sock: socket.socket, data: bytes) -> None:
    sock.sendall(len(data).to_bytes(4, "big") + data)


def op_open(vm, pos) -> None:
    _need(vm, "~o", "t", pos)
    port = _port(vm, "~o", pos, vm.pop("~o", pos))
    host = vm.pop("~o", pos)
    from .vm import type_name
    if not isinstance(host, str):
        raise vm.err(TYPE_MISMATCH, f"`~o` expected a host string, got {type_name(host)}",
                     op="~o", pos=pos)
    if host != "::1":
        raise vm.err(BOUNDS, "`~o` stays on [::1]", op="~o", pos=pos,
                     hint="this slice does not dial off the machine")
    conn = _io(vm, "~o", pos, lambda: _connect(port))
    vm.push(Stream(conn))


def op_listen(vm, pos) -> None:
    _need(vm, "~l", "r", pos)
    port = _port(vm, "~l", pos, vm.pop("~l", pos))
    proto = _word(vm, "~l", pos, vm.pop("~l", pos))
    name = _word(vm, "~l", pos, vm.pop("~l", pos))
    def listen():
        listener = _listener(port, 1)
        try:
            _register(name, proto, port)
            conn, _addr = listener.accept()
        finally:
            listener.close()
        return Conn(conn)
    vm.push(Stream(_io(vm, "~l", pos, listen)))


def op_connect(vm, pos) -> None:
    _need(vm, "~c", "t", pos)
    proto = _word(vm, "~c", pos, vm.pop("~c", pos))
    name = _word(vm, "~c", pos, vm.pop("~c", pos))
    found = _lookup(name)
    if found is None:
        raise vm.err(BOUNDS, f"`~c` has no peer named {name}", op="~c", pos=pos,
                     hint="the listener registers the name with ~l")
    offered, port = found
    if offered != proto:
        raise vm.err(TYPE_MISMATCH,
                     f"`~c` protocol {proto} does not match {offered} for {name}",
                     op="~c", pos=pos)
    vm.push(Stream(_io(vm, "~c", pos, lambda: _connect(port))))


def op_send(vm, pos) -> None:
    _need(vm, "~s", "t", pos)
    msg = vm.pop("~s", pos)
    from .vm import type_name
    if not isinstance(msg, str):
        raise vm.err(TYPE_MISMATCH, f"`~s` expected a string, got {type_name(msg)}",
                     op="~s", pos=pos)
    data = msg.encode("utf-8")
    if len(data) > BUF_MAX:
        raise vm.err(BOUNDS, "`~s` message is too large", op="~s", pos=pos)
    stm = _stream(vm, "~s", pos, vm.pop("~s", pos))
    try:
        _io(vm, "~s", pos, lambda: _send_frame(stm.conn.sock, data))
    finally:
        vm.push(stm)


def op_recv(vm, pos) -> None:
    _need(vm, "~r", "r", pos)
    stm = _stream(vm, "~r", pos, vm.pop("~r", pos))
    try:
        data = _io(vm, "~r", pos, lambda: _recv_frame(stm.conn.sock))
        text = data.decode("utf-8")
    except UnicodeDecodeError:
        vm.push(stm)
        raise vm.err(TYPE_MISMATCH, "`~r` frame is not UTF-8", op="~r", pos=pos) from None
    except Exception:
        vm.push(stm)
        raise
    vm.push(stm)
    vm.push(text)


def op_relay(vm, pos) -> None:
    if "r" not in vm.caps:
        _deny(vm, "~w", "r", pos)
    if "t" not in vm.caps:
        _deny(vm, "~w", "t", pos)
    port = _port(vm, "~w", pos, vm.pop("~w", pos))
    def relay():
        listener = _listener(port, 2)
        try:
            left_sock, _a = listener.accept()
            right_sock, _b = listener.accept()
        finally:
            listener.close()
        left, right = Conn(left_sock), Conn(right_sock)
        try:
            _send_frame(right.sock, _recv_frame(left.sock))
            _send_frame(left.sock, _recv_frame(right.sock))
        finally:
            left.close()
            right.close()
    _io(vm, "~w", pos, relay)
