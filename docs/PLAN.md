# Zown Development Plan (Working Doc)

This is the **living engineering plan** for Zown. `ROADMAP.md` is the high-level
"what and why"; `DESIGN.md` is the whole-machine vision; this doc is the "how, in
what order, and what done looks like". It is meant to be picked up cold in any
future session — by a human or an AI agent: read this top to bottom and you know
exactly where we are and what the next concrete task is. (`AGENTS.md` at the repo
root is the short operational version: environment, commands, and the rules.)

**How to use this doc**
- The **Status Board** is the single source of truth for progress. Update it in
  the same commit that finishes a slice.
- Each milestone has: *Goal → Slices/Tasks → Acceptance criteria → Risks*. A
  milestone is built in **slices** small enough to finish in one focused session;
  every slice follows the checklist in "How a slice is done".
- The **reference interpreter (Python) is the oracle**: every native feature must
  produce identical observable behavior, verified by the conformance suite.
- **Open decisions** and **Known debts** are tracked in their own sections below so
  nothing is lost between sessions. When you resolve one, move it into the SPEC or
  the milestone it belongs to.

Last updated: 2026-10-24 (M13: typed layout tree and a PPM picture).

---

## North Star

A hyper-dense, AI-native language **and** the self-contained substrate built on
it (full architecture: [`DESIGN.md`](./DESIGN.md)):
1. Minimizes tokens per feature (1–2 char everything) so huge programs fit in a
   context window; meaning lives in the shadow manifest, not in identifiers.
2. Compiles to native speed (LLVM) and the web/sandbox (WASM ✅), and down to bare
   metal (kernels).
3. **Safe by construction**: zero authority by default (capabilities), no
   undefined behavior; failures are actionable, structured `.zerr` packets.
4. **Self-contained / zero-install**: networking, graphics, UI, database,
   security, and distribution are native — an app needs nothing but the runtime.
5. **Decentralized**: every device is a peer in an identity/content-addressed
   mesh; apps are signed, content-addressed, and verified before they run.
6. **Eventually self-hosts**: the Zown compiler is written in Zown — the ultimate
   proof the language can "build anything without external languages."

**Sequencing rule:** the security/type/network/graphics SPEC is frozen and
validated in the Python oracle **before** the native compiler (M7 ✅, M8 ✅),
because those decisions shape the ABI and codegen. See `DESIGN.md` §1.

---

## Status Board

Legend: ✅ done · 🔄 in progress · ⏳ next up · ⬜ not started

| # | Milestone | State | Notes |
|---|-----------|-------|-------|
| M0 | Reference interpreter (Python) | ✅ | lexer/parser/VM/stdlib/CLI/manifest/.zerr/tests |
| M1 | Spec hardening + conformance suite | ✅ | 13 cases + 7 error cases, golden runner (`conformance/run.py`) |
| M2 | Rust toolchain + `zownc` skeleton | ✅ | workspace + crates; `zownc lex` (no CI yet — see Known debts) |
| M3 | Rust frontend (lexer+parser+AST) parity | ✅ | `zownc ast` == Python `zown ast` on all 16 programs |
| M4 | Tree-walking Rust VM parity | ✅ | `zownc run` == oracle on all 20 v0.1 conformance cases |
| M5 | IR + lowering | ✅ | `zown-ir`; lossless round-trip on all 16 programs |
| M6 | WASM backend (`-o .wasm`) | ✅ | Full v0.1 -> `.wat` + binary `.wasm`; all 13 cases run in wasmtime |
| M7 | **Design freeze: SPEC v0.2 + oracle** | ✅ | M7a capabilities, M7b manifest v2, M7c numerics+SIMD, M7d pattern matching — all in the oracle; net/UI/crypto types frozen as design-only |
| M8 | Safety core (type & memory model) | ✅ | M8a–M8d ✅. `zownc check` gates `zownc run` (exit 2). WASM runs v0.1 with checked `int`. v0.2 tag numbers are D8; executing them in WASM is still `UNSUPPORTED` |
| M9 | Native backend + perf (Cranelift→LLVM) | ✅ | M9a–M9f ✅. Int ops inlined, SIMD `va`/`vs`/`vm` (except `vu` multiply), deterministic exes, `zownc native -O` and `zownc llvm` (`clang -O2`). `buf` is design-only (§21). `native_parity` 59/59 |
| M10 | Concurrency: dynamic fast lanes (`~ ^ \|`) | ✅ | cooperative ticks; `1 lc` trace is stable; `native_parity` 67/67. OS `SCHED_FIFO` is not observable behavior (§22) |
| M11 | Batteries stdlib + `std` in Zown | ✅ | `mx` `mn` `cl` are Zown; `zown test` and `zown fmt` ship. Crypto, files, LSP, and collections are SPEC §23 and are not stubbed |
| M12 | ZownNet — decentralized mesh | ✅ | BLAKE3 `h3`, `~p`/`~f`, `zown mesh` on `[::1]`. QUIC, DHT, Ed25519, and ChaCha are not implemented |
| M13 | Native UI + ZownGPU + audio + input | ✅ | layout tree `co tx bt rw cm ly` and `zown draw` PPM. GPU, audio, and input delivery are not implemented |
| M14 | Embedded store + distribution | ⬜ | mmap graph+columnar, CRDT, version chain, transparency log, semantic patch protocol, signed sync |
| M15 | AI control plane + rolling execution | ⬜ | telemetry/tracing, declarative intent, hot-swap, self-healing loop, on-device ML |
| M16 | **Self-hosting**: compiler rewritten in Zown | ⬜ | the endgame |
| M17 | Zown OS / bare metal | ⬜ | `--target=bare`, microkernel demo |

> M7–M17 were expanded from the original M7–M15 to fold in the full
> sovereign-substrate vision (`DESIGN.md`). The key change: a **design-freeze
> milestone (M7) now precedes the native compiler (M9)**, and the safety/security
> model (M8) lands before codegen so capabilities and no-UB shape the ABI.
> Older docs/commits that say "M7 native backend" or "M14 self-hosting" use the
> pre-expansion numbering; this table is authoritative.

---

## Development environment

What you need to run everything in this repo, and what is known about the
machines it has been developed on.

- **Python ≥ 3.9 works in practice** (`pyproject.toml` declares `>=3.10`; every
  module uses `from __future__ import annotations`, and the full suite passes on
  3.9.13). No third-party packages — the oracle is dependency-free by design.
  - macOS/Linux: `python3 …`. Windows: `C:\Python39\python.exe` was the working
    interpreter on the 2026-09 dev machine (the `python` on PATH there is the
    Microsoft Store stub). Prefer `python -X utf8` on Windows so `—` in
    diagnostics renders instead of `?`.
  - `bin/zown` is a `#!/usr/bin/env python3` launcher; on Windows use
    `python -m zown.cli …` from the repo root instead.
- **Rust (stable ≥ 1.96) + cargo** are required only for `zownc` and the four
  parity runners. Install with rustup; then `cd zownc && cargo build && cargo test`.
  The 2026-09 Windows machine has rustup + `stable-x86_64-pc-windows-gnu`
  (no Visual C++ `link.exe`, so the default `windows-msvc` host cannot link).
  Build with `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu`; do not change
  `zownc/rust-toolchain.toml` (it stays host-default for macOS/Linux). The
  parity runners locate the binary at `zownc/target/debug/zownc` (`.exe` on
  Windows).
- **wasmtime** is required only for `conformance/wasm_parity.py`.
- **git** on the Windows machine lives at `C:\Program Files\Git\cmd` (not on PATH
  in every shell). Commits carry `Co-authored-by: Cursor <cursoragent@cursor.com>`
  when an agent wrote them.

Full verification (what "green" means for the oracle side, ~2 s):

```bash
python3 tests/test_zown.py          # unit tests (101)
python3 tests/test_examples.py      # the shipped examples
python3 conformance/run.py          # v0.1 goldens (20)
python3 conformance/v2.py           # v0.2 goldens (39)
python3 conformance/check.py        # static-checker goldens (14)
```

---

## How a slice is done

Every milestone is delivered as slices (`M8b`, `M7c-ii`, …), one commit each,
following the same shape the history already shows. Do them in this order:

1. **Read**: this doc's Status Board + the milestone section + the relevant SPEC
   part. Confirm the slice is still the "⏳ next up" item.
2. **Design in the SPEC first** if the slice adds observable semantics: write the
   rule, the tokens (marked *provisional* until the density audit), the error
   codes. Anything the oracle cannot observe honestly is documented as
   *design-only (M…)*, not coded.
3. **Implement in the oracle** (`zown/`). Keep v0.1 behavior byte-identical unless
   the SPEC says otherwise; the Rust runners must stay green.
4. **Unit tests** in `tests/test_zown.py` (plain asserts, no framework) — cover
   the positive path, each new error code, and any soundness/no-false-positive
   claim.
5. **Conformance goldens**: add `.zn` cases to the right corpus and `--bless` them
   (`run.py` / `v2.py` / `check.py`). Commit the `.zn` with its golden. A new
   static check must leave the entire runnable corpus clean.
6. **Docs**: SPEC section marked *implemented (M…, oracle/checker)*; `MANIFEST.md`
   / `WASM.md` / `IR.md` if touched; CLI docstrings/help.
7. **This doc**: tick the slice, update the Status Board row, refresh
   "Immediate next actions", add anything learned to Open decisions / Known
   debts. Bump "Last updated".
8. **Commit** with an imperative, scoped subject (`M8b: static capability flow in
   the checker`, `docs: …`, `vm: …`) and a body that explains the semantics, lists
   the files, and states what was *not* touched.

---

## Architecture overview (target state)

```
            .zn source
                │
        ┌───────▼────────┐     (identical semantics, differential-tested)
        │   Frontend     │ ──────────────────────────────┐
        │ lexer → parser │                                │
        └───────┬────────┘                                │
                │ AST                                      │
        ┌───────▼────────┐                        ┌────────▼────────┐
        │ Static checker │  (M8: names, match,    │  Python ref VM  │
        │  (zown check)  │   capability flow, …)  │   (the oracle)  │
        └───────┬────────┘                        └─────────────────┘
                │ checked AST
        ┌───────▼────────┐
        │  Lowering →    │
        │   Zown IR      │
        └───┬───────┬────┘
            │       │
   ┌────────▼─┐  ┌──▼─────────┐
   │ WASM emit│  │ LLVM/Cran. │   → .wasm / .exe / .bin(bare)
   └──────────┘  └────────────┘
```

- **One frontend, multiple backends.** Lower the AST to a small, explicit **Zown
  IR**; every backend consumes IR, never the AST directly.
- **The checker sits between parse and lowering.** It is built in the oracle
  first (`zown/checker.py`) with its own golden corpus, then ported to Rust (M8d)
  exactly like the lexer/parser/VM were.
- **The Python VM is retired only after M14**, and even then kept as a spec
  oracle for regression testing.

---

## Repo layout (target)

```
zown/                 # Python reference implementation (the oracle) — keep
  checker.py          #   static checker (M8) — the oracle for `zownc check`
zownc/                # Rust workspace (the real toolchain)
  crates/
    zown-lexer/       # lexer (zero-copy spans, later SIMD)
    zown-parser/      # parser -> AST
    zown-ast/         # AST + node defs shared across crates
    zown-check/       # static checker port: names, match shape, capability flow (M8d)
    zown-ir/          # Zown IR + lowering from AST
    zown-vm/          # tree-walking interpreter (parity bring-up)
    zown-wasm/        # WASM backend
    zown-codegen/     # LLVM/Cranelift backend (M9)
    zown-types/       # result/ownership/memory model (M8c rules, enforced from M9)
    zown-net/         # ZownNet: transport, DHT, gossip, content addressing (M12)
    zown-ui/          # layout engine + ZownGPU bindings (M13)
    zown-store/       # embedded graph+columnar store, CRDTs (M14)
    zown-mesh/        # version chain, transparency log, patch protocol, sync (M14)
    zown-ctl/         # AI control plane: telemetry, intent, hot-swap (M15)
    zown-cli/         # the `zownc` binary (driver)
  Cargo.toml          # workspace manifest
conformance/          # language-agnostic golden tests (.zn + .out/.code/.diag) + runners
std/                  # standard library written in Zown (self-host prep, M11+)
examples/
docs/                 # DESIGN / SPEC / ROADMAP / PLAN / MANIFEST / WASM / IR
tests/                # Python-side tests
AGENTS.md             # cold-start guide for AI agents (env, commands, rules)
```

---

## Milestones in detail

### M1 — Spec hardening + conformance suite  ✅
**Goal:** freeze v0.1 semantics as language-agnostic golden tests so the Rust
port has an exact target.

**Tasks**
- [x] Create `conformance/` with cases as pairs: `name.zn` + `name.out`
      (expected stdout), and `errors/name.zn` + `name.code` (expected error code
      and offending op — compared on code/op, never on message text).
- [x] Port every Python unit-test scenario into conformance cases.
- [x] Runner `conformance/run.py` executes each `.zn` through the Python VM and
      diffs output; the same cases later run through `zownc`.
- [x] Semantic edge cases documented in `SPEC.md` (number/print `.` ambiguity,
      string `+`/`*` overloads, truthiness, select-then-invoke arity).

**Acceptance (met):** `python3 conformance/run.py` is green; cases cover every
operator, every stdlib word, and at least one error per recovery code.

---

### M2 — Rust toolchain + `zownc` skeleton  ✅
**Goal:** a compiling Rust workspace with a CLI that can at least lex+print.

**Tasks**
- [x] Install Rust (stable 1.96) via rustup.
- [x] Scaffold `zownc/` workspace + crates (`zown-lexer`, `zown-cli`).
- [x] `zownc lex file.zn` prints the token stream (first real parity target).
- [x] Rust lexer unit tests (incl. hello-world token shape == Python).
- [x] `zown-parser`, `zown-ast`, `zown-ir`, `zown-vm`, `zown-wasm` crates added
      as M3–M6 landed.
- [ ] CI: `cargo test` + conformance runners on push — **not done**; tracked in
      Known debts.

**Acceptance (met):** `cargo build` succeeds; `zownc --help` and `zownc lex` work.

---

### M3 — Rust frontend parity  ✅
**Goal:** Rust lexer+parser produce an AST equivalent to Python's.

**Tasks**
- [x] Port the two-state lexer (code/literal), exact token kinds + spans.
- [x] Port number/`.`-print disambiguation and the reserved-`|` rule.
- [x] Port the parser + nested blocks + the same parse errors (codes/positions).
- [x] `zownc ast` emits JSON shape-compatible with Python `zown ast`.
- [x] `conformance/ast_parity.py` diffs both frontends on every program.

**Acceptance:** for all conformance `.zn`, Rust AST == Python AST. ✅ (16/16)

---

### M4 — Tree-walking Rust VM parity  ✅
**Goal:** `zownc run x.zn` is byte-for-byte identical to `zown x.zn`.

**Tasks**
- [x] Port the operand stack, env, `Block` value (Rc-shared), truthiness.
- [x] Port all operators and the full stdlib `WORDS` (incl. Python-style modulo,
      whole-float collapse, banker's rounding).
- [x] Port `.zerr` packet emission (`zownc run --zerr`), same codes + offending op.
- [x] `conformance/vm_parity.py`: run all cases through `zownc`, diff stdout +
      error code against the goldens.

**Acceptance:** 20/20 conformance parity vs the oracle, zero diffs. ✅

**This milestone completes the rewrite of the v0.1 language in Rust.** Known gaps
carried forward (see Known debts): Rust ints are `i64` (oracle is bigint) and
Rust runtime errors omit `pos` (its AST has no spans yet); neither affects the
current corpus.

---

### M5 — Zown IR + lowering  ✅
**Goal:** a small, explicit IR the backends share.

**Tasks**
- [x] Define IR (`Instr`, `IrBlock`, `IrProgram`) — const/load/bind/op + the three
      control instrs (invoke/select/while); quotations become addressable blocks.
- [x] Lower AST → IR; `[ ... ]` blocks split into the block table.
- [x] `unlower` rebuilds the exact AST (lossless verification).
- [x] IR pretty-printer + `zownc ir` / `zownc irast`; documented in `docs/IR.md`.
- [x] `conformance/ir_roundtrip.py` proves `irast == ast` across the corpus.

**Acceptance:** lossless round-trip on all 16 programs. ✅

> Decision: instead of a second full interpreter, M5 validates the IR by a
> lossless round-trip against the oracle AST (a stronger structural guarantee for
> less duplicated code). The real second execution path is the M6 WASM backend,
> validated against the same goldens.

---

### M6 — WASM backend  ✅
**Goal:** `zownc build x.zn -o x.wasm` runs in wasmtime and the browser.

Built in slices (see `docs/WASM.md`); coverage tracked by `conformance/wasm_parity.py`.

- [x] **M6a — integer core.** `zown-wasm` emits `.wat`; integers, `+ - * % _`,
      comparisons, `&& || !`, and `.` (itoa + WASI `fd_write`). `compare` and
      `logic` run in wasmtime and match the goldens; unsupported constructs error
      with the slice that will add them. `zownc build`/`wat` commands.
- [x] **M6b — tagged values + strings.** Every value is a `(tag, payload)` pair on
      the WASM stack; strings are `[len][bytes]` literals + a bump heap. `$...$`,
      string `+`/`*`, words `tr/up/lo/rv/ln`, and stack ops `= , \ & rt`.
- [x] **M6c — blocks + control.** Operand stack moved to linear memory (`$sp`) so
      blocks (compiled to `() -> ()` functions in a `funcref` table) share it.
      `[ … ]`, `@`→`call_indirect`, `?`→`select`, `;`→`block`/`loop`, `:bind`/name
      load via per-name globals. **10/13** cases green.
- [x] **M6d — floats + binary.** Float values (tag `1`, `f64` bit pattern) with
      int→float promotion; `/ % _`, comparisons, and the math words `sq pw ab mx
      mn fl ce rd`, plus `n s dp clr pr`. `$fmt_f64` renders floats per the
      oracle's display rule. Binary `.wasm` via the `wat` crate (`build -o x.wasm`).

**Acceptance (M6 overall): MET.** All 13 conformance cases compile and match the
goldens under wasmtime, in **both** `.wat` and binary `.wasm`.

**Known limitation:** float-to-decimal is exact for dyadic values but not yet a
shortest round-tripping formatter (Ryu/Grisu) — see Known debts.

---

### M7 — Design freeze: SPEC v0.2 + oracle  ✅
**Goal:** freeze the ABI-shaping language decisions and prove them in the Python
oracle **before** any native codegen. Nothing here needs a new backend.

v0.2 semantics are validated against the oracle only; the corpus lives in
`conformance/cases_v2/` + `errors_v2/` (run `python3 conformance/v2.py`), kept
separate so the Rust `zownc` parity runners stay green until M8d implements v0.2.

- [x] **M7a — capabilities (the security core).** `` ` `` capability sigil lexes
      to a `Cap` token; zero authority by default; `gr` (grant-scope), `rq`
      (require → `CAP_DENIED`, kind `sec`, with a `cap` field), `hv` (non-fatal
      probe). Security `.zerr` codes added (`CAP_DENIED`, `AUTH_FAIL`,
      `INTEGRITY_FAIL`, `SIG_INVALID`, `RATE_LIMITED`, `UB_TRAP`).
- [x] **M7b — manifest v2.** `zown/manifest.py` emits the v2 shape: per user
      symbol `caps` (discovered from the bound block's body + merged with authored
      ones), `sec`/`tele`/`i18n` (scaffolded), and a module provenance block
      (crypto fields null until M14). Builtins keep the lean v1 shape.
- [x] **M7c — numeric model.** _M7c-i:_ fixed-width integer tags (`i8…i128
      u8…u128`, each a `WidthTag`) + policy words `wr`/`st`/`ck` make overflow
      explicit (new `OVERFLOW` code). _M7c-ii:_ fixed-lane SIMD vectors (`f4 d2
      i4 b16`) with `vadd`/`vsub`/`vmul`/`vsum`/`vat`; integer lanes wrap.
      `dec`/`cx` and tensors stay design-only (revisit with M8 types).
- [x] **M7d — pattern matching.** `??` dispatches `subject [arms] ??` over
      `[pattern][body]` pairs; patterns are a type name, a literal, or `_`; the
      body runs with the subject on the stack. New codes `NO_MATCH`/`BAD_PATTERN`.
      Crypto `Key/Sig/Hash/NodeID` (§13), network `~`-family (§14), UI/GPU (§15)
      are **design-only** (they need real crypto/I/O/rendering); tuple
      destructuring waits on compound value types.

**Acceptance (met):** `SPEC.md` Part II is complete and internally consistent —
every section is marked *implemented (oracle)* or *design-only (M…)*; the oracle
implements every semantically-observable addition; `conformance/v2.py` is green.

---

### M8 — Safety core (type & memory model)  🔄
**Goal:** make the "unhackable" guarantees real in the checker + runtime. A
`zown check` pass runs *before* execution and shapes the ABI before codegen.

**The checker's contract (binding for every slice):** a diagnostic is either *a
definite unhandled fault* ("if this node executes, it raises a `.zerr` nothing
catches" — inside `at` a fault is the handled path and is not reported) or *a
violation of a mandatory static rule the SPEC declares* (a name bound nowhere,
malformed `??` arms, non-exhaustive result handling). The checker never reasons
about whether a node is reached (dead branches are checked too) and never reports
something that could succeed. **Zero false positives** is the invariant; the whole
runnable corpus must check clean (`test_check_corpus_has_no_false_positives`), and
since M8c-0 a finding *rejects the program* (`zown run` exits 2), so a false
positive is a false rejection.

#### Slices

- [x] **M8a — checker skeleton + name resolution.** `zown/checker.py` walks the
      AST and emits structured `.zerr` diagnostics (kind `check`) without running
      the program; `zown check <file>` reports all findings (a JSON array under
      `--zerr`). Two sound checks: a `name` never bound anywhere and not a builtin
      is a *static* `NAME_UNRESOLVED`; an arms block literally before `??` is
      validated for shape — patterns are introspected, never resolved as names.
- [x] **M8b — static capability flow.** `` `c rq `` is a compile-time `CAP_DENIED`
      (kind `check`, with the `cap` field) whenever the checker can *prove* `c` is
      not granted. Grants have dynamic extent, so this is flow, not lexical scope:
      (1) top level holds exactly zero authority and block literals consumed
      inline (`@ ; ?@ ?? gr`, SIMD ctors) inherit an *exact* granted set; (2) a
      capability with no free `` `c `` literal anywhere can never reach `gr`, so
      its `rq` is denied even in escaped blocks. Everything else is "unknown" and
      stays silent — `[ `s rq ] :f  `s [ f @ ] gr` is clean. Shadowed `rq`/`gr`/`hv`
      are honored. 10 unit tests + a corpus zero-false-positive guard; new golden
      runner `conformance/check.py` (10 cases incl. the sound-silence ones) that the
      Rust `zownc check` port will target. SPEC §12 documents the rules.

- [x] **M8c — the language-level safety story (oracle + checker).** Five slices,
      each its own commit, driven by the maintainer's decisions of 2026-09-07
      (D1 checked-int default, D2 check gates run, D3 results via `??`, D4 keep
      `!`/`&` and make ownership one concise sigil, D5 `.zerr` 0.2):
      - [x] **M8c-0 — check-before-run (D2).** `zown run` (and the shorthand) runs
        `check` first; any finding is reported as `check` packets and the process
        exits **2** ("rejected — rework the code") vs 1 (lex/parse/runtime error)
        and 0. `--unchecked` is a debugging bypass. `VM.run_src` stays ungated (it
        is the runtime oracle). `.zerr` version 0.2. SPEC §8.
      - [x] **M8c-i — numeric model (D1).** `int` is a **checked 64-bit** integer:
        results of `+ - * _ pw ab fl ce rd n` outside the range are `OVERFLOW`,
        literals beyond it are a lex-time `OVERFLOW`. `b` is the one-char word for
        the exact, unbounded `big` (`5 b`, `$digits$ b`); any big operand keeps
        the result big; `[big]` pattern; literal patterns compare by value. Floats
        are IEEE-754: `±inf`/`nan` are values (printed as such), non-finite →
        integer traps (`nan` `UB_TRAP`, `inf` `OVERFLOW`), negative `sq` and a
        negative base with a fractional `pw` exponent are `UB_TRAP`. Width-policy
        results are `int` for widths inside `int`, `big` for `u64 i128 u128`
        (static rule). SPEC §1/§2/§11.
      - [x] **M8c-ii — result values (D3).** `[ body ] at` runs the body on an
        isolated stack → `ok(value)` or `err(code)`; fixed stack effect `( blk --
        res )`. The payload is reachable **only** via `??` `[ok]`/`[err]` arms
        (unwrapped); `[res]` is the type pattern; opaque to other operators;
        truthy iff ok. **Exhaustiveness is a static rule**: `[…] at [arms] ??`
        must cover ok and err (or `_`/`res`) else static `NO_MATCH`. Checker
        contract refined: definite *unhandled* fault, or a mandatory static rule;
        capability-flow denials inside `at` are the handled path (suppressed).
        SPEC §16.1; DESIGN §2.2 wording.
      - [x] **M8c-iii — the no-UB catalogue.** SPEC §19: a table of every
        undefined/implementation-defined situation → one Zown outcome (value,
        structured trap, or static rejection), each with a golden. Closed the two
        raw-crash holes: unbounded recursion → `STACK_OVERFLOW` (depth limit 128,
        op/pos at the call site, loops never consume depth) and pathological
        nesting → parse `REPAIR_SYNTAX` (limit 256).
      - [x] **M8c-iv — linear bindings (D4).** `'x` **moves** a binding out (push +
        unbind); a bare `x` is the borrow. One char for move, zero for borrow;
        `!`/`&` untouched; `"` reserved for a future mutable-reference sigil.
        Static **use-after-move** on straight-line paths (with sound inherit/clear
        rules for inline blocks, `;` cond vs body, invokers, `at`). SPEC §3/§20.
      The fat-pointer/`buf` memory model is *specified* (§19 last rows, §20 "why
      it matters later") and **enforced with M9**, where memory first exists —
      the re-sequencing the previous plan proposed, now adopted.

- [x] **M8d — Rust toolchain reaches v0.2 + `zownc check`.**
      The frontend, the VM, and the checker speak v0.2. WASM runs the v0.1
      language with checked `int` and refuses v0.2 constructs with
      `UNSUPPORTED` (M9) rather than a silent wrong answer. Slices:
      - [x] **M8d-i frontend.** Lexer: `Cap` + `Move` tokens, `??`, the int-literal
        range check (`OVERFLOW` past `i64::MAX`); AST: `cap`/`move` nodes plus a
        source `Pos` on every node (JSON still omits it, matching `zown ast`);
        `zownc ast` parity on v0.1 + v0.2 + check + examples (45/45). IR gained
        `ConstCap`/`Move` so unlower stays lossless (45/45). Runtime `.zerr`s
        now carry `pos` (zerr 0.2). WASM refuses those two instrs with a
        structured `UNSUPPORTED` naming M8d-iv. `ast_parity.py` /
        `ir_roundtrip.py` scan `cases_v2` and `check/`.
      - [x] **M8d-ii/iii VM values, numerics, and words.** `WidthTag`/`Vec`/`Big`/`Res`;
        checked i64 arithmetic (`OVERFLOW` per SPEC §11) with an in-tree limb
        `big` (no extra crate; `f64` conversion is round-to-nearest-even and
        matched to CPython); §19 float traps; width-result typing (`wr`/`st`/`ck`
        yield `int` or `big` by the width). Words: `b`, `gr`/`rq`/`hv`, width
        tags, vector ctors/`vadd`/`vsub`/`vmul`/`vsum`/`vat`, `??` (ok/err
        unwrap), `at`, `'x` (frontend already), `STACK_OVERFLOW` at depth 128.
        `vm_parity.py` covers v0.1 and v0.2: **59/59**. WASM `int` still wraps
        (M8d-iv). A `big` exponent that does not fit `u32` is an
        implementation-defined `OVERFLOW` (the oracle has no such budget).
      - [x] **M8d-iv IR + WASM.** `Match` (`??`) and `Attempt` (the name `at`)
        round-trip with the AST (`ir_roundtrip` 45/45). WASM integer `+ - * _`
        and integer `pw`/`ab` trap on overflow instead of wrapping. `ConstCap`,
        `Move`, `Match`, `Attempt`, and v0.2 words are a structured
        `UNSUPPORTED` naming M9. `wasm_parity` stays 13/13.
      - [x] **M8d-v `zown-check`.** Port of `checker.py` (name resolution, match
        shape, capability flow, result exhaustiveness, use-after-move).
        `zownc check` prints the golden line format; `zownc run` is gated
        (exit 2, `--unchecked` skips it). `check_parity.py` is 14/14. The
        runnable corpus checks clean. `vm_parity` passes `--unchecked` so it
        still compares runtime behavior.
      **Acceptance:** `vm_parity` green on all v0.1+v0.2 cases; `check_parity`
      green on the check corpus; `cargo test` green; `wasm_parity` still 13/13.
      **Prereq on the current dev machine:** install Rust (`rustup`).

**Tasks (later, enforced from M9 with the memory model)**
- [ ] Fat-pointer descriptors `[base|bounds|perms]` as the only reference type.
- [ ] Ownership (move/borrow) checked during lowering; linear stack-frame
      allocation; drop reclaims; no GC.
- [ ] `Secret[T]` (auto-zero), `ct-eq`, secure RNG, capability-level rate limiting
      (needs M11 crypto).

**Acceptance (M8 overall):** capability violations and unmatched results are
compile-time `.zerr`s under both `zown check` and `zownc check` (14/14). Every
row of the no-UB table has a conformance case and passes in the oracle and the
Rust VM (59/59). WASM passes the v0.1 rows (13/13) with checked `int`; v0.2
values there are the structured `UNSUPPORTED` M8d-iv allowed, and they run in
WASM in M9. (The fuzzer/no-use-after-free acceptance moves to M9, where memory
first exists.)

**Risks:** *scope creep into the memory model before there is memory* —
mitigation: M8c-iii is spec-only until M9. *Unsound "helpful" checks* —
mitigation: the corpus-clean test is mandatory for every new check; when in
doubt, stay silent.

---

### M9 — Native backend + performance
**Goal:** real desktop binaries, built with the full type system. **Prereq: M8d.**

**Decision:** **Cranelift** first (pure-Rust, fast builds, ms JIT for hot-swap);
add **LLVM** (`inkwell`) later for `-O` and bare metal. One IR feeds both.

**Tasks**
- [x] **M9a.** Each IR block is a Cranelift function. Calls and `while` are native; values, operators, and words go through `zown-rt` (the reference VM), so results match the tree-walker. `zownc native` runs in-process. `zownc native -o prog.exe` emits an object and links it with the host `rustc` (the exe’s `main` is Rust, so the allocator and stdout are initialized). Verified on Windows x64 gnu: v0.1 conformance 20/20, plus the v0.2 programs that do not use `??`, `at`, `gr`, or `f4`/`d2`/`i4`/`b16` (`conformance/native_parity.py`). ELF/Mach-O use the same object path and are not separately exercised here.
- [x] **M9b.** `??`, `ty`, `gr`, and `vf`/`vd`/`vi`/`vu` run as native calls (D6 renamed the earlier spellings `at` and `f4`/`d2`/`i4`/`b16`). The runtime keeps each block's instruction shape so `??` can read patterns, then calls the matching arm. `ty` and the vector constructors run the block on a fresh stack; `gr` grants the capability for the call and drops it after. A bound name still shadows the word. `native_parity` is 59/59, and a linked exe that uses `??` matches the JIT.
- [x] **M9c.** When every operand is a plain `int`, `+ - * _` and `== != < > <= >=` are Cranelift instructions. Overflow is `OVERFLOW` (the same code the VM raises from `fit_int`). Floats, bigs, `/`, `%`, stack ops, and words still call `zown-rt`. The JIT object is non-PIC; the linked object is PIC so the host `rustc -pie` link succeeds. `native_parity` stays 59/59.
- [x] **M9d.** `buf` / fat pointers are SPEC §21, *design-only*. The oracle does not allocate one. Native `big` is a sign plus little-endian base-2^32 limbs (D7). WASM tags 4–8 are assigned in `WASM.md` (D8) and are not emitted yet.
- [x] **M9e.** `va`/`vs`/`vm` on `vi`/`vf`/`vd`, and `va`/`vs` on `vu`, are Cranelift vector instructions (the host ISA picks SSE/AVX or the equivalent). `vm` on `vu` stays on the VM: this x86 Cranelift has no `imul.i8x16`. `native_parity` stays 59/59.
- [x] **M9f.** Two `zownc native -o` builds of the same source are byte-identical (`SOURCE_DATE_EPOCH`, no build-id, stripped, remapped paths). `zownc native -O` uses Cranelift `opt_level=speed`. `zownc llvm -o exe` emits LLVM IR and links it with `clang -O2` against `libzown_rt.a`. Bare metal remains M17. Zero-copy I/O stays with the network milestone.

**Acceptance:** native binaries pass conformance on the developer's host
(Windows x64 today; macOS/arm64 and Linux x64 before M11); two builds of the
same source are byte-identical; a fuzzer finds no OOB/use-after-free.

---

### M10 — Concurrency: dynamic fast lanes
**Goal:** make `~ ^ |` real and separate the data plane from the control plane.

**Tasks**
- [x] **M10a.** `hw` reads the CPU count (`ZOWN_CPUS` overrides it). `n lc` grants `min(n, max(cpus-1, 1))` real-time slots and pushes that number. The default request, used when `lc` has not run, is `min(2, room)`. Workers are `max(cpus - granted, 1)`. `zown tune` and `zown tune N` print cpus, the grant, the worker count, and `pinned` or `fibers`.
- [x] **M10b.** `~m` and `~n` register a block as the input and net lanes. Any other `~name` is `UNSUPPORTED` (those stay the §14 network words). Each tick runs the pinned lanes, then the lanes that did not fit (fibers), then at most one compute job. Both classes run every tick, so a fiber is not a skipped lane. The trace (`sg`) is the timing model: `cases_v2/lanes` prints `mncmnmn` on every machine because `1 lc` grants one slot.
- [x] **M10c.** `^` queues a block on worker 0. A worker pops from the left of its own deque; an idle worker steals from the right of another and `ws` counts that. `|` sends into the running lane's pipe (capacity 8). `pm` and `pn` pull. Outside a lane, `|` is `TYPE_MISMATCH`. Re-entering `rn` is `TYPE_MISMATCH`. The native runtime runs compiled lane bodies through the same helpers, so `native_parity` covers the demo and the traps.

**Acceptance:** a multi-lane demo holds real-time timing under load; control-plane
work never starves a real-time lane. Met by the trace, not by a wall-clock
sleep: five queued jobs and three ticks still print `mncmncmnc` (unit test).
The oracle cannot show OS `SCHED_FIFO`; SPEC §22 says a later pinned-thread
host must not change this trace.

---

### M11 — Batteries stdlib + `std` in Zown
**Goal:** the zero-install core, and the start of the self-host migration.

**Tasks**
- [x] **M11a.** `zown/std/core.zn` defines `mx`, `mn`, and `cl`. Both VMs load it at startup; `wd` registers each block as an auto-running word that a user binding can still shadow. A trap inside the body is reported as that word. `ab` stays a host intrinsic: `<` cannot see the sign bit of `-0.0`. `words_math` and `test_ok` pass on the oracle and on `zownc native`.
- [x] **M11b.** `zown test [path]` runs `*.zn` against a sibling `.out` or `.code`, or as a pass/fail script. `[body] ts` and `k [body] pp` are the in-language checks (`TEST_FAIL` when the body leaves a false value). `zown fmt` canonicalizes whitespace and is idempotent. `-w` writes the file.
- [x] **M11c.** Crypto (BLAKE3, ChaCha20-Poly1305, Ed25519, X25519), files, clocks, collections, LSP, and the doc generator are SPEC §23, *design-only*. They are not stubbed. The words `h3` `ae` `ad` `es` `ev` `xk` are reserved there and are not in the VM.

**Acceptance:** a meaningful subset of `WORDS` is defined in Zown and passes the
same conformance cases; `zown test`/`zown fmt` ship and work. Met. `native_parity`
is 69/69.

---

### M12 — ZownNet (decentralized mesh)
**Goal:** networking native to the language (see `DESIGN.md` §3).

**Tasks**
- [x] **M12a.** `h3` is unkeyed BLAKE3 of a string's UTF-8, checked against the official vectors (empty, 1 byte, 1024, 1025, and the repeating 251-byte pattern). `~p` stores a bundle under that digest and `~f` reads it. Both VMs agree (`cases_v2/bundle`). A missing hash is `BOUNDS`.
- [x] **M12b.** A node id is 32 bytes (`ni`). `ZOWN_NODE` fixes it as `BLAKE3` of that string; otherwise it is random. `ua` prints an IPv6 ULA in `fd00::/8` from the first 16 bytes, with the first byte forced to `0xfd`. It is not an Ed25519 public key. Ed25519, X25519, and ChaCha20-Poly1305 stay §23.
- [x] **M12c.** `zown mesh` binds two IPv6 UDP sockets. They rendezvous at `[::1]:59770` (or the given port), announce, and one fetches `hello bundle` by hash. The announce is also sent to `ff02::5a77`; this host does not deliver that group, so the tested path is the `[::1]` rendezvous, which never touches the internet. QUIC, Kademlia, relays, and `~o`/`~s`/`~r`/`~c`/`~l`/`~w` are not implemented.

**Acceptance:** two peers discover each other and exchange a content-addressed
bundle with no internet. Met on `[::1]` by `zown mesh`. A second physical NIC
that delivers `ff02::5a77` uses the same datagrams; this environment does not.

---

### M13 — Native UI + ZownGPU + audio + input
**Goal:** a full frontend with no HTML/CSS (see `DESIGN.md` §4–§5).

**Tasks**
- [x] **M13a.** `co` is a color (`r g b`, channels `0..255`). `tx` and `bt` are leaves. `rw` and `cm` run a block on a fresh stack and lay the nodes out, start-aligned, with `pad` and `gap`. `ly` pushes the preorder dump. Both VMs match (`cases_v2/layout`). A bad channel is `BOUNDS`; a non-node child is `TYPE_MISMATCH`.
- [x] **M13b.** Leaf contrast is `ok` or `low` against the parent box (black if the leaf is the root). Luma is linear Rec. 709 in integer arithmetic, so Python and Rust cannot drift. The button's accessible name is its label.
- [x] **M13c.** `zown draw` paints the one node a program leaves as a `P6` PPM: parent rectangle first, children on top. GPU, shaders, animation, input delivery, audio, and PDF stay design-only (§15). The picture is the CPU rectangles, not a font.

**Acceptance:** a sample scene lays out and paints without HTML. Met by
`examples/ui.zn` and `zown draw`. A screen reader, animation, and PDF are not
in this slice. `native_parity` is 74/74.

---

### M14 — Embedded store + distribution
**Goal:** drop the external DB; make the mesh the version-control system
(see `DESIGN.md` §6–§7).

**Tasks**
- [ ] mmap graph + columnar stores; lazy schema evolution; CRDT sync types.
- [ ] Token-dense query pipeline (`» « →`).
- [ ] Signed module-version chain; distributed transparency log + inclusion proofs.
- [ ] Semantic patch protocol + four-tier trust hierarchy; signed `SyncBundle`
      (USB/GitHub export/import); conflict resolution.

**Acceptance:** an app and its history survive a crash; a tampered SyncBundle is
rejected; a signed patch hot-swaps only after independent verification.

---

### M15 — AI control plane + rolling execution
**Goal:** an AI manages every instance in real time (see `DESIGN.md` §8).

**Tasks**
- [ ] Continuous telemetry in the manifest format; distributed tracing; structured
      metrics/logs tagged with module version + content hash.
- [ ] Declarative intent API (desired state); runtime converges via hot-swap +
      mesh routing.
- [ ] Zero-downtime hot-swap; self-healing loop (`.zerr` → semantic patch → verify
      → swap → confirm → record); broadcast verified fixes.
- [ ] On-device ML inference (ONNX, tensors, quantized local LLM); federated hooks.

**Acceptance:** a fleet converges to declared intent; an injected bug is detected,
patched, verified, and hot-swapped with no downtime.

---

### M16 — Self-hosting (the endgame)
**Goal:** the Zown compiler is written in Zown.

**Strategy (bootstrap chain):**
1. Rust compiler (`zownc`) is the **stage-0** compiler.
2. Write the Zown frontend (lexer/parser/lowering) **in Zown**; compile it with
   stage-0 → **stage-1** compiler binary.
3. Use stage-1 to compile the same Zown sources → **stage-2**.
4. **Fixed-point check:** stage-1 and stage-2 binaries must be identical. When
   they match, Zown self-hosts.
5. Rust `zownc` is retained only as a bootstrap + differential oracle.

**Prereqs:** M4 (semantics), M9 (a backend that can emit the compiler binary), M8
(memory model strong enough to write a compiler), M11 (stdlib in Zown).

**Acceptance:** `stage1 == stage2` (byte-identical), and stage-2 passes the full
conformance suite.

---

### M17 — Zown OS / bare metal
**Goal:** prove "build anything," including kernels.

**Tasks**
- [ ] `--target=bare` (x86_64-unknown-none): no std, freestanding.
- [ ] MMIO + a bootable image that writes to the VGA text buffer.
- [ ] Minimal interrupt handler demo.

**Acceptance:** a `.bin` boots in QEMU and prints to screen. End state: a
microkernel in Zown whose trusted core is only hardware abstraction + capability
enforcement + isolation; everything else is a Zown module.

---

## Open decisions

Decisions that shape the ABI or the SPEC and are **not yet made**. Each gets a
recommended default so work can proceed; the maintainer confirms or overrides.
Resolve one by writing it into `SPEC.md` and moving it to the log below.

*(none open right now — the next batch will come out of M8d/M9: the native
representation of `big`, the WASM tags for the new value kinds, and the `buf`
memory type.)*

### Decision log

| # | Decision (date) | Where it lives |
|---|-----------------|----------------|
| D1 | `int` is a **checked 64-bit** integer; `big` is the explicit arbitrary-precision type, spelled `b` (2026-09-07) | SPEC §1, §11; M8c-i |
| D2 | `zown run` **is gated** by `zown check`; rejection is exit **2**; `--unchecked` for debugging; the VM API stays ungated (2026-09-07) | SPEC §8; M8c-0 |
| D3 | Results are consumed through **`??`** (`[ok]`/`[err]`), not `?` (2026-09-07) | SPEC §16.1; DESIGN §2.2; M8c-ii |
| D4 | `!`/`&` stay not/over; **`'x` is move**, a bare name is borrow; `"` reserved for a mutable-reference sigil (2026-09-07) | SPEC §3, §17, §20; M8c-iv |
| D5 | `.zerr` version **0.2** (2026-09-07) | `errors.py`; SPEC §8 |
| D6 | **One token, one meaning. A prebuilt word is 1, then 2, then at most 3 characters** (2026-10-24). `+`/`*` are numeric (`ct` concatenates, `rp` repeats). `_` negates (`any` is the default pattern; there is no `bool` type). `ty` attempts; `at` indexes. Vectors are `vf vd vi vu` with `va vs vm vh`. Widths are `i8…i64 ix` and `u8…u64 ux`. `cl` clears. Standard net-send is `` `t ``, because `s` is `to_str`. Retired spellings are unbound, not aliases | SPEC §4, §7, §11, §12, §16 |
| D7 | Native `big` is a sign plus little-endian base-2^32 limbs, no leading zero, zero non-negative (2026-10-24) | SPEC §11; `zown-vm` `BigInt` |
| D8 | WASM tags 4–8 are big, cap, width, vec, res (2026-10-24). Not emitted yet | `WASM.md` |
| D9 | Fast lanes are a **cooperative tick trace**, not OS threads (2026-10-24). `room = max(cpus-1, 1)`, grant `min(request, room)`, default request `min(2, room)`. Each tick runs every registered lane, then at most one job. `\|` sends; `pm`/`pn` pull. Pipe capacity 8. `~m`/`~n` only; other `~name` is `UNSUPPORTED` | SPEC §22; M10 |
| D10 | Std words are Zown blocks registered with `wd` (2026-10-24). `mx`, `mn`, and `cl` live in `std/core.zn`. `ab` stays a host word because `<` cannot see `-0.0`. Crypto and files are named in §23 and not stubbed | SPEC §7, §23; M11 |
| D11 | The mesh slice is **BLAKE3 plus IPv6 UDP**, not QUIC (2026-10-24). `h3`/`~p`/`~f` are the content address. The node id is not an Ed25519 key. `zown mesh` exchanges one bundle on `[::1]` | SPEC §24; M12 |
| D12 | UI in the oracle is a **layout tree and a PPM**, not a GPU (2026-10-24). `co tx bt rw cm ly` build it. Contrast is integer luma. `zown draw` paints rectangles | SPEC §25; M13 |
| — | M8c-iii ownership *enforcement* re-sequenced into M9 (memory model) — the rules are specified now (2026-09-07) | SPEC §19/§20; M9 |

---

## Known debts / carry-over

Independent, unblocking items. Pick them up opportunistically or attach them to
the milestone noted.

| Debt | Where | Attach to |
|------|-------|-----------|
| WASM has no tags for cap/width/vec/big/res; those programs are `UNSUPPORTED` (not a silent wrong answer). Checked `int` traps with `unreachable`, not a `.zerr` | `zown-wasm` | rest of **M9** |
| WASM still does not *emit* tags 4–8, `Move`, `??`, or `ty` (the numbers are D8). Checked `int` overflow in WASM is `unreachable`, not a `.zerr` | `zown-wasm` | after **M9**, before mesh/UI need those values on the web |
| `vm` of a `vu` vector is not a Cranelift `imul.i8x16` (the x86 backend has no such instruction); it runs on the VM | `zown-codegen` | only if a future Cranelift grows the op |
| `zownc llvm` needs `libzown_rt.a` (`cargo rustc -p zown-rt --crate-type staticlib`) and `clang` | `zown-codegen` | document in the README if the llvm path is used in CI |
| Rust runtime `.zerr` packets omit `pos` (no spans in the Rust AST) | `zown-ast`, `zown-vm` | M8d-i |
| `zownc check` gates `run`; IR has `Match`/`Attempt`. WASM still refuses v0.2 values | `zown-wasm` | **M9** |
| `$fmt_f64` is exact for dyadic floats but not shortest-round-trip (Ryu/Grisu); must also print `inf`/`-inf`/`nan` and keep `1e+30` per §11 | `zown-wasm` | M8d-iv / M9 |
| No CI: nothing runs `tests/` + the oracle runners (+ `cargo test`) on push | repo | add a GitHub Actions workflow: Python 3.10+ matrix running the five oracle commands; a Rust job once M8d lands |
| `pyproject.toml` says `>=3.10`; 3.9 works — either lower the floor or start using 3.10 syntax deliberately | `pyproject.toml` | maintainer call |
| WASM float formatting only exercised by one fractional value (`2.5`) in the v0.1 corpus (the v0.2 corpus now has `float_ieee`) | `conformance/cases` | M8d-iv |
| The oracle has no resource limits for `big` (a `2 b 1000000000 pw` allocates ~125 MB; `ty`-wrapped recursion can be made exponential) — implementation-defined per §19, but a native runtime should expose a budget | `zown/builtins.py` | M9/M15 (telemetry budget) |
| `Res` err payload is the code string only; the full packet (op/pos/cap/msg) is not reachable from inside the language yet — the self-healing loop (M15) will want it | `zown/vm.py` | M11 (`std`) |
| `tele.errors` in the manifest could be auto-discovered from `ty`/`rq`/`??` usage | `zown/manifest.py` | M11 |

---

## Conventions

- **Density audit:** any new surface syntax must justify its token cost; prefer a
  symbol over a word; update `SPEC.md` and the manifest docs together. New tokens
  are *provisional* until audited.
- **Oracle discipline:** never change observable semantics without first updating
  the conformance suite (and a note in `SPEC.md` on resolved ambiguities).
- **Checker soundness:** a static diagnostic is a definite *unhandled* fault or a
  violation of a SPEC-mandated static rule — never a guess. Every runnable
  program in the corpus must check clean; if a check cannot be made sound, it is
  not added (a runtime `.zerr` still catches it). Because `run` is gated, a false
  positive is a false rejection.
- **Errors are values when asked:** any new runtime trap must be a `.zerr` (so
  `ty` can catch it) and must get a row in SPEC §19 with a golden — no new code
  path may crash the oracle with a host exception.
- **Design-only is honest:** anything the oracle cannot exhibit faithfully
  (crypto, I/O, rendering, memory) is written into the SPEC with its target
  milestone and *not* stubbed.
- **Commit style:** imperative, scoped (`M8b: …`, `lexer: …`, `vm: …`, `docs: …`);
  body states semantics, files, and what was not touched.
- **No silent breakage:** a backend that diverges from the oracle is a bug in the
  backend, not the oracle, until the spec says otherwise.

## Immediate next actions (pick up here)

1. **M14 — store.** The graph, the query tokens, and signed bundles. Layout is
   SPEC §25; GPU and audio are still §15.
2. **Carry-over:** CI (Python oracle + `cargo test` + `native_parity`); Ryu/Grisu
   `$fmt_f64`; emit WASM tags 4–8 (lane ops included — they are refused today);
   auto-discover `tele.errors`.
3. **Windows link.** No MSVC `link.exe` on the 2026-09 machine. Build with
   `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu`. Do not change
   `rust-toolchain.toml`.
