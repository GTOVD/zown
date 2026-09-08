# Conformance suite

Language-agnostic golden tests that pin down Zown's observable behavior. The
Python reference VM is the **oracle**; the native `zownc` toolchain is validated
against these same `.zn` files as it is built (see `docs/PLAN.md`).

## Layout

```
cases/<name>.zn     + cases/<name>.out      # v0.1 program -> expected stdout
errors/<name>.zn    + errors/<name>.code    # v0.1 program -> expected "<CODE>" or "<CODE> <op>"
cases_v2/, errors_v2/                       # v0.2 (Part II) corpus, same shapes — oracle-only today
check/<name>.zn     + check/<name>.diag     # static checker: "<CODE> <op> [<cap>]" per line, or "ok"

run.py            oracle runner for cases/ + errors/      (golden check / --bless)
v2.py             oracle runner for cases_v2/ + errors_v2/ (golden check / --bless)
check.py          oracle runner for check/                 (golden check / --bless)
ast_parity.py     diff Python AST vs `zownc ast`                (needs cargo build)
vm_parity.py      run cases/ + errors/ through `zownc run`      (needs cargo build)
ir_roundtrip.py   assert `zownc irast` == `zownc ast`           (needs cargo build)
wasm_parity.py    build + run cases/ under wasmtime, .wat/.wasm (needs cargo build + wasmtime)
```

## Commands

```bash
# oracle goldens (no dependencies)
python3 conformance/run.py       # v0.1 programs + errors
python3 conformance/v2.py        # v0.2 programs + errors
python3 conformance/check.py     # static-checker diagnostics

# regenerate goldens from the oracle after an INTENTIONAL semantics change
python3 conformance/run.py --bless      # likewise v2.py / check.py

# native toolchain parity (from the repo root; requires `cd zownc && cargo build`)
python3 conformance/ast_parity.py
python3 conformance/vm_parity.py
python3 conformance/ir_roundtrip.py
python3 conformance/wasm_parity.py      # also requires wasmtime on PATH
```

## Adding a case

1. Drop a program in the right directory: `cases*/` (prints something), `errors*/`
   (must fault at runtime), or `check/` (exercise the static checker — clean
   programs belong here too; they pin the checker's *no-false-positives* rule).
2. Run the matching runner with `--bless` to capture the golden.
3. Commit the `.zn` and its `.out`/`.code`/`.diag` together.

## Coverage today

- **v0.1** (`cases/`, `errors/`): every operator, every stdlib word, select/while/
  invoke control flow; one error per recovery code: `STACK_UNDERFLOW`, `DIV_ZERO`,
  `NAME_UNRESOLVED`, `NOT_CALLABLE`, `TYPE_MISMATCH`, `REPAIR_SYNTAX` (lex + parse).
- **v0.2** (`cases_v2/`, `errors_v2/`): capabilities (`gr rq hv`, `CAP_DENIED`),
  fixed-width ints (`wr st ck`, `OVERFLOW`), SIMD vectors (`vadd vsub vmul vsum
  vat`, `BOUNDS`), pattern matching (`??`, `NO_MATCH`, `BAD_PATTERN`), the
  checked-int / `big` / IEEE-float numeric model (`b`, `OVERFLOW` on int results
  and literals, `inf`/`nan` values, `UB_TRAP` on nan→int and negative `sq`).
  Also: result values (`at`, `[ok]`/`[err]`, `NO_MATCH`, `BOUNDS`), linear
  bindings (`'x`), and the SPEC §19 no-UB rows (`STACK_OVERFLOW`, float
  `DIV_ZERO`, `vec_oob`, `defined_behavior`).
- **check** (`check/`): name resolution, `??` arm shape, capability flow, result
  exhaustiveness, and use-after-move — both the definite findings and the
  sound-silence cases (dynamic extent, escaped blocks, inline consumers, faults
  inside `at`, rebinding, the `;` body).

The Rust parity runners scan `cases/`+`errors/` only: `zownc` does not implement
v0.2 or the checker yet (PLAN M8d). When it does, `v2.py`'s and `check.py`'s
corpora become its targets too.
