# Zown

**An AI-native, token-dense, stack-based programming language.**

Zown is built for machines to read and write, not humans. Keywords are single
ASCII characters, the standard library uses 1–2 character words, and the
stack-based model means programs rarely need variable names at all. The goal: fit
*dramatically* more working code inside an LLM's context window, while compiling
(eventually) straight to bare-metal and WebAssembly speed.

> This repo is the **v0.1 reference implementation** — a complete, runnable
> language (lexer → parser → stack VM → stdlib → CLI) that nails down the
> semantics. The native WASM/LLVM compiler, concurrency fast-lanes, embedded
> graph DB, hot-swap, and self-healing runtime are designed and phased in
> [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Hello, World

```
[$Hello, World!$.]:h h@
```

That's the whole program. `[ ... ]` makes a block, `:h` binds it to `h`, `h`
pushes it, and `@` runs it.

## Quickstart

No dependencies — just Python 3.10+.

```bash
# run a program
python3 bin/zown examples/hello.zn
python3 bin/zown examples/fizzbuzz.zn

# parse/lint only
python3 bin/zown check examples/fib.zn

# get a machine-readable error packet
python3 bin/zown --zerr examples/hello.zn

# generate the shadow manifest (meaning for the dense symbols)
python3 bin/zown manifest examples/fib.zn   # writes examples/fib.zn.json

# interactive stack session
python3 bin/zown repl
```

Install the `zown` command for real:

```bash
pip install -e .
zown examples/fizzbuzz.zn
```

## A taste of the language

```
# FizzBuzz 1..15
1:i
[ i 16 < ] [
  i 15 % 0 == [ $FizzBuzz$ ] [
    i 3 % 0 == [ $Fizz$ ] [
      i 5 % 0 == [ $Buzz$ ] [ i ] ? @
    ] ? @
  ] ? @
  .
  i 1 + :i
] ;
```

- `*` `+` `-` `/` `%` arithmetic (`+` also concatenates strings)
- `== != < > <= >= && || !` comparison & logic (push `1`/`0`)
- `=` dup · `,` drop · `\` swap · `&` over
- `cond [then] [else] ? @` is if/else; `[cond] [body] ;` is a while loop
- `.` prints; stdlib words like `tr` (trim), `ln` (length), `up` (upper) auto-run

Full definition: [`docs/SPEC.md`](docs/SPEC.md).

## Why it looks like this

The [design conversation](#design-notes) that started Zown is full of ambition
*and* contradictions (e.g. `*` used for both "multiply" and "loop"; `=` for both
"dup" and "equals"). v0.1's job was to turn that into a **coherent language that
actually runs**, then phase in the systems vision without breaking it. Where the
notes disagreed with themselves, [`docs/SPEC.md`](docs/SPEC.md) is the authority
and marks the resolution.

Three ideas make density safe rather than confusing:

1. **Stack-based** — most values flow on the operand stack, so they never need a
   name. Fewer names = fewer tokens.
2. **Shadow manifest** — a sidecar `<file>.zn.json` maps each tiny symbol to an
   `alias`, a description, and an `ai_hint`. The code stays microscopic; meaning
   lives in the manifest. See [`docs/MANIFEST.md`](docs/MANIFEST.md).
3. **Errors are instructions** — failures emit a structured `.zerr` packet
   (recovery code + stack snapshot + hint) an AI agent can act on directly,
   instead of prose it has to decode.

## Repo layout

```
zown/            reference implementation (the language itself)
  lexer.py         two-state lexer (code/literal modes)
  parser.py        tokens -> nested AST
  vm.py            stack virtual machine + core operators
  builtins.py      token-dense standard library (WORDS)
  errors.py        structured .zerr diagnostics
  manifest.py      shadow manifest generator
  cli.py           the `zown` command
bin/zown         dev launcher (no install needed)
examples/        hello / fib / fizzbuzz
tests/           run with: python3 tests/test_zown.py
docs/            SPEC.md · ROADMAP.md · MANIFEST.md
```

## Tests

```bash
python3 tests/test_zown.py
python3 tests/test_examples.py
```

## Status & roadmap

v0.1 runs two ways that are kept byte-for-byte identical via a conformance suite:

- the **Python reference interpreter** (`zown`, the behavioral oracle), and
- the **native Rust toolchain** (`zownc`, in `zownc/`) — its lexer, parser, and
  stack VM are ported and verified: `zownc run` matches the oracle on all
  conformance cases.

Still ahead: a shared IR, **WASM + native (Cranelift/LLVM) backends**, the
type/memory model, dynamic fast lanes, embedded graph DB, rolling hot-swap, the
self-healing loop, the communication mesh, and ultimately **self-hosting** (the
Zown compiler written in Zown). The detailed, milestone-by-milestone plan is in
[`docs/PLAN.md`](docs/PLAN.md); the high-level vision is in
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Design notes

The original brainstorm (the full Q&A that motivated every feature) is the seed
of `docs/ROADMAP.md`; the roadmap is its de-contradicted, buildable form.

## License

MIT.
