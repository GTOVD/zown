#!/usr/bin/env python3
"""Stage-2 compiler parity (M23).

The lowering in `zown/std/lower.zn` prints the IR text `zownc ir` prints.
The fixed point is that file's own source. The oracle runs it, and so does
`zownc native --unchecked`.

The byte emitter in `zown/std/emit.zn` writes the 512-byte print-subset boot
sector. Three images are compared, and they are the same bytes:

  * `zownc bare` (stage-0)
  * `zownc native --unchecked` of the emitter (the compiler running in stage-0)
  * the executable `zownc native -o` writes, run on its own

That third image is what QEMU boots. A boot sector cannot host a second
compiler, so the byte identity is the image, not an ELF compared with a sector.

Usage:
    python3 conformance/stage2_parity.py
"""

from __future__ import annotations

import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, ROOT)
sys.path.insert(0, os.path.join(ROOT, "conformance"))

import bare_boot  # noqa: E402
from zown.stage import (  # noqa: E402
    emit_program,
    image_from_hex,
    lower_program,
    zown_image,
    zown_ir,
)

ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
IR_FILES = [
    os.path.join(ROOT, "zown", "std", "lower.zn"),
    os.path.join(ROOT, "zown", "std", "emit.zn"),
    os.path.join(ROOT, "examples", "hello.zn"),
    os.path.join(ROOT, "examples", "bare.zn"),
    os.path.join(ROOT, "examples", "fib.zn"),
]
# Print-subset sources and the VGA text the sector must show.
BOOTS = [
    (os.path.join(ROOT, "examples", "bare.zn"), None, "Zown\n"),
    (None, "$A$ .\n$B$ pr\n", "A\nB"),
    (None, "$Hi$ pr\n", "Hi"),
]

GREEN, RED, RST = "\033[32m", "\033[31m", "\033[0m"


def host_ir(path: str) -> str:
    out = subprocess.run([ZOWNC, "ir", path], capture_output=True, text=True)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or "zownc ir failed")
    return out.stdout


def host_image(path: str) -> bytes:
    fd, out_path = tempfile.mkstemp(suffix=".bin")
    os.close(fd)
    try:
        proc = subprocess.run(
            [ZOWNC, "bare", path, "-o", out_path],
            capture_output=True, text=True,
        )
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or "zownc bare failed")
        with open(out_path, "rb") as fh:
            return fh.read()
    finally:
        os.remove(out_path)


def native_stdout(prog: str) -> str:
    fd, path = tempfile.mkstemp(suffix=".zn")
    os.close(fd)
    try:
        with open(path, "w", encoding="utf-8") as fh:
            fh.write(prog)
        out = subprocess.run(
            [ZOWNC, "native", "--unchecked", path],
            capture_output=True, text=True,
        )
    finally:
        os.remove(path)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or out.stdout)
    return out.stdout


def linked_stdout(prog: str) -> str:
    """stage-0 writes an executable; that executable's stdout is the next stage."""
    src_fd, src = tempfile.mkstemp(suffix=".zn")
    os.close(src_fd)
    exe = src + ".exe"
    try:
        with open(src, "w", encoding="utf-8") as fh:
            fh.write(prog)
        built = subprocess.run(
            [ZOWNC, "native", "--unchecked", "-o", exe, src],
            capture_output=True, text=True,
        )
        if built.returncode != 0:
            raise RuntimeError(built.stderr.strip() or "zownc native -o failed")
        out = subprocess.run([exe], capture_output=True, text=True)
    finally:
        os.remove(src)
        if os.path.exists(exe):
            os.remove(exe)
    if out.returncode != 0:
        raise RuntimeError(out.stderr.strip() or out.stdout)
    return out.stdout


def boot_image(blob: bytes, text: str) -> None:
    if len(blob) != 512 or blob[-2:] != b"\x55\xaa":
        raise bare_boot.BootFailure("image is not a boot sector")
    work = tempfile.mkdtemp()
    img = os.path.join(work, "boot.bin")
    with open(img, "wb") as fh:
        fh.write(blob)
    vga, flag, e9 = bare_boot.boot(img)
    if flag != 0xB1:
        raise bare_boot.BootFailure(f"int 0x20 left {flag:#x} at 0x8000")
    if e9 != text.encode():
        raise bare_boot.BootFailure(f"port 0xe9 is {e9!r}")
    bare_boot.expect_screen(vga, text)


def check_ir(path: str) -> None:
    name = os.path.relpath(path, ROOT)
    src = open(path, encoding="utf-8").read()
    host = host_ir(path)
    got = zown_ir(src, path)
    if got != host:
        raise RuntimeError("oracle IR differs from zownc ir")
    native = native_stdout(lower_program(src, file=path))
    if native != host:
        raise RuntimeError("native IR differs from zownc ir")
    print(f"{GREEN}ok{RST}   ir {name}")


def check_boot(path: str | None, src: str, text: str, label: str) -> None:
    if path is not None:
        src = open(path, encoding="utf-8").read()
    fd, zn = tempfile.mkstemp(suffix=".zn")
    os.close(fd)
    try:
        with open(zn, "w", encoding="utf-8") as fh:
            fh.write(src)
        host = host_image(zn)
    finally:
        os.remove(zn)
    oracle = zown_image(src)
    if oracle != host:
        raise RuntimeError("oracle image differs from zownc bare")
    prog = emit_program(src)
    native = image_from_hex(native_stdout(prog))
    if native != host:
        raise RuntimeError("native image differs from zownc bare")
    linked = image_from_hex(linked_stdout(prog))
    if linked != host:
        raise RuntimeError("linked image differs from zownc bare")
    boot_image(linked, text)
    print(f"{GREEN}ok{RST}   boot {label}")


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    fail = 0
    for path in IR_FILES:
        try:
            check_ir(path)
        except (RuntimeError, OSError) as e:
            print(f"{RED}FAIL{RST} ir {os.path.relpath(path, ROOT)}: {e}")
            fail += 1
    for path, src, text in BOOTS:
        label = os.path.relpath(path, ROOT) if path else text.replace("\n", "\\n")
        try:
            check_boot(path, src or "", text, label)
        except (RuntimeError, OSError, bare_boot.BootFailure) as e:
            print(f"{RED}FAIL{RST} boot {label}: {e}")
            fail += 1
    total = len(IR_FILES) + len(BOOTS)
    print(f"\n{total - fail} parity, {fail} diff")
    return 1 if fail else 0


if __name__ == "__main__":
    raise SystemExit(main())
