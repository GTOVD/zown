# zownc — native Zown toolchain (Rust)

This is the **stage-0** native toolchain for Zown, being built per
[`../docs/PLAN.md`](../docs/PLAN.md). The Python package in `../zown/` is the
behavioral **oracle**; everything here is differentially tested against it.

## Status

| Piece | State |
|-------|-------|
| `zown-lexer` | ✅ ported from the Python reference, unit-tested |
| `zown-ast` / `zown-parser` | ✅ AST parity with the oracle (`zownc ast`) |
| `zown-vm` | ✅ tree-walking VM; `zownc run` matches the oracle (20/20) |
| `zown-ir` | ✅ IR + lowering; lossless round-trip (`zownc ir` / `irast`) |
| `zown-wasm` | 🔄 M6b: tagged values + strings compile to `.wat`, run in wasmtime |
| native backend | ⬜ (M7) |

## Build & run

```bash
cargo build
cargo test
./target/debug/zownc ../examples/fizzbuzz.zn      # run (shorthand)
./target/debug/zownc run ../examples/hello.zn
./target/debug/zownc run --zerr broken.zn         # JSON .zerr packet on error
./target/debug/zownc ast ../examples/hello.zn
./target/debug/zownc lex ../examples/hello.zn

# WASM backend (needs wasmtime):
printf '$foo$ $bar$ + . $ab$ 3 * .' > /tmp/p.zn
./target/debug/zownc build /tmp/p.zn -o /tmp/p.wat && wasmtime run /tmp/p.wat
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
  zown-wasm/    WASM backend: IR -> .wat (tagged values + strings; blocks next)
  zown-cli/     the `zownc` binary (run / lex / ast / ir / irast / wat / build)
```

The native code crate (`zown-codegen`) is added when M7 begins.

## The endgame

Once the native toolchain can compile Zown to a runnable binary (M6/M7) and the
memory model is in place (M8), we begin rewriting the compiler **in Zown itself**
and bootstrap to a self-hosting fixed point (M14). At that point this Rust crate
becomes the bootstrap stage and a differential oracle.
