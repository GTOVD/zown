#!/usr/bin/env python3
"""Boot a Zown print program in long mode and read the VGA text buffer.

`zownc kern` writes a floppy image. The 64-bit kernel prints the text at
`0xB8000` and on port `0xE9`. After four timer ticks the cell at row 24,
column 0 is `4`. A load of the unmapped page at `0x5000` stores `0xF1` at
`0x7E00` and `F` in the next cell. The kernel halts. It does not reset.

Usage:
    python3 conformance/kern_boot.py
"""

from __future__ import annotations

import os
import re
import shutil
import socket
import subprocess
import tempfile
import time

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
ZOWNC = os.path.join(ROOT, "zownc", "target", "debug",
                     "zownc.exe" if os.name == "nt" else "zownc")
QEMU = "qemu-system-x86_64"
EXAMPLE = os.path.join(ROOT, "examples", "bare.zn")

GREEN, RED, YEL, RST = "\033[32m", "\033[31m", "\033[33m", "\033[0m"


class BootFailure(Exception):
    pass


def build(src_path: str, out_path: str) -> None:
    proc = subprocess.run([ZOWNC, "kern", src_path, "-o", out_path], capture_output=True, text=True)
    if proc.returncode != 0:
        raise BootFailure(proc.stderr.strip() or proc.stdout.strip() or "zownc kern failed")


def boot(img: str) -> tuple[list[int], list[int], list[int], bytes]:
    work = tempfile.mkdtemp()
    sock = os.path.join(work, "mon.sock")
    log = os.path.join(work, "e9.txt")
    proc = subprocess.Popen(
        [
            QEMU, "-drive", f"file={img},format=raw,if=floppy", "-boot", "a",
            "-display", "none", "-vga", "std", "-machine", "pc,accel=tcg",
            "-monitor", f"unix:{sock},server,nowait",
            "-device", "isa-debugcon,iobase=0xe9,chardev=dbg",
            "-chardev", f"file,id=dbg,path={log}",
            "-net", "none", "-no-reboot",
        ],
        stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
    )
    try:
        client = None
        for _ in range(50):
            if proc.poll() is not None:
                err = proc.stderr.read().decode("utf-8", "replace")
                raise BootFailure(f"qemu exited early: {err.strip()}")
            try:
                client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                client.settimeout(2)
                client.connect(sock)
                break
            except (FileNotFoundError, ConnectionRefusedError):
                client.close()
                client = None
                time.sleep(0.05)
        if client is None:
            raise BootFailure("qemu monitor did not open")
        client.settimeout(0.2)
        try:
            while client.recv(4096):
                pass
        except socket.timeout:
            pass
        client.settimeout(2)

        def cmd(text: str) -> str:
            client.sendall((text + "\n").encode())
            data = b""
            deadline = time.time() + 2
            while time.time() < deadline:
                try:
                    chunk = client.recv(4096)
                except socket.timeout:
                    break
                if not chunk:
                    break
                data += chunk
                if data.rstrip().endswith(b"(qemu)"):
                    break
            return data.decode("utf-8", "replace")

        flag: list[int] = []
        deadline = time.time() + 3
        while time.time() < deadline:
            flag = _bytes(cmd("xp /16bx 0x7e00"))
            if flag and flag[0] in (0xF1, 0xEE, 0xE0):
                break
            time.sleep(0.05)
        vga = _bytes(cmd("xp /176bx 0xb8000"))
        # Row 24 is 24*160 bytes past 0xB8000.
        status = _bytes(cmd("xp /4bx 0xb8f00"))
        cmd("quit")
        client.close()
        proc.wait(timeout=5)
        raw = open(log, "rb").read()
        if len(flag) < 12:
            raise BootFailure("no report at 0x7E00")
        return vga, flag, status, raw
    finally:
        if proc.poll() is None:
            proc.kill()
            proc.wait(timeout=5)


def _bytes(text: str) -> list[int]:
    vals: list[int] = []
    for line in text.splitlines():
        if ":" not in line:
            continue
        payload = line.split(":", 1)[1]
        vals.extend(int(v, 16) for v in re.findall(r"0x([0-9a-fA-F]{2})", payload))
    return vals


def expect_screen(vga: list[int], text: str) -> None:
    di = 0
    for ch in text:
        if ch == "\n":
            di = (di // 160 + 1) * 160
            continue
        if di + 1 >= len(vga):
            raise BootFailure(f"VGA dump is shorter than the text ({len(vga)} bytes)")
        if vga[di] != ord(ch) or vga[di + 1] != 0x07:
            raise BootFailure(
                f"cell {di}: got {vga[di]:02x} {vga[di + 1]:02x}, want {ord(ch):02x} 07"
            )
        di += 2


def check(name: str, src: str, text: str) -> None:
    work = tempfile.mkdtemp()
    zn = os.path.join(work, "p.zn")
    img = os.path.join(work, "kern.bin")
    with open(zn, "w", encoding="utf-8") as fh:
        fh.write(src)
    build(zn, img)
    blob = open(img, "rb").read()
    if len(blob) < 512 or len(blob) % 512 != 0 or blob[510:512] != b"\x55\xaa":
        raise BootFailure(f"{name}: image is not a floppy")
    vga, flag, status, e9 = boot(img)
    if flag[0] != 0xF1:
        raise BootFailure(f"{name}: fault report at 0x7E00 is {flag[0]:#x}")
    if flag[1] != 0x64:
        raise BootFailure(f"{name}: long-mode marker is {flag[1]:#x}")
    cr2 = flag[4] | (flag[5] << 8) | (flag[6] << 16) | (flag[7] << 24)
    if cr2 != 0x5000:
        raise BootFailure(f"{name}: CR2 is {cr2:#x}")
    if flag[8] != 4:
        raise BootFailure(f"{name}: timer count is {flag[8]}")
    if len(status) < 4 or status[0] != ord("4") or status[1] != 0x07:
        raise BootFailure(f"{name}: tick cell is {status}")
    if status[2] != ord("F") or status[3] != 0x07:
        raise BootFailure(f"{name}: fault cell is {status}")
    if e9 != text.encode():
        raise BootFailure(f"{name}: port 0xe9 is {e9!r}")
    expect_screen(vga, text)
    print(f"{GREEN}ok{RST}   {name}")


def main() -> int:
    if not os.path.exists(ZOWNC):
        print(f"{RED}error{RST}: {ZOWNC} not found. Build it: (cd zownc && cargo build)")
        return 2
    if shutil.which(QEMU) is None:
        print(f"{YEL}skip{RST} kern_boot  ({QEMU} not found)")
        return 0
    try:
        src = open(EXAMPLE, encoding="utf-8").read()
        check("examples/bare.zn", src, "Zown\n")
        check("row", "$A$ .\n$B$ pr\n", "A\nB")
    except BootFailure as e:
        print(f"{RED}FAIL{RST} {e}")
        return 1
    print("\n2 booted, 0 failed")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
