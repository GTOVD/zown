# Zown WASM backend

`zown-wasm` lowers Zown IR to WebAssembly — both text (`.wat`) and binary
(`.wasm`) — that runs in `wasmtime` (and any WASM/WASI runtime). This is
milestone **M6**, built in slices; as of **M6d it is complete**: all 13
conformance cases compile and match the goldens under wasmtime, in both formats.

## Try it

```bash
# requires wasmtime (https://wasmtime.dev) and a cargo build of zownc
zownc build conformance/cases/fizzbuzz.zn -o fb.wasm   # binary (.wasm extension)
wasmtime run fb.wasm
zownc build conformance/cases/fib.zn -o fib.wat        # text (any other extension)
wasmtime run fib.wat
zownc wat conformance/cases/hello.zn                    # print the .wat to stdout
python3 conformance/wasm_parity.py                      # all cases, .wat + .wasm
```

## Slices

| Slice | Scope | State |
|-------|-------|-------|
| **M6a** | integers: literals, `+ - * % _`, comparisons, `&& \|\| !`, and `.` | ✅ |
| **M6b** | tagged values + strings: `$...$`, `+`/`*` on strings, `tr/up/lo/ln/rv`, stack ops `= , \ & rt` | ✅ |
| **M6c** | blocks + control: `[ … ]`, `@ ? ;`, `:bind` / name load (in-memory stack + `call_indirect`) | ✅ |
| **M6d** | floats + math words (`/ sq pw fl ce rd`, `n s dp clr pr`); binary `.wasm` emission | ✅ **all 13 cases green** |

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

## Floats and binary emission (M6d)

Floats use tag `1`, storing the `f64` **bit pattern** in the `i64` payload
(`i64.reinterpret_f64` / `f64.reinterpret_i64`). Numeric operators promote int →
float exactly like the interpreter: `+ - *` are int-only when both operands are
ints, else `f64`; `/` is always `f64` then collapsed; `%` mirrors Python's
divisor-signed modulo for both. Math words map to native ops where possible —
`sq`→`f64.sqrt`, `fl`→`f64.floor`, `ce`→`f64.ceil`, `rd`→`f64.nearest`
(round-half-to-even, like Python `round`) — with `pw` using an integer fast path
(`$ipow`) and `ab`/`mx`/`mn` preserving the operand's type. `n` parses a string
to a number; `s` renders one; `dp`/`clr` use the stack pointer; `pr` prints
without a newline.

**Float formatting.** `$fmt_f64` applies the display rule (whole-valued finite
floats render as integers, e.g. `7.0` → `7`) and otherwise emits the integer part
then a terminating long-division expansion of the fraction. This is *exact* for
dyadic values — the only fractional float the suite prints is `2.5` — but is not
the shortest round-tripping representation for values like `0.1`. A Ryu/Grisu
formatter is future work.

**Binary `.wasm`.** `emit_wasm` assembles the `.wat` and encodes it with the
`wat` crate. `zownc build -o x.wasm` writes a binary module (`\0asm` …); any other
extension writes `.wat`. Both run identically under wasmtime, verified by
`conformance/wasm_parity.py` (each case is checked in both formats).

## Next: M7 — native backend

Real desktop binaries via Cranelift (then LLVM). The IR and the tagged-value
model carry over; the operand-stack/heap/string-runtime design is reused with a
native calling convention instead of WASI.
