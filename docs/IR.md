# Zown IR

The IR is the contract between the frontend and the backends. The AST is
tree-shaped and convenient for parsing; the IR is **flat and addressable**, which
is what code generators (WASM, native) want.

Defined in `zownc/crates/zown-ir`. Inspect it with `zownc ir <file.zn>`.

## Shape

```
IrProgram { blocks: Vec<IrBlock>, main: usize }
IrBlock(Vec<Instr>)
```

- A program is a **table of blocks** plus the id of the entry (`main`) block.
- Every `[ ... ]` quotation is lowered to its own block; the place it appeared
  becomes a `ConstBlock(id)` that pushes a reference to it. This mirrors the VM's
  treatment of blocks as first-class values, and gives codegen one function per
  block to emit.

## Instructions

| Instr           | From (AST)         | Meaning                                   |
|-----------------|--------------------|-------------------------------------------|
| `ConstInt(i64)` | int literal        | push an integer                           |
| `ConstFloat`    | float literal      | push a float                              |
| `ConstStr`      | `$...$`            | push a string                             |
| `ConstBlock(id)`| `[ ... ]`          | push a reference to block `id`            |
| `Bind(name)`    | `:name`            | pop and bind `name`                       |
| `Load(name)`    | `name`             | resolve name: user binding, else builtin  |
| `Op(sym)`       | value operators    | `+ - * / % _ == != < > <= >= && \|\| ! = , \ & .` |
| `Invoke`        | `@`                | pop a block and run it                     |
| `Select`        | `?`                | choose a block by a condition              |
| `While`         | `;`                | `[cond] [body] ;`                          |

The three control words (`@ ? ;`) get dedicated instructions because backends
must treat them specially (calls, branches, loops); all other operators are
uniform `Op(sym)`.

## Example

`1 [$y$] [$n$] ? @ .` lowers to:

```
main = b4
b0:  push.s "y"
b1:  push.s "n"
b4:  push.i 1
     push.blk b0
     push.blk b1
     select
     invoke
     op .
```

## Lowering is lossless

`lower(ast)` flattens; `unlower(ir)` rebuilds the **exact** AST. Because the AST
frontend already matches the Python oracle (M3), an exact round-trip proves the
IR represents the program faithfully — the property every backend relies on.

```bash
zownc irast file.zn            # lower -> unlower -> AST JSON
python3 conformance/ir_roundtrip.py   # asserts irast == ast across the corpus
```

## Why not a typed SSA IR yet?

Zown v0.1 is dynamically typed (int/float/str/block share one stack). The first
backends (M6 WASM) target a small **tagged-value runtime**, for which this linear
stack IR is a direct fit. A typed/SSA lowering is a later optimization pass once
the type/memory model (M8) gives us static types to lower from.
