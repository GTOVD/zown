# Zown

**An AI-native, token-dense, stack-based programming language.**

Zown is built for machines to read and write, not humans. Keywords are single
ASCII characters, the standard library uses 1–2 character words, and the
stack-based model means programs rarely need variable names at all. The goal: fit
*dramatically* more working code inside an LLM's context window, while compiling
(eventually) straight to bare-metal and WebAssembly speed.

> This repo is the **v0.1 reference implementation** — a complete, runnable
> language (lexer → parser → stack VM → stdlib → CLI) plus a Rust toolchain and a
> full WASM backend. The bigger goal is a **self-contained, decentralized
> substrate** where networking, graphics/UI, database, security, and distribution
> are all native — no external libraries, no servers. The full architecture is in
> [`docs/DESIGN.md`](docs/DESIGN.md); it's phased in [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Hello, World

```
[$Hello, World!$.]:h h@
```

That's the whole program. `[ ... ]` makes a block, `:h` binds it to `h`, `h`
pushes it, and `@` runs it.

## Quickstart

No dependencies — just Python 3.10+.

```bash
# run a program (the static checker runs first; a program it faults is
# rejected with exit status 2 before anything executes)
python3 bin/zown examples/hello.zn
python3 bin/zown examples/fizzbuzz.zn

# static checker only (no execution): unresolved names, malformed `??` arms,
# capability requirements that can never be satisfied
python3 bin/zown check examples/fib.zn

# get a machine-readable error packet
python3 bin/zown --zerr examples/hello.zn

# generate the shadow manifest (meaning for the dense symbols)
python3 bin/zown manifest examples/fib.zn   # writes examples/fib.zn.json

# interactive stack session
python3 bin/zown repl

# how many real-time lanes this machine will pin (optional N is the request)
python3 bin/zown tune

# canonical whitespace, and the Zown test runner (tests/zn by default)
python3 bin/zown fmt examples/hello.zn
python3 bin/zown test

# two local IPv6 peers exchange one BLAKE3-addressed bundle
python3 bin/zown mesh

# the layout tree, then the same scene as a PPM picture
python3 bin/zown examples/ui.zn
python3 bin/zown draw examples/paint.zn > ui.ppm

# seal a program's graph, then refuse a bundle whose bytes changed
python3 bin/zown keep export examples/graph.zn -o graph.sync
python3 bin/zown keep verify graph.sync

# declared intent, then the log of a fault and its note
python3 bin/zown examples/intent.zn

# the Zown lexer tokenizes its own source; the dump must match the host
python3 bin/zown stage zown/std/lex.zn

# a boot sector whose VGA text is the string this program prints
# (after `cd zownc && cargo build`)
./zownc/target/debug/zownc bare examples/bare.zn -o zown.bin
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

- `*` `+` `-` `/` `%` arithmetic (`ct` joins strings, `rp` repeats one)
- `== != < > <= >= && || !` comparison & logic (push `1`/`0`)
- `=` dup · `,` drop · `\` swap · `&` over
- `cond [then] [else] ? @` is if/else; `[cond] [body] ;` is a while loop
- `.` prints; stdlib words like `tr` (trim), `ln` (length), `up` (upper) auto-run

And the v0.2 safety surface, all live in the oracle:

```
`t [ `t rq $sending$ . ] gr          # capabilities: zero authority unless granted
[ 1 0 / ] ty [ [ok]  [ . ]           # errors are values: ok(v) / err(code) ...
              [err] [ $failed: $ \ ct . ] ] ??  # ...and only ?? can open them
2 b 100 pw .                         # int is a checked 64-bit; `b` is exact big
5 :x 'x .                            # 'x moves a binding out; a bare x borrows
```

- integer overflow, nan→int, negative sqrt, unbounded recursion — every one is a
  structured `.zerr` (or a static rejection), never undefined behavior (SPEC §19)
- `zown run` refuses a program the static checker faults (exit status 2)

Full definition: [`docs/SPEC.md`](docs/SPEC.md).

## Why it looks like this

Each token has one meaning, and every prebuilt word is one character, then two,
then at most three. `*` is multiply (`rp` repeats a string, `;` loops). `=` is
dup (`==` tests equality). `+` adds (`ct` joins strings). `_` negates (`any` is
the default pattern). `b` builds a big integer. `at` indexes a vector lane, and
`ty` turns a block into a result. The long name of a short word lives in the
shadow manifest. [`docs/SPEC.md`](docs/SPEC.md) is the authority for each of these.

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
docs/            DESIGN.md · SPEC.md · ROADMAP.md · PLAN.md · MANIFEST.md · WASM.md · IR.md
```

## Tests

```bash
python3 tests/test_zown.py
python3 tests/test_examples.py
```

## Status & roadmap

v0.1 runs three ways that are kept byte-for-byte identical via a conformance suite:

- the **Python reference interpreter** (`zown`, the behavioral oracle),
- the **native Rust toolchain** (`zownc`, in `zownc/`) — lexer, parser, IR, and
  stack VM, verified so `zownc run` matches the oracle on all conformance cases, and
- the **WASM backend** (`zownc build`) — the full v0.1 language compiles to
  WebAssembly text (`.wat`) **and** binary (`.wasm`); all 13 conformance cases run
  under `wasmtime` and match the goldens (`conformance/wasm_parity.py`).

The v0.2 design freeze (M7) is done: capabilities, fixed-width + SIMD numerics,
and pattern matching run in the oracle (`conformance/v2.py`), and a static
checker (`zown check`) already rejects unresolved names, malformed `??` arms, and
capability requirements it can prove will fail (`conformance/check.py`).

Still ahead, in dependency order (the milestone-by-milestone plan is in
[`docs/PLAN.md`](docs/PLAN.md); agents start at [`AGENTS.md`](AGENTS.md)):

1. **Design freeze (M7) ✅** — the type, capability/security, network, and graphics
   SPEC is frozen and validated in the oracle *before* the native compiler,
   because those decisions shape the ABI ([`docs/SPEC.md`](docs/SPEC.md) Part II).
2. **Safety core (M8) 🔄** — the static checker (names, match shape, capability
   flow, result exhaustiveness, use-after-move ✅), check-before-run ✅, the
   checked-int/`big` numeric model ✅, result values ✅, the no-UB catalogue ✅,
   linear bindings ✅; **next: the Rust toolchain catching up to v0.2**
   (`zownc check`, M8d). Fat pointers are specified here and enforced with the
   native memory model (M9).
3. **Native backend + perf (M9–M10)** — Cranelift→LLVM, SIMD, zero-copy I/O,
   deterministic builds, dynamic fast lanes.
4. **Batteries (M11)** — the zero-install stdlib: crypto, collections, full
   numerics, testing/fmt/LSP/doc tools.
5. **ZownNet (M12)** — IPv6 crypto-identity, a QUIC-equivalent transport, a P2P
   DHT+gossip mesh, content addressing, offline mesh — every device a peer.
6. **Native UI + GPU (M13)** — a typed declarative layout tree (no HTML/CSS),
   ZownGPU, audio, input, accessibility, i18n.
7. **Store + distribution (M14)** — the running slice is an in-memory graph
   (`gv`, `ge`, `»`, `→`, `«`) and `zown keep` (an fsynced BLAKE3 bundle).
   mmap, CRDTs, Ed25519, and hot-swap are not implemented.
8. **AI control plane (M15)** — the running slice is a logical log (spans,
   declared intent, and faults) and `zown watch`. A fleet, hot-swap, and
   on-device ML are not implemented.
9. **Self-hosting (M16)** — the running slice is a lexer written in Zown
   (`zown stage`). A parser and a stage-2 compiler binary are not implemented.
10. **Bare metal (M17)** — `zownc bare` boots a print program in QEMU and
    writes the VGA text buffer. A microkernel is not implemented.

The whole vision — the "sovereign computing substrate" — is in
[`docs/DESIGN.md`](docs/DESIGN.md); the high-level phases in
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Design notes

The original brainstorm (the full Q&A that motivated every feature) is the seed
of `docs/ROADMAP.md`; the roadmap is its de-contradicted, buildable form.

## License

MIT.
