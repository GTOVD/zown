# Conformance suite

Language-agnostic golden tests that pin down Zown v0.1's observable behavior. The
Python reference VM is the **oracle**; the native `zownc` toolchain is validated
against these same `.zn` files as it is built (see `docs/PLAN.md`).

## Layout

```
cases/<name>.zn    + cases/<name>.out     # program -> expected stdout
errors/<name>.zn   + errors/<name>.code   # program -> expected "<CODE>" or "<CODE> <op>"
run.py                                    # oracle runner (golden check / --bless)
ast_parity.py                             # diff Python AST vs `zownc ast`
```

## Commands

```bash
# check programs + errors against goldens (oracle)
python3 conformance/run.py

# regenerate goldens from the oracle after an intentional semantics change
python3 conformance/run.py --bless

# verify the Rust frontend parses identically to the oracle
python3 conformance/ast_parity.py        # requires: (cd zownc && cargo build)
```

## Adding a case

1. Drop a program in `cases/` (prints something) or `errors/` (must fault).
2. Run `python3 conformance/run.py --bless` to capture the golden.
3. Commit the `.zn` and its `.out`/`.code` together.

## Coverage today

- Every operator, every stdlib word, select/while/invoke control flow.
- One error per recovery code: `STACK_UNDERFLOW`, `DIV_ZERO`, `NAME_UNRESOLVED`,
  `NOT_CALLABLE`, `TYPE_MISMATCH`, and `REPAIR_SYNTAX` (lex + parse).
