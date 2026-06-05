# zownc — native Zown toolchain (Rust)

This is the **stage-0** native toolchain for Zown, being built per
[`../docs/PLAN.md`](../docs/PLAN.md). The Python package in `../zown/` is the
behavioral **oracle**; everything here is differentially tested against it.

## Status

| Piece | State |
|-------|-------|
| `zown-lexer` | ✅ ported from the Python reference, unit-tested |
| `zownc lex`  | ✅ tokenizes a file |
| parser / AST | ⬜ next (M3) |
| tree-walking VM | ⬜ (M4) |
| IR + WASM/native backends | ⬜ (M5–M7) |

## Build & run

```bash
cargo build
cargo test
./target/debug/zownc lex ../examples/hello.zn
./target/debug/zownc --help
```

## Layout

```
crates/
  zown-lexer/   two-state tokenizer (Pos, Token, TokenKind, lex)
  zown-cli/     the `zownc` binary
```

New crates (`zown-parser`, `zown-ast`, `zown-ir`, `zown-vm`, `zown-wasm`,
`zown-codegen`) are added as their milestones begin.

## The endgame

Once the native toolchain can compile Zown to a runnable binary (M6/M7) and the
memory model is in place (M8), we begin rewriting the compiler **in Zown itself**
and bootstrap to a self-hosting fixed point (M14). At that point this Rust crate
becomes the bootstrap stage and a differential oracle.
