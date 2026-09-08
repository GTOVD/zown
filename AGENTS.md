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
push, operators pop; `[ ... ]` is a first-class block; `:x` binds, `x` pushes (or
runs a builtin word), `@` invokes. Meaning lives in a sidecar *shadow manifest*
(`file.zn.json`), not in identifiers. Errors are structured `.zerr` packets with a
recovery `code`. v0.2 adds capability tokens (`` `s ``, zero authority by default,
`gr`/`rq`/`hv`), fixed-width ints (`u8 wr`), SIMD vectors (`[1 2 3 4] i4`), and
pattern matching (`x [ [int] [..] [_] [..] ] ??`). A static checker (`zown check`)
runs before execution.

## The three implementations and the one rule

1. **`zown/` — the Python reference interpreter. It is the oracle.** Its
   observable behavior *is* the semantics.
2. **`zownc/` — the Rust toolchain** (lexer, parser, IR, VM, WASM backend).
   Differentially tested against the oracle. Currently implements **v0.1 only**.
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
  && python3 conformance/ir_roundtrip.py && python3 conformance/wasm_parity.py
```

All five oracle commands must end in `0 failed`. `--bless` on a runner rewrites
its goldens from the oracle — only after an *intentional* semantics change, and
commit the `.zn` with its `.out`/`.code`/`.diag`.

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

- **Checker soundness.** A `zown check` diagnostic means *"if this node executes,
  it definitely faults."* Never add a check that can fire on a program that would
  have run fine. `test_check_corpus_has_no_false_positives` enforces this over the
  whole runnable corpus; keep it green. When unsure, stay silent.
- **Density.** Prefer a symbol to a word, a 1–2 char word to a long one. A Zown
  identifier is `[A-Za-z][A-Za-z0-9]*` — it cannot contain operator characters
  (that is why vector ops are `vadd`, not `v+`).
- **Reserved symbols stay reserved** (`| ~ ^ ( ) { }`, `» « →`) — see SPEC §5/§17.
- **Never fabricate provenance/crypto.** Manifest `module` crypto fields stay
  `null` until M14 computes real hashes/signatures.
- **Don't touch Rust parity accidentally.** Until M8d, the Rust runners scan the
  v0.1 corpus only; a v0.2 change must not alter any v0.1 golden.

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
cap op`; a `blk` value is a list of nodes. The VM's environment is a single
global dict (bindings are global, which is what makes static name resolution
sound).

## Current position (keep in sync with PLAN.md)

M0–M7 done. **M8 (safety core) in progress:** M8a name resolution + match shape
✅, M8b static capability flow ✅. **Next: M8c-i result values** (`Res`, `at`,
`[ok]`/`[err]` arms), then M8c-ii the no-UB catalogue, then M8d Rust reaches
v0.2. Open decisions needing the maintainer are listed in `PLAN.md` → "Open
decisions" (D1 integer semantics is the big one).
