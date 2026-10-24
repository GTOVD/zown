"""Logical control-plane log (SPEC §27).

Events are numbered in program order. There is no clock, so two VMs print
the same log. A remediation note is recorded on the next matching fault.
It is not executed, and it does not replace running code.
"""

from __future__ import annotations

from dataclasses import dataclass, field

from .errors import TYPE_MISMATCH


@dataclass
class Telemetry:
    content: str = "-"
    version: str = "-"
    trace: str = "-"
    spans: list[str] = field(default_factory=list)
    wants: dict[str, str] = field(default_factory=dict)
    have: dict[str, str] = field(default_factory=dict)
    remedy: dict[str, str] = field(default_factory=dict)
    events: list[str] = field(default_factory=list)
    seq: int = 0

    def line(self, kind: str, rest: str = "") -> None:
        self.seq += 1
        tail = f" {rest}" if rest else ""
        self.events.append(
            f"{self.seq} {self.content} {self.version} {self.trace} {kind}{tail}"
        )

    def text(self) -> str:
        return "\n".join(self.events)

    def fault(self, code: str, op: str | None) -> None:
        note = self.remedy.get(code, "-")
        self.line("fault", f"{code} {op or '-'} {note}")

    def drift(self) -> str:
        lines = []
        for name, want in self.wants.items():
            have = self.have.get(name)
            if have != want:
                lines.append(f"{name} {want} {have if have is not None else '-'}")
        return "\n".join(lines) if lines else "ok"


def _word(vm, op: str, pos, v) -> str:
    from .vm import type_name
    if not isinstance(v, str) or v == "" or any(ch.isspace() for ch in v):
        got = type_name(v) if not isinstance(v, str) else "a name with whitespace"
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a single-word name, got {got}",
                     op=op, pos=pos, hint="this field is one word, with no spaces")
    return v


def _note(vm, pos, v) -> str:
    from .vm import type_name
    if not isinstance(v, str) or v == "" or "\n" in v or "\r" in v:
        if not isinstance(v, str):
            got = type_name(v)
        elif v == "":
            got = "an empty string"
        else:
            got = "a string with a newline"
        raise vm.err(TYPE_MISMATCH, f"`nt` expected a one-line note, got {got}",
                     op="nt", pos=pos)
    return v


def _hex(vm, pos, v) -> str:
    from .vm import type_name
    if (not isinstance(v, str) or len(v) != 64
            or any(c not in "0123456789abcdef" for c in v.lower())):
        got = type_name(v) if not isinstance(v, str) else "a string that is not 64 hex digits"
        raise vm.err(TYPE_MISMATCH, f"`ch` expected 64 hex digits, got {got}",
                     op="ch", pos=pos, hint="hash the text with h3 first")
    return v.lower()


def b_ch(vm, pos) -> None:
    vm.tele.content = _hex(vm, pos, vm.pop("ch", pos))


def b_vr(vm, pos) -> None:
    vm.tele.version = _word(vm, "vr", pos, vm.pop("vr", pos))


def b_td(vm, pos) -> None:
    vm.tele.trace = _word(vm, "td", pos, vm.pop("td", pos))


def b_sp(vm, pos) -> None:
    name = _word(vm, "sp", pos, vm.pop("sp", pos))
    vm.tele.spans.append(name)
    vm.tele.line("open", name)


def b_se(vm, pos) -> None:
    if not vm.tele.spans:
        raise vm.err(TYPE_MISMATCH, "`se` has no open span",
                     op="se", pos=pos, hint="open one with $name$ sp")
    name = vm.tele.spans.pop()
    vm.tele.line("close", name)


def b_nt(vm, pos) -> None:
    text = _note(vm, pos, vm.pop("nt", pos))
    vm.tele.line("note", text)


def b_wn(vm, pos) -> None:
    value = _word(vm, "wn", pos, vm.pop("wn", pos))
    name = _word(vm, "wn", pos, vm.pop("wn", pos))
    vm.tele.wants[name] = value
    vm.tele.line("want", f"{name} {value}")


def b_ob(vm, pos) -> None:
    value = _word(vm, "ob", pos, vm.pop("ob", pos))
    name = _word(vm, "ob", pos, vm.pop("ob", pos))
    vm.tele.have[name] = value
    vm.tele.line("obs", f"{name} {value}")
    want = vm.tele.wants.get(name)
    if want is None:
        return
    if want == value:
        vm.tele.line("meet", name)
    else:
        vm.tele.line("drift", f"{name} {want} {value}")


def b_df(vm, pos) -> None:
    vm.push(vm.tele.drift())


def b_rm(vm, pos) -> None:
    note = _word(vm, "rm", pos, vm.pop("rm", pos))
    code = _word(vm, "rm", pos, vm.pop("rm", pos))
    vm.tele.remedy[code] = note


def b_lg(vm, pos) -> None:
    vm.push(vm.tele.text())
