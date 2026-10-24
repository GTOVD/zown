"""Load `std/core.zn` into a VM.

The file is ordinary Zown. `wd` registers each block as an auto-running word,
so a later `mx` runs that block instead of a host function. User bindings still
win: `:mx` shadows the std word.
"""

from __future__ import annotations

import os

from .errors import ZownError


def std_path() -> str:
    return os.path.join(os.path.dirname(os.path.abspath(__file__)), "std", "core.zn")


def load_std(vm) -> None:
    path = std_path()
    with open(path, encoding="utf-8") as fh:
        src = fh.read()
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or path
        raise
