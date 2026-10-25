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
| **M6b** | tagged values + strings: `$...$`, `ct`/`rp`, `tr/up/lo/ln/rv`, stack ops `= , \ & rt` | ✅ |
| **M6c** | blocks + control: `[ … ]`, `@ ? ;`, `:bind` / name load (in-memory stack + `call_indirect`) | ✅ |
| **M6d** | floats + math words (`/ sq pw fl ce rd`, `n s dp cl pr`); binary `.wasm` emission | ✅ **all 13 cases green** |

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
- `+` / `*` → numeric add and multiply (int, or float when either operand is a
  float). A string operand traps. Concatenation is the word `ct` (`$str_concat`
  of two string payloads); repeat is the word `rp` (`$str_repeat`)
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
to a number; `s` renders one; `dp`/`cl` use the stack pointer; `pr` prints
without a newline.

**Float formatting.** `$fmt_f64` prints non-finite values as `inf`, `-inf`, and
`nan`. A whole finite value that fits `int` prints as that int (`7.0` → `7`).
Larger magnitudes use Python's scientific form (`1e+30`). Dyadic fractions such
as `2.5` still expand by long division. A full shortest-round-trip formatter
for values like `0.1` is not in this backend yet.

**Checked `int` (M22).** Integer `+ - * _` and the integer `pw` path call
`$iadd` / `$isub` / `$imul` / `$ineg`. A result outside `i64` writes a `.zerr`
(`OVERFLOW`) and `proc_exit`s, unless it is inside `ty`, which turns the same
fault into `err(OVERFLOW)`. Division by zero is `DIV_ZERO`, for ints, bigs,
and floats. `wasm_parity` checks those packets against `errors/` and
`errors_v2/`.

**Binary `.wasm`.** `emit_wasm` assembles the `.wat` and encodes it with the
`wat` crate. `zownc build -o x.wasm` writes a binary module (`\0asm` …); any other
extension writes `.wat`. Both run identically under wasmtime, verified by
`conformance/wasm_parity.py` (each case is checked in both formats).

## v0.2 tags (decision D8, 2026-10-24)

The tag is still the `i64` under the payload. Tags 0–3 are unchanged. The rest
are allocated so a later emitter cannot pick a colliding number. A payload that
does not fit in one `i64` is an index into a side table in the module, not the
bytes themselves.

| tag | type | payload |
|-----|------|---------|
| `0` | int | the `i64` |
| `1` | float | `f64` bit pattern |
| `2` | str | pointer to `[i32 len][bytes…]` |
| `3` | block | funcref table index |
| `4` | big | pointer to a decimal `[i32 len][digits]` (optional leading `-`) |
| `5` | cap | pointer to the capability name, length-prefixed like a string |
| `6` | width | `(signed << 16) \| bit_width` (`ix`/`ux` are bit width 128) |
| `7` | vec | pointer to kind, count, then one `i64` lane each (`vi`/`vf`/`vd`/`vu`) |
| `8` | res | pointer to `i32 ok`, then the inner tag and payload |

M22 emits these. `??` reads a side table of block instructions (patterns are
not executed). `ty` runs on a saved stack and catches faults as `err(CODE)`.
`'name` clears a per-binding flag; an unbound name falls through to the builtin.
Vector arithmetic is scalar calls, not WASM SIMD opcodes. Lane operators
(`|`, `^`, `~name`), buffers, files, crypto, the graph, and the mesh still
compile to `UNSUPPORTED` and `wasm_parity` records that skip. They are not
given a guessed value.
