"""Language server over stdio (SPEC §9).

Diagnostics are the checker's human text. Go to definition finds a user
binding. Formatting is `zown fmt`. There are no speculative fixes.
"""

from __future__ import annotations

import json
import sys
from typing import Any

from .checker import check_src
from .errors import ZownError
from .fmt import format_source
from .lexer import T_BIND, T_EOF, T_IDENT, Lexer


def _offset(src: str, line: int, character: int) -> int:
    """LSP positions are 0-based. Zown columns count characters."""
    lines = src.splitlines(keepends=True)
    if line < 0:
        return 0
    if line >= len(lines):
        return len(src)
    base = sum(len(lines[i]) for i in range(line))
    return base + max(0, character)


def _lsp_pos(src: str, offset: int) -> dict[str, int]:
    line = src.count("\n", 0, offset)
    last = src.rfind("\n", 0, offset)
    col = offset if last < 0 else offset - last - 1
    return {"line": line, "character": col}


def _range_from(src: str, start: int, length: int) -> dict[str, Any]:
    return {"start": _lsp_pos(src, start), "end": _lsp_pos(src, start + length)}


def diagnostics(src: str, file: str) -> list[dict[str, Any]]:
    """Same texts `zown check` prints, one diagnostic per finding."""
    try:
        diags = check_src(src, file)
    except ZownError as e:
        e.file = e.file or file
        return [_diagnostic(src, e)]
    return [_diagnostic(src, d) for d in diags]


def _diagnostic(src: str, err: ZownError) -> dict[str, Any]:
    if err.pos is not None:
        start = err.pos.offset
        length = len(err.op) if err.op else 1
    else:
        start, length = 0, 1
    return {
        "range": _range_from(src, start, length),
        "severity": 1,
        "source": "zown",
        "code": err.code,
        "message": err.render_human(),
    }


def _tokens(src: str):
    return [t for t in Lexer(src).tokens() if t.kind != T_EOF]


def definition(src: str, line: int, character: int) -> dict[str, Any] | None:
    """The `:name` that binds the name under the cursor. Builtins have none."""
    offset = _offset(src, line, character)
    toks = _tokens(src)
    hit = None
    for t in toks:
        begin, length = t.span
        if begin <= offset < begin + length:
            hit = t
            break
    if hit is None or hit.kind not in (T_IDENT, T_BIND):
        return None
    name = str(hit.value)
    binds = [t for t in toks if t.kind == T_BIND and t.value == name]
    if not binds:
        return None
    if hit.kind == T_BIND:
        chosen = hit
    else:
        before = [t for t in binds if t.span[0] <= hit.span[0]]
        chosen = before[-1] if before else binds[0]
    begin, length = chosen.span
    return _range_from(src, begin, length)


def formatting(src: str) -> list[dict[str, Any]]:
    """One edit replacing the document with `zown fmt` output."""
    new = format_source(src)
    end = _lsp_pos(src, len(src))
    return [{
        "range": {"start": {"line": 0, "character": 0}, "end": end},
        "newText": new,
    }]


class Server:
    def __init__(self) -> None:
        self.docs: dict[str, str] = {}
        self.shutdown = False

    def handle(self, msg: dict[str, Any]) -> list[dict[str, Any]]:
        method = msg.get("method")
        mid = msg.get("id")
        params = msg.get("params") or {}
        if method == "initialize":
            return [self._result(mid, {
                "capabilities": {
                    "textDocumentSync": 1,
                    "definitionProvider": True,
                    "documentFormattingProvider": True,
                },
                "serverInfo": {"name": "zown", "version": "0.2"},
            })]
        if method == "initialized":
            return []
        if method == "shutdown":
            self.shutdown = True
            return [self._result(mid, None)]
        if method == "exit":
            return []
        if method == "textDocument/didOpen":
            doc = params.get("textDocument") or {}
            uri = doc.get("uri", "")
            text = doc.get("text", "")
            self.docs[uri] = text
            return [self._publish(uri, text)]
        if method == "textDocument/didChange":
            doc = params.get("textDocument") or {}
            uri = doc.get("uri", "")
            changes = params.get("contentChanges") or []
            if changes and "text" in changes[0]:
                self.docs[uri] = changes[0]["text"]
            return [self._publish(uri, self.docs.get(uri, ""))]
        if method == "textDocument/definition":
            return [self._result(mid, self._definition(params))]
        if method == "textDocument/formatting":
            try:
                return [self._result(mid, self._format(params))]
            except ZownError as e:
                return [self._error(mid, e.render_human())]
        if method == "textDocument/diagnostic":
            uri = (params.get("textDocument") or {}).get("uri", "")
            text = self.docs.get(uri, "")
            return [self._result(mid, {"kind": "full", "items": diagnostics(text, uri)})]
        if mid is not None:
            return [self._error(mid, f"method {method} is not implemented")]
        return []

    def _definition(self, params: dict[str, Any]) -> dict[str, Any] | None:
        uri = (params.get("textDocument") or {}).get("uri", "")
        pos = params.get("position") or {}
        text = self.docs.get(uri, "")
        found = definition(text, int(pos.get("line", 0)), int(pos.get("character", 0)))
        if found is None:
            return None
        return {"uri": uri, "range": found}

    def _format(self, params: dict[str, Any]) -> list[dict[str, Any]]:
        uri = (params.get("textDocument") or {}).get("uri", "")
        return formatting(self.docs.get(uri, ""))

    def _publish(self, uri: str, text: str) -> dict[str, Any]:
        return {
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": diagnostics(text, uri)},
        }

    @staticmethod
    def _result(mid: Any, result: Any) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": mid, "result": result}

    @staticmethod
    def _error(mid: Any, message: str) -> dict[str, Any]:
        return {"jsonrpc": "2.0", "id": mid, "error": {"code": -32601, "message": message}}


def serve(stdin: Any | None = None, stdout: Any | None = None) -> int:
    """Read JSON-RPC messages framed with Content-Length. Exit 0 after shutdown."""
    inp = stdin if stdin is not None else sys.stdin.buffer
    out = stdout if stdout is not None else sys.stdout.buffer
    server = Server()
    while True:
        msg = _read(inp)
        if msg is None:
            return 1 if not server.shutdown else 0
        replies = server.handle(msg)
        for reply in replies:
            _write(out, reply)
        if msg.get("method") == "exit":
            return 0 if server.shutdown else 1


def _read(inp: Any) -> dict[str, Any] | None:
    headers: dict[str, str] = {}
    while True:
        line = inp.readline()
        if line == b"":
            return None
        if line in (b"\r\n", b"\n"):
            break
        text = line.decode("utf-8", "replace")
        if ":" in text:
            key, value = text.split(":", 1)
            headers[key.strip().lower()] = value.strip()
    n = int(headers.get("content-length", "0"))
    body = inp.read(n)
    if not body:
        return None
    return json.loads(body.decode("utf-8"))


def _write(out: Any, msg: dict[str, Any]) -> None:
    data = json.dumps(msg).encode("utf-8")
    out.write(f"Content-Length: {len(data)}\r\n\r\n".encode("ascii"))
    out.write(data)
    out.flush()
