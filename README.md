# Zown

**An AI-native, token-dense, stack-based programming language.**

Zown is built for machines to read and write, not humans. Keywords are single
ASCII characters, the standard library uses words of one to three characters,
and the stack-based model means programs rarely need variable names at all. The
goal: fit *dramatically* more working code inside an LLM's context window, and
run that code as a Python oracle, a native binary, WebAssembly, or a 512-byte
boot sector.

> Milestones **M0–M30** are in this tree. The Python package is the behavioral
> oracle. `zownc` is the Rust toolchain: checker, virtual machine, Cranelift,
> LLVM, WebAssembly, and a real-mode boot sector. The milestone map is
> [`docs/PLAN.md`](docs/PLAN.md). The whole-machine architecture is
> [`docs/DESIGN.md`](docs/DESIGN.md); the phases are
> [`docs/ROADMAP.md`](docs/ROADMAP.md).

## Hello, World

```
[$Hello, World!$.]:h h@
```

That's the whole program. `[ ... ]` makes a block, `:h` binds it to `h`, `h`
pushes it, and `@` runs it.

## Quickstart

The interpreter needs only Python 3.10+. The `zownc` lines need a Rust
build (`cd zownc && cargo build`); `wasmtime run` needs wasmtime.

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
python3 bin/zown doc examples/fib.zn        # print those fields; a null sig stays null
python3 bin/zown pkg app.zpkg               # resolve zown: hashes in a directory

# interactive stack session
python3 bin/zown repl

# how many real-time lanes this machine will pin (optional N is the request)
python3 bin/zown tune

# canonical whitespace, and the Zown test runner (tests/zn by default)
python3 bin/zown fmt examples/hello.zn
python3 bin/zown lsp                        # check, definition, and format on stdin
python3 bin/zown test

# two local IPv6 peers exchange one BLAKE3-addressed bundle
python3 bin/zown mesh

# the layout tree, then the same scene as a PPM of glyphs
python3 bin/zown examples/ui.zn
python3 bin/zown draw examples/paint.zn > ui.ppm

# seal a program's graph, then refuse a bundle whose bytes changed
python3 bin/zown keep export examples/graph.zn -o graph.sync
python3 bin/zown keep verify graph.sync

# declared intent, then the log of a fault and its note
python3 bin/zown examples/intent.zn

# the Zown lexer tokenizes its own source; the dump must match the host
python3 bin/zown stage zown/std/lex.zn

# the Zown parser prints the same AST as the host, including on its own source
python3 bin/zown stage parse zown/std/parse.zn

# the Zown lowerer prints the same IR text as `zownc ir`
python3 bin/zown stage lower zown/std/lower.zn

# a 512-byte boot sector, written by the Zown emitter (print subset only)
python3 bin/zown stage emit examples/bare.zn > zown.bin

# native toolchain (after `cd zownc && cargo build`)
./zownc/target/debug/zownc run examples/fib.zn
./zownc/target/debug/zownc build examples/fib.zn -o fib.wasm
wasmtime run fib.wasm

# a boot sector whose VGA text is the string this program prints
./zownc/target/debug/zownc bare examples/bare.zn -o zown.bin

# the same print, from long mode: four timer ticks, then a reported page fault
./zownc/target/debug/zownc kern examples/bare.zn -o kern.bin

# an exclusive buffer, a stored byte, and a borrow's read
python3 bin/zown examples/buf.zn

# read a fixture under a path grant, write it back, read the copy
python3 bin/zown examples/io.zn

# RFC seal, signature, and X25519 shared secret
python3 bin/zown conformance/cases_v2/crypt.zn
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

And the v0.2 safety surface, live in the oracle and in `zownc`:

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
zown/            Python oracle: lexer, parser, checker, VM, stdlib
zown/std/        lexer and parser written in Zown
zownc/           Rust toolchain (`zownc`)
bin/zown         dev launcher (no install needed)
examples/        hello, fib, fizzbuzz, buf, io, ui, graph, intent, bare
conformance/     goldens and parity runners
tests/           python3 tests/test_zown.py
docs/            SPEC · DESIGN · PLAN · ROADMAP · WASM · IR · MANIFEST
```

## Tests

```bash
python3 tests/test_zown.py
python3 tests/test_examples.py
python3 conformance/run.py       # v0.1 goldens
python3 conformance/v2.py        # v0.2 goldens
python3 conformance/check.py     # static checker

# after: cd zownc && cargo build
python3 conformance/vm_parity.py
python3 conformance/native_parity.py
python3 conformance/wasm_parity.py   # needs wasmtime
python3 conformance/stage2_parity.py # IR fixed point and boot-image bytes
python3 conformance/stream_pair.py   # two peers on [::1]
python3 conformance/map_reopen.py    # snapshot file, new process
python3 conformance/fleet_pair.py   # trace id and drift across one socket
python3 conformance/bare_boot.py    # QEMU; skips only when qemu-system-i386 is absent
python3 conformance/kern_boot.py    # long mode; skips only when qemu-system-x86_64 is absent
```

The same oracle commands, `cargo test`, `cargo build`, `native_parity`, `stage_parity`,
`parse_parity`, `bare_boot`, and `kern_boot` run in [`.github/workflows/ci.yml`](.github/workflows/ci.yml).

## Status

**M0–M33 are done.** `op` decodes one narrowband packet and `oe` writes
it back (`cases_v2/opus`, `cases_v2/opus_enc`). `ve` writes one lossless
16×16 still and `av` reads those pixels back (`cases_v2/av1`). The same programs run three ways, held to one conformance
suite:

- **Python** (`zown`) is the oracle.
- **Native** (`zownc run`, `zownc native`, `zownc llvm`) matches it, including
  exclusive buffers, capability-scoped files, and the RFC crypto words
  (ChaCha20-Poly1305, Ed25519, X25519).
- **WebAssembly** (`zownc build`, then `wasmtime`) matches it for the language
  core: checked 64-bit integers, big integers, IEEE floats, fixed widths,
  vectors, capabilities, results, moves, `??`, and `ty`. Checked overflow is a
  `.zerr`. A program that needs files, buffers, crypto, or lane operators is
  refused at compile time; `conformance/wasm_parity.py` records that skip.

Also in the tree: `zown fmt` and `zown test`, a lexer, parser, and lowering
written in Zown (`zown stage`), a layout tree and a PPM of 5×7 glyphs
(`zown draw`), a button that runs when a pointer or a key hits it, one
animation tick on the node's width, and a software pipeline (`gp`) whose
readback is a `buf`. One BLAKE3 bundle is exchanged over IPv6
(`zown mesh`). A sealed graph (`zown keep`) carries an Ed25519 trailer and
an inclusion proof when the program asks. A logical control-plane log
(`zown watch`) and a 512-byte boot sector are there too. Stage-0 writes
that sector with `zownc bare`. `zown stage emit` writes the same bytes
from Zown, and the executable `zownc native -o` links does too. A list,
map, set, queue, ring, B-tree, trie, bloom filter, priority queue, and
bit vector are fat pointers. `iz` and `uz` are pointer widths. `dc` is
a checked decimal and `cx` is a Gaussian integer.

`sw` replaces a word registered with `wd` only after its Ed25519
signature checks and its test passes. A failure leaves the old word.
Two peers on `[::1]` share a trace id and print the same drift.

`zown lsp` reports the same diagnostic text as `zown check`, jumps to a
user binding, and formats with `zown fmt`. `zown doc` prints the manifest
fields a file has, and leaves `sig` null when nothing signed it. `zown pkg`
resolves a dependency named by its BLAKE3 in a local directory.

`zownc kern` boots that print in long mode. Four timer ticks mark row 24.
A load of an unmapped page stores `0xF1` at `0x7E00` and halts. The
real-mode sector is still `zownc bare`.

**Next is M34:** ring 3 on the long-mode kernel. The map is
[`docs/PLAN.md`](docs/PLAN.md); agents start at [`AGENTS.md`](AGENTS.md). The
whole vision is [`docs/DESIGN.md`](docs/DESIGN.md), and the phases are
[`docs/ROADMAP.md`](docs/ROADMAP.md).

## Design notes

The original brainstorm (the full Q&A that motivated every feature) is the seed
of `docs/ROADMAP.md`; the roadmap is its de-contradicted, buildable form.

## License

MIT.
