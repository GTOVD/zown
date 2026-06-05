# Zown WASM backend

`zown-wasm` lowers Zown IR to WebAssembly text (`.wat`) that runs in `wasmtime`
(and, once binary emission lands, any WASM runtime / the browser). This is
milestone **M6**, built in slices so coverage grows visibly against the
conformance goldens.

## Try it

```bash
# requires wasmtime (https://wasmtime.dev) and a cargo build of zownc
printf '2 3 + . 10 20 * . 2 5 + _ .' > prog.zn
zownc build prog.zn -o prog.wat     # compile
wasmtime run prog.wat               # -> 5 / 200 / -7
zownc wat prog.zn                   # print the .wat to stdout
python3 conformance/wasm_parity.py  # run the supported subset under wasmtime
```

## Slices

| Slice | Scope | State |
|-------|-------|-------|
| **M6a** | integers: literals, `+ - * % _`, comparisons, `&& \|\| !`, and `.` | ✅ runs in wasmtime; `compare` + `logic` match goldens |
| M6b | strings: `$...$`, `+`/`*` on strings, string words (`tr/up/lo/ln/rv`) | ⬜ |
| M6c | blocks + control: `@ ? ;`, `:bind` / name load (tagged values, `call_indirect`) | ⬜ |
| M6d | floats + remaining math words; binary `.wasm` emission | ⬜ |

## How M6a maps to WASM

The integer subset has no blocks, bindings, strings, or floats, so every value is
an `i64` and maps directly onto the WASM value stack — no in-memory tagged-value
runtime is needed yet:

- `ConstInt(v)` → `i64.const v`
- `+ - *` → `i64.add/sub/mul` (operands popped into locals `$a`,`$b` for clarity)
- `%` → `$zmod` helper (Python-style: result takes the divisor's sign)
- `_` → `(i64.sub (i64.const 0) x)`
- comparisons → `i64.{eq,ne,lt_s,gt_s,le_s,ge_s}` then `i64.extend_i32_u` (Zown
  pushes `1`/`0`)
- `&& || !` → truthiness via `i64.ne 0` / `i64.eqz`, combined, then extended
- `.` → `$print_i64`: an `itoa` loop into linear memory + WASI `fd_write` to fd 1

Codegen tracks the compile-time stack depth, emits `drop`s for any leftover
values so `$main` validates, and rejects unsupported constructs with a clear
message naming the slice that will add them.

## The tagged-value runtime (M6b+)

Strings and blocks need a heap and dynamic typing. The plan: each value becomes a
`(tag, payload)` slot in a linear-memory operand stack; strings are
`[len][bytes]`; blocks are function-table indices invoked via `call_indirect`.
Most operators then become calls to small runtime functions emitted once per
module. This is the substantive design step for the next slice.
