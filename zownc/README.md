# zownc — native Zown toolchain (Rust)

This is the **stage-0** native toolchain for Zown, being built per
[`../docs/PLAN.md`](../docs/PLAN.md). The Python package in `../zown/` is the
behavioral **oracle**; everything here is differentially tested against it.

## Status

| Piece | State |
|-------|-------|
| `zown-lexer` | ✅ v0.2 tokens: Cap, Move (`'x`), `??`, int-literal `OVERFLOW` |
| `zown-ast` / `zown-parser` | ✅ AST parity on v0.1+v0.2+check (`zownc ast`, 45/45); every node has a `Pos` |
| `zown-vm` | ✅ v0.1 run parity (20/20); Cap/`'x` nodes execute; words/numerics still v0.1 (M8d-ii) |
| `zown-ir` | ✅ + `ConstCap`/`Move`; lossless on the 45-program corpus |
| `zown-wasm` | ✅ M6: full v0.1; v0.2 cap/move instrs are a structured `UNSUPPORTED` |
| remaining v0.2 (checked int, `big`, `at`/`??` unwrap, words, `zown-check`) | 🔄 M8d-ii…v |
| native backend | ⬜ (M9, after M8d) |

## Build & run

```bash
cargo build
cargo test
./target/debug/zownc ../examples/fizzbuzz.zn      # run (shorthand)
./target/debug/zownc run ../examples/hello.zn
./target/debug/zownc run --zerr broken.zn         # JSON .zerr packet on error
./target/debug/zownc ast ../examples/hello.zn
./target/debug/zownc lex ../examples/hello.zn

# WASM backend (needs wasmtime): full language -> binary .wasm or text .wat
./target/debug/zownc build ../conformance/cases/fizzbuzz.zn -o /tmp/fb.wasm
wasmtime run /tmp/fb.wasm
```

Verify parity with the Python oracle (from the repo root):

```bash
python3 conformance/ast_parity.py   # frontend parity
python3 conformance/vm_parity.py    # full run parity (stdout + error codes)
python3 conformance/wasm_parity.py  # WASM backend under wasmtime
```

## Layout

```
crates/
  zown-lexer/   two-state tokenizer (Pos, Token, TokenKind, lex)
  zown-ast/     Node enum + JSON rendering (parity with `zown ast`)
  zown-parser/  tokens -> AST (+ structured parse errors)
  zown-vm/      stack VM: Value, RunError/.zerr, operators, stdlib WORDS
  zown-ir/      IR: Instr/IrBlock/IrProgram, lower/unlower, pretty
  zown-wasm/    WASM backend: IR -> .wat + binary .wasm (full v0.1)
  zown-cli/     the `zownc` binary (run / lex / ast / ir / irast / wat / build)
```

Next crates, in order: `zown-check` (M8d — port of `zown/checker.py`, validated
against `conformance/check/`), then `zown-codegen` (M9, Cranelift → LLVM).

> `ast_parity.py` and `ir_roundtrip.py` now scan v0.1 + v0.2 + `check/` (45
> programs). `vm_parity.py` / `wasm_parity.py` still scan v0.1 only until
> M8d-ii/iv. Milestone numbers follow `docs/PLAN.md`.

## The endgame

Once the native toolchain can compile Zown to a runnable binary (M9) and the
memory model is in place (M8), we begin rewriting the compiler **in Zown itself**
and bootstrap to a self-hosting fixed point (M16). At that point this Rust crate
becomes the bootstrap stage and a differential oracle.
