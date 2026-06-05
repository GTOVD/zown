"""Zown: an AI-native, token-dense, stack-based programming language.

This package is the v0.1 reference implementation: a lexer, parser, stack VM,
token-dense standard library, structured (.zerr) diagnostics, and a shadow
manifest generator. See docs/SPEC.md for the language definition and
docs/ROADMAP.md for the path toward the native (WASM/LLVM) toolchain.
"""

from __future__ import annotations

from .errors import ZownError
from .parser import parse
from .vm import VM, Block, run_source

__version__ = "0.1.0"

__all__ = ["ZownError", "parse", "VM", "Block", "run_source", "__version__"]
