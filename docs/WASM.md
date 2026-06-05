# Zown WASM backend

`zown-wasm` lowers Zown IR to WebAssembly text (`.wat`) that runs in `wasmtime`
(and, once binary emission lands, any WASM runtime / the browser). This is
milestone **M6**, built in slices so coverage grows visibly against the
conformance goldens.

## Try it

```bash
# requires wasmtime (https://wasmtime.dev) and a cargo build of zownc
printf '$foo$ $bar$ + . $ab$ 3 * .' > prog.zn
zownc build prog.zn -o prog.wat     # compile
wasmtime run prog.wat               # -> foobar / ababab
zownc wat prog.zn                   # print the .wat to stdout
python3 conformance/wasm_parity.py  # run the supported subset under wasmtime
```

## Slices

| Slice | Scope | State |
|-------|-------|-------|
| **M6a** | integers: literals, `+ - * % _`, comparisons, `&& \|\| !`, and `.` | ✅ runs in wasmtime |
| **M6b** | tagged values + strings: `$...$`, `+`/`*` on strings, `tr/up/lo/ln/rv`, stack ops `= , \ & rt` | ✅ matches goldens |
| **M6c** | blocks + control: `[ … ]`, `@ ? ;`, `:bind` / name load (in-memory stack + `call_indirect`) | ✅ `hello`, `select`, `while`, `fib`, `fizzbuzz` match goldens (10/13 cases) |
| M6d | floats + remaining math words (`/ sq pw fl ce …`); binary `.wasm` emission | ⬜ |

## The tagged-value model (M6b)

Every Zown value is a pair of `i64`s on the WASM value stack — a **tag** (deeper)
and a **payload** (on top):

| tag | type | payload |
|-----|------|---------|
| `0` | int | the `i64` value |
| `1` | float | the `f64` bit pattern *(reserved, M6d)* |
| `2` | str | pointer into linear memory |
| `3` | block | block-table index *(reserved, M6c)* |

Strings live in linear memory as `[i32 len][bytes…]`; the pointer addresses the
length field. Literals are emitted into a data segment (length-prefixed,
`\HH`-escaped); dynamic strings are bump-allocated from a heap (`$hp`). A small
fixed runtime prelude is emitted once per module.

As of **M6c**, the operand stack also lives in linear memory (global `$sp`, 16
bytes per value: `[i64 tag][i64 payload]`), reached through `$push` / `$pop`. The
WASM value stack is only used for transient scratch within a single instruction.
This is what lets blocks — compiled to separate functions — share one stack.

### Memory map

| range | use |
|-------|-----|
| `[0, 8)` | iovec for `fd_write` (ptr@0, len@4) |
| `16` | scratch newline byte |
| `[44, 64)` | `itoa` scratch (digits written backwards, ending at 64) |
| `[1024, …)` | string-literal data segment |
| operand stack | 16 B/value, `$sp`, just past the literals (64 KiB) |
| heap | bump allocator `$hp`, just past the operand stack |

### Lowering

- `ConstInt(v)` → push `(0, v)`; `ConstStr(s)` → push `(2, litptr)`
- `+` → if either operand is a string: `$str_concat($sval(a), $sval(b))` (ints are
  rendered to decimal via `$str_from_int`); else integer add
- `*` → if `str * int`: `$str_repeat`; else integer mul
- `- % _ < > <= >=` → integer ops; `==`/`!=` compare strings by content
  (`$str_eq`) and ints by value, with mismatched tags unequal
- `&& || !` → `$truthy` (ints nonzero, strings nonempty), combined
- `.` → `$print_value`: strings write their bytes; ints go through `$print_i64`;
  both get a trailing newline
- stack ops `= , \ &` (dup/drop/swap/over) and `rt` (rotate) shuffle the
  `(tag,payload)` pairs through locals
- words: `tr`→`$str_trim`, `up`/`lo`→`$str_case`, `rv`→`$str_reverse`,
  `ln`→ the string's length field

String helpers use the `memory.copy` bulk instruction.

## Blocks, control flow, and bindings (M6c)

Each IR block becomes a `() -> ()` WASM function `$blkN`, and the program's block
table is mirrored in a `funcref` table (`(elem …)`), so a block value is just its
table index (tag `3`). Control flow dispatches by that index:

- `ConstBlock(id)` → push `(3, id)`
- `@` (invoke) → pop a block, `call_indirect (type $bt)` with its index
- `?` (select) → pop else/then blocks and a condition, push the chosen block via
  the WASM `select` instruction (then it's usually `@`-invoked)
- `;` (while) → pop body and cond blocks, then a `block`/`loop`: invoke cond,
  `pop` its result, `br_if` out when falsy, else invoke body and loop
- `:name` (bind) → pop the top value into a pair of mutable globals `$gNt`/`$gNp`
- name load → if the name was bound anywhere, push its globals (user bindings
  shadow builtins, matching the VM's `env`-first lookup); otherwise it's a word

Globals are shared across all block functions, so bindings persist across
invocations exactly like the interpreter's environment. Truthiness treats blocks
as always-true.

## Next: M6d

Floats (`/ sq pw fl ce rd`, float literals) using tag `1` with the `f64` bit
pattern, the remaining math words, and binary `.wasm` emission (e.g. via
`wasm-encoder`) alongside `.wat`. Unlocks `arith`, `convert`, `words_math`.
