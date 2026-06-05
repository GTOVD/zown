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
| **M6b** | tagged values + strings: `$...$`, `+`/`*` on strings, `tr/up/lo/ln/rv`, stack ops `= , \ & rt` | ✅ `compare`, `logic`, `stackops`, `strings`, `words_str` match goldens |
| M6c | blocks + control: `@ ? ;`, `:bind` / name load (`call_indirect` + function table) | ⬜ |
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
`\HH`-escaped); dynamic strings are bump-allocated from a heap (`$hp`) that starts
just past the literals. A small fixed runtime prelude is emitted once per module.

### Memory map

| range | use |
|-------|-----|
| `[0, 8)` | iovec for `fd_write` (ptr@0, len@4) |
| `16` | scratch newline byte |
| `[44, 64)` | `itoa` scratch (digits written backwards, ending at 64) |
| `[1024, …)` | string-literal data segment |
| heap | bump allocator `$hp`, just past the literals |

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

String helpers use the `memory.copy` bulk instruction. Codegen tracks the
compile-time stack depth, emits `drop`s for leftovers, and rejects unsupported
constructs with a clear message naming the slice that will add them.

## Next: M6c

Blocks (`[ … ]`) become entries in a WASM function table; `@` / `?` / `;` invoke
them via `call_indirect`, and `:bind` / name load read from a binding frame. That
unlocks `hello`, `select`, `while`, `fib`, and `fizzbuzz`.
