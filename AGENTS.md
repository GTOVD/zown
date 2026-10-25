# AGENTS.md — working on Zown as an AI agent

Read this first; it is the operational summary. The long-form documents are:

| Doc | What it answers |
|-----|-----------------|
| [`docs/PLAN.md`](docs/PLAN.md) | **Where are we, what is next, what does done look like.** Status Board, milestone slices, open decisions, known debts. *Start here after this file.* |
| [`docs/SPEC.md`](docs/SPEC.md) | The language, normatively. Part I = v0.1 (runs everywhere). Part II = v0.2 (oracle + checker; each section marked *implemented* or *design-only*). |
| [`docs/DESIGN.md`](docs/DESIGN.md) | The whole-machine vision (sovereign substrate: capabilities, mesh, UI, store, AI control plane). Rationale, not a task list. |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | High-level phases. |
| [`docs/MANIFEST.md`](docs/MANIFEST.md) · [`docs/IR.md`](docs/IR.md) · [`docs/WASM.md`](docs/WASM.md) | Shadow manifest format · IR contract · WASM backend. |
| [`conformance/README.md`](conformance/README.md) | The golden test corpora and every runner. |

## What Zown is, in one paragraph

A stack-based, token-dense language meant to be written by machines. Literals
push, operators pop; `[ ... ]` is a first-class block; `:x` binds, `x` pushes a
copy (a *borrow*; or runs a builtin word), `'x` *moves* the value out and unbinds,
`@` invokes. Meaning lives in a sidecar *shadow manifest* (`file.zn.json`), not in
identifiers. Errors are structured `.zerr` packets with a recovery `code`, and
`[ body ] ty` turns one into a value (`ok(v)`/`err(code)`) that only `??` with
`[ok]`/`[err]` arms can open. `int` is a checked 64-bit integer (overflow is an
`OVERFLOW`, never silent); `b` makes an exact `big`. v0.2 also has capability
tokens (`` `t `` is net-send, zero authority by default, `gr`/`rq`/`hv`), fixed-width ints
(`u8 wr`, 128-bit is `ix`/`ux`), SIMD vectors (`[1 2 3 4] vi`, `va`/`vs`/`vm`), and pattern matching
(`x [ [int] [..] [any] [..] ] ??`). **`zown run` refuses any program the static
checker faults** (exit 2); there is no undefined behavior (SPEC §19).

## The three implementations and the one rule

1. **`zown/` — the Python reference interpreter. It is the oracle.** Its
   observable behavior *is* the semantics.
2. **`zownc/` — the Rust toolchain** (lexer, parser, IR, VM, checker, WASM,
   Cranelift). Differentially tested against the oracle. The VM and checker
   implement v0.2, including fast lanes (SPEC §22). Cranelift runs v0.2. WASM still refuses `??`, `ty`, `gr`, vectors, and the lane operators.
3. **The conformance corpus (`conformance/`)** — language-agnostic goldens that
   both must satisfy.

**The rule:** never change observable semantics without first updating the
conformance goldens and the SPEC. A backend that disagrees with the oracle is a
bug in the backend until the SPEC says otherwise.

## Environment

- **Python ≥ 3.9**, no packages. (`pyproject.toml` says 3.10; 3.9 passes the full
  suite because every module uses `from __future__ import annotations`.)
  - Windows dev machine (2026-09): the real interpreter is `C:\Python39\python.exe`
    (`python` on PATH is the Store stub). Run with `-X utf8` so `—` renders. Use
    `python -m zown.cli …` instead of `bin/zown`.
  - macOS/Linux: `python3 bin/zown …` or `pip install -e . && zown …`.
- **Rust + cargo (≥ 1.96)** only for `zownc/` and the parity runners. Not
  installed on the 2026-09 Windows machine; install via rustup when M8d starts.
- **wasmtime** only for `conformance/wasm_parity.py`.
- **git** on Windows: `C:\Program Files\Git\cmd\git.exe` (may not be on PATH).

## Verify (do this before and after any change — ~2 s)

```bash
python3 tests/test_zown.py          # unit tests
python3 tests/test_examples.py      # shipped examples
python3 conformance/run.py          # v0.1 goldens
python3 conformance/v2.py           # v0.2 goldens (oracle-only)
python3 conformance/check.py        # static-checker goldens (oracle-only)
# with Rust built (cd zownc && cargo build && cargo test):
python3 conformance/ast_parity.py && python3 conformance/vm_parity.py \
  && python3 conformance/ir_roundtrip.py && python3 conformance/wasm_parity.py \
  && python3 conformance/check_parity.py && python3 conformance/native_parity.py \
  && python3 conformance/stage_parity.py && python3 conformance/parse_parity.py \
  && python3 conformance/bare_boot.py
```

All five oracle commands must end in `0 failed` (128 unit tests, 20 +
64 + 14 goldens). `--bless` on a runner rewrites its goldens from the oracle —
only after an *intentional* semantics change, and commit the `.zn` with its
`.out`/`.code`/`.diag`. Beware when probing by hand: `zown run` is gated, so use
`--unchecked` to observe the *runtime* packet of a program the checker rejects,
and never leave a probe program running (a `;` loop whose condition always pushes
`1` never ends).

## How to do a slice (the only workflow)

Work is delivered in **slices** (`M8b`, `M7c-ii`, …), one commit each. The full
checklist is in `PLAN.md` → "How a slice is done"; the short form:

1. `PLAN.md` Status Board → find the ⏳ item and read its milestone section.
2. SPEC text first for anything observable (tokens are *provisional* until the
   density audit; anything the oracle cannot exhibit honestly is *design-only*).
3. Implement in `zown/`. Keep v0.1 byte-identical.
4. Unit tests (`tests/test_zown.py`, plain asserts) + conformance goldens.
5. Docs (SPEC section marked implemented, CLI help, MANIFEST/IR/WASM if touched).
6. Update `PLAN.md`: tick the slice, Status Board, "Immediate next actions",
   Open decisions / Known debts, "Last updated".
7. Commit: `M8c-i: result values in the oracle` style subject; body = semantics,
   files, what was not touched; `Co-authored-by: Cursor <cursoragent@cursor.com>`
   if an agent wrote it.

## Hard constraints

- **Checker soundness.** A `zown check` diagnostic is either a definite
  *unhandled* fault ("if this node executes, it raises a `.zerr` nothing catches"
  — inside `at` the fault is the handled path) or a violation of a static rule
  the SPEC mandates (unbound name, malformed `??` arms, non-exhaustive result
  handling, use after move). Never add a check that can fire on a program that
  would have run fine — since `run` is gated, a false positive is a false
  rejection. `test_check_corpus_has_no_false_positives` enforces this over the
  whole runnable corpus; keep it green. When unsure, stay silent.
- **No undefined behavior.** Every new runtime failure must be a `.zerr` (so `at`
  can catch it) with a row in SPEC §19 and a golden. The oracle must never die
  with a Python traceback on any input.
- **Overflow is never silent.** New int-producing words go through
  `vm.int_result`; float→int through `vm.int_from_float`; use `_numeric` for
  binary arithmetic so `big`/float promotion is uniform.
- **Density.** Prefer a symbol to a word, a 1–2 char word to a long one. A Zown
  identifier is `[A-Za-z][A-Za-z0-9]*` — it cannot contain operator characters
  (that is why vector ops are `va`/`vs`/`vm`, not `v+`). A library word is at most three characters and has one meaning.
- **Reserved symbols stay reserved** (`| ~ ^ ( ) { }`, `» « →`) — see SPEC §5/§17.
- **Never fabricate provenance/crypto.** Manifest `module` crypto fields stay
  `null` until M14 computes real hashes/signatures.
- **Don't touch Rust parity accidentally.** `vm_parity.py` scans v0.1 and v0.2
  (59). `ast_parity.py` / `ir_roundtrip.py` scan v0.2 as well. `wasm_parity.py`
  is still the 13 v0.1 cases — a v0.2 change must not alter a v0.1 golden.

## Layout cheat-sheet

```
zown/lexer.py     two-state lexer          zown/checker.py  static checker (M8)
zown/parser.py    tokens -> AST tuples     zown/manifest.py shadow manifest v2
zown/vm.py        stack VM + operators     zown/cli.py      `zown` command
zown/builtins.py  WORDS (stdlib)           zown/errors.py   .zerr packets + codes
tests/            unit + example tests     conformance/     goldens + runners
zownc/crates/     Rust: lexer ast parser ir vm wasm cli     docs/  see table above
```

AST nodes are tuples `(tag, value, pos)` with tags `int float str blk name bind
move cap op`; a `blk` value is a list of nodes. The VM's environment is a single
global dict (bindings are global, which is what makes static name resolution
sound). Runtime value kinds: `int float str Block Big Res Cap WidthTag Vec`
(`vm.type_name` is the type-pattern vocabulary).

## Current position (keep in sync with PLAN.md)

M0–M8 done. M8a name resolution + match shape
✅, M8b static capability flow ✅, **M8c ✅** (check-before-run gate, checked
`int` + `big`, result values `ty`, no-UB catalogue §19, `'x` moves). **M8d-i ✅** (Rust frontend: Cap/`'x`/`??`/checked int literals, AST spans,
`zownc ast` 45/45). **M8d-ii/iii ✅** (checked `int`, `big`, widths, vectors,
`??`, `ty`, caps; `vm_parity` 59/59). **M8d-iv/v ✅** (`Match`/`Attempt`, checked
WASM `int`, `zownc check` gates `run`; `check_parity` 14/14, `wasm_parity` 13/13).
**M9 ✅** — Cranelift int fast path, SIMD `va`/`vs`/`vm`, byte-identical
`native -o` exes, `native -O`, and `zownc llvm` (`clang -O2`). `buf` is
SPEC §21 and is not implemented. **M10 ✅** — cooperative fast lanes
(`~m` `~n` `^` `|`, `zown tune`). The trace is the timing model (SPEC §22).
WASM tags 4–8 are assigned and not emitted, and WASM
refuses the lane operators. **M11 ✅** — `mx`, `mn`, and `cl` are Zown in
`zown/std/core.zn`; `zown test` and `zown fmt` ship. Crypto, files, and LSP
are SPEC §23 and are not implemented. **M12 ✅** — BLAKE3 `h3`, local
`~p`/`~f`, and `zown mesh` (two IPv6 UDP peers on `[::1]`). QUIC, the DHT,
and Ed25519 are not implemented. **M13 ✅** — layout tree (`co`, `tx`,
`bt`, `rw`, `cm`, `ly`) and `zown draw` (PPM). GPU, audio, and input
delivery are not implemented. **M14 ✅** — in-memory graph (`gv`, `ge`,
`gs`, `gl`, `gh`, `sb`, `so`, `»`, `«`, `→`) and `zown keep` (fsynced
BLAKE3 bundle, `INTEGRITY_FAIL` on a changed byte). mmap, CRDTs, Ed25519,
and hot-swap are not implemented. **M15 ✅** — logical control-plane log
(`ch`, `vr`, `td`, `sp`, `se`, `nt`, `wn`, `ob`, `df`, `rm`, `lg`) and
`zown watch`. No clock. A fleet, hot-swap, and on-device ML are not
implemented. **M16 ✅** — `zown/std/lex.zn` is the lexer. `zown stage`
checks its token dump against the host, including on its own source.
`stage_parity` runs that program natively. A stage-2 compiler binary is
not produced. **M17 ✅** — `zownc bare` writes a 512-byte real-mode boot
sector. QEMU reads `Zown` from the VGA text buffer, and `int 0x20` stores
`0xB1` at `0x8000`. A microkernel is not implemented. **M18 ✅** —
`bf` `bg` `bp` are an exclusive `buf`. A bare name or `=` pushes a
borrow. `,` on the owner frees the bytes. No address is visible.
`native_parity` is 84/84. **M19 ✅** — `zown/std/parse.zn` is the parser.
`zown stage parse` checks its AST against the host, including on its own
source. A leaf keeps the raw lexeme (`int 007`, `float 1.50`). `parse_parity`
runs that program natively (5/5). A stage-2 compiler binary is not produced.
Next is **M20** (host I/O behind capabilities). The byte-identical stage-2
compiler is M23. The rest of the map is **M18–M32** in `docs/PLAN.md`.
On Windows without Visual C++, `cargo` needs
`RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu`. No maintainer decisions are
open; the decision log is in `PLAN.md`.
