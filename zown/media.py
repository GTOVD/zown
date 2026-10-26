"""Media words (SPEC §23).

`op` decodes one Opus packet into an exclusive `buf` of s16le PCM.
`av` decodes one AV1 still into an exclusive `buf` of 8-bit I420.
`ve` encodes one 16×16 4:2:0 frame into a raw OBU sequence.
`oe` encodes 960 bytes of s16le into one narrowband Opus packet.
A bare name passes a borrow, so the binding keeps the exclusive buffer.
A camera, a microphone, and a player are not this slice.
"""

from __future__ import annotations

from .buf import Alloc, Buf, release
from .errors import BOUNDS, TYPE_MISMATCH, Pos


def _take(vm, op: str, pos: Pos) -> bytes:
    from .vm import type_name
    v = vm.pop(op, pos)
    if not isinstance(v, Buf):
        raise vm.err(TYPE_MISMATCH, f"`{op}` expected a buf, got {type_name(v)}",
                     op=op, pos=pos, hint="the packet or the frame is a buf")
    if v.alloc.data is None:
        raise vm.err(BOUNDS, f"`{op}` buffer was freed",
                     op=op, pos=pos, hint="the exclusive owner was dropped with `,`")
    data = bytes(v.alloc.data)
    release(v)
    return data


def _push(vm, data: bytes) -> None:
    alloc = Alloc(len(data))
    if data and alloc.data is not None:
        alloc.data[:] = data
    vm.push(Buf(alloc, True))


def _fault(vm, op: str, pos: Pos, err: BaseException) -> None:
    code = getattr(err, "code", TYPE_MISMATCH)
    raise vm.err(code, str(err), op=op, pos=pos)


def b_oe(vm, pos: Pos) -> None:
    # pcm oe -> exclusive buf of one TOC config-18 mono packet
    from .opus import Fault
    from .opusenc import encode
    data = _take(vm, "oe", pos)
    try:
        pkt = encode(data)
    except Fault as err:
        _fault(vm, "oe", pos, err)
    _push(vm, pkt)


def b_op(vm, pos: Pos) -> None:
    # packet op -> exclusive buf of s16le PCM
    from .opus import Fault, decode
    data = _take(vm, "op", pos)
    try:
        pcm = decode(data)
    except Fault as err:
        _fault(vm, "op", pos, err)
    _push(vm, pcm)


def b_av(vm, pos: Pos) -> None:
    # obu av -> exclusive buf of I420 (Y, then U, then V)
    from .av1 import Fault, decode
    data = _take(vm, "av", pos)
    try:
        pic = decode(data)
    except Fault as err:
        _fault(vm, "av", pos, err)
    _push(vm, pic)


def b_ve(vm, pos: Pos) -> None:
    # i420 ve -> exclusive buf of a raw AV1 OBU sequence
    from .av1 import Fault
    from .av1enc import encode
    data = _take(vm, "ve", pos)
    try:
        obu = encode(data)
    except Fault as err:
        _fault(vm, "ve", pos, err)
    _push(vm, obu)
