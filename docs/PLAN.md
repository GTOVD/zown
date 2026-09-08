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

Last updated: 2026-09-07 (late — M8d-i frontend in Rust).

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
validated in the Python oracle **before** the native compiler (M7 ✅, M8 🔄),
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
| M8 | Safety core (type & memory model) | 🔄 | M8a–M8c ✅; **M8d-i ✅** Rust frontend (Cap/`'x`/`??`/checked int literals + AST spans); **⏳ M8d-ii** checked-int VM + `big` |
| M9 | Native backend + perf (Cranelift→LLVM) | ⬜ | `-o .exe`, SIMD, zero-copy I/O, deterministic builds — **blocked on M8d** (needs v0.2 in Rust) |
| M10 | Concurrency: dynamic fast lanes (`~ ^ \|`) | ⬜ | app-defined lanes; data plane vs control plane split |
| M11 | Batteries stdlib + `std` in Zown | ⬜ | crypto/collections/numerics/fs/time/secrets + test/fmt/LSP/doc; starts self-host migration |
| M12 | ZownNet — decentralized mesh | ⬜ | IPv6 crypto-identity, ZownTransport, DHT+gossip, content addressing, offline mesh |
| M13 | Native UI + ZownGPU + audio + input | ⬜ | typed layout tree (no HTML/CSS), wgpu→native, a11y, i18n |
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
python3 tests/test_zown.py          # unit tests (98)
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

- [ ] **M8d — Rust toolchain reaches v0.2 + `zownc check`.**  🔄
      The frontend now speaks v0.2. The VM still runs v0.1 arithmetic (wrapping
      `i64`, no `big`/`Res`/widths/vecs/words). Nothing in M9 can start until
      the rest of this milestone lands. Remaining slices:
      - [x] **M8d-i frontend.** Lexer: `Cap` + `Move` tokens, `??`, the int-literal
        range check (`OVERFLOW` past `i64::MAX`); AST: `cap`/`move` nodes plus a
        source `Pos` on every node (JSON still omits it, matching `zown ast`);
        `zownc ast` parity on v0.1 + v0.2 + check + examples (45/45). IR gained
        `ConstCap`/`Move` so unlower stays lossless (45/45). Runtime `.zerr`s
        now carry `pos` (zerr 0.2). WASM refuses those two instrs with a
        structured `UNSUPPORTED` naming M8d-iv. `ast_parity.py` /
        `ir_roundtrip.py` scan `cases_v2` and `check/`.
      - **M8d-ii VM values + numerics.** ⏳ `WidthTag`/`Vec`/`Big`/`Res` values;
        **checked i64 arithmetic** (`OVERFLOW` per SPEC §11 — today the Rust VM
        and WASM wrap silently, which is now non-conformant) with `b` (`i128`
        fast path spilling to a bigint crate, or a small bigint); the §19 float
        rules; the width-result typing rule. `Cap` tokens already push.
      - **M8d-iii VM words.** `gr rq hv`, `wr st ck`, width words, vector
        ctors/ops, `??` with ok/err unwrap, `at` (isolated stack), `'x`,
        `STACK_OVERFLOW` depth trap, `.zerr` `cap` field. Parity: `vm_parity.py`
        over the v2 corpus (59 cases total).
      - **M8d-iv IR + WASM.** `Match` + `Attempt` instructions (`ConstCap`/`Move`
        already land in M8d-i). WASM: new
        tags after `3` for cap/width/vec/big/res, or a structured `UNSUPPORTED`
        naming M9 — never a silent divergence; `wasm_parity` stays 13/13.
      - **M8d-v `zown-check`.** Port `checker.py` (name resolution, match shape,
        capability flow, result exhaustiveness, use-after-move) to a crate;
        `zownc check` and the gate in `zownc run` (exit 2); `check_parity.py`
        over `conformance/check/`.
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
compile-time `.zerr`s; every row of the no-UB table has a conformance case and
passes in the oracle, the Rust VM, and WASM; the checker corpus runs identically
under `zown check` and `zownc check`. (The fuzzer/no-use-after-free acceptance
moves to M9, where memory first exists.)

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
- [ ] IR → Cranelift IR → object → link to `x.exe`/ELF/Mach-O.
- [ ] Runtime shim (entry, print, alloc) as a tiny static lib.
- [ ] Memory model from M8c-iii: `buf` values, fat pointers, bounds traps.
- [ ] SIMD lowering (SSE/AVX/NEON by target); zero-copy I/O (`io_uring`/IOCP).
- [ ] **Deterministic builds** (bit-identical output — content hashing depends on it).
- [ ] Conformance run for the native target (v0.1 + v0.2 corpora).

**Acceptance:** native binaries pass conformance on the developer's host
(Windows x64 today; macOS/arm64 and Linux x64 before M11); two builds of the
same source are byte-identical; a fuzzer finds no OOB/use-after-free.

---

### M10 — Concurrency: dynamic fast lanes
**Goal:** make `~ ^ |` real and separate the data plane from the control plane.

**Tasks**
- [ ] Hardware topology query; application-defined real-time lane counts.
- [ ] `~n` net / `~m` input lanes pinned off the compute pool; degrade to fibers.
- [ ] `^` work-stealing pool; `|` lock-free pipe; `zown tune` live rebalance.

**Acceptance:** a multi-lane demo holds real-time timing under load; control-plane
work never starves a real-time lane.

---

### M11 — Batteries stdlib + `std` in Zown
**Goal:** the zero-install core, and the start of the self-host migration.

**Tasks**
- [ ] Crypto suite (BLAKE3, ChaCha20-Poly1305, Ed25519, X25519), collections,
      complete numerics, UTF-8 strings, time/clocks, filesystem, env/secrets.
- [ ] Developer toolchain: built-in testing (+ property-based), LSP, canonical
      formatter, doc generator, mesh-aware REPL.
- [ ] Reimplement pure-Zown stdlib words in `std/`; bootstrap loader links `std/`;
      keep Rust intrinsics only for true primitives (syscalls, memory, SIMD).

**Acceptance:** a meaningful subset of `WORDS` is defined in Zown and passes the
same conformance cases; `zown test`/`zown fmt` ship and work.

---

### M12 — ZownNet (decentralized mesh)
**Goal:** networking native to the language (see `DESIGN.md` §3).

**Tasks**
- [ ] Crypto identity → IPv6 (`BLAKE3(pubkey)`); IPv6-only.
- [ ] ZownTransport (QUIC-equivalent over UDP via FFI first): encrypted-by-default,
      multiplexed, 0-RTT, mutual auth.
- [ ] Discovery: multicast-local + Kademlia DHT + gossip (no DNS).
- [ ] Content addressing + re-hosting; NAT traversal + relays; offline mesh.
- [ ] `~`-family ops wired to the transport; protocol-as-type compatibility checks.

**Acceptance:** two devices on a LAN discover each other and exchange a
content-addressed bundle with zero config; an offline pair communicates with no
internet.

---

### M13 — Native UI + ZownGPU + audio + input
**Goal:** a full frontend with no HTML/CSS (see `DESIGN.md` §4–§5).

**Tasks**
- [ ] Typed declarative layout tree; retained scene graph; fl/grid; typed style;
      responsive-as-constraint; animation.
- [ ] ZownGPU over `wgpu` (FFI) → native Vulkan/Metal/DX12 later; shaders in a
      Zown subset; text shaping in stdlib.
- [ ] Audio signal-graph + low-latency lane; unified input + sensors
      (capability-gated); accessibility + i18n in the node types; PDF/PNG export.

**Acceptance:** a sample app renders, animates, takes input, reads a screen reader
tree, and exports itself to PDF.

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
| — | M8c-iii ownership *enforcement* re-sequenced into M9 (memory model) — the rules are specified now (2026-09-07) | SPEC §19/§20; M9 |

---

## Known debts / carry-over

Independent, unblocking items. Pick them up opportunistically or attach them to
the milestone noted.

| Debt | Where | Attach to |
|------|-------|-----------|
| Rust VM/WASM integers **wrap silently**; SPEC §11 now requires a checked `int` + `big` — the Rust side is non-conformant to v0.2 until this lands | `zown-vm`, `zown-wasm` | **M8d-ii** |
| Rust runtime `.zerr` packets omit `pos` (no spans in the Rust AST) | `zown-ast`, `zown-vm` | M8d-i |
| `zownc` implements v0.1 only (no caps/numerics/vecs/`??`/`at`/`'x`/checker/gate) | `zownc/` | **M8d** |
| `$fmt_f64` is exact for dyadic floats but not shortest-round-trip (Ryu/Grisu); must also print `inf`/`-inf`/`nan` and keep `1e+30` per §11 | `zown-wasm` | M8d-iv / M9 |
| No CI: nothing runs `tests/` + the oracle runners (+ `cargo test`) on push | repo | add a GitHub Actions workflow: Python 3.10+ matrix running the five oracle commands; a Rust job once M8d lands |
| `pyproject.toml` says `>=3.10`; 3.9 works — either lower the floor or start using 3.10 syntax deliberately | `pyproject.toml` | maintainer call |
| WASM float formatting only exercised by one fractional value (`2.5`) in the v0.1 corpus (the v0.2 corpus now has `float_ieee`) | `conformance/cases` | M8d-iv |
| The oracle has no resource limits for `big` (a `2 b 1000000000 pw` allocates ~125 MB; `at`-wrapped recursion can be made exponential) — implementation-defined per §19, but a native runtime should expose a budget | `zown/builtins.py` | M9/M15 (telemetry budget) |
| `Res` err payload is the code string only; the full packet (op/pos/cap/msg) is not reachable from inside the language yet — the self-healing loop (M15) will want it | `zown/vm.py` | M11 (`std`) |
| `tele.errors` in the manifest could be auto-discovered from `at`/`rq`/`??` usage | `zown/manifest.py` | M11 |

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
  `at` can catch it) and must get a row in SPEC §19 with a golden — no new code
  path may crash the oracle with a host exception.
- **Design-only is honest:** anything the oracle cannot exhibit faithfully
  (crypto, I/O, rendering, memory) is written into the SPEC with its target
  milestone and *not* stubbed.
- **Commit style:** imperative, scoped (`M8b: …`, `lexer: …`, `vm: …`, `docs: …`);
  body states semantics, files, and what was not touched.
- **No silent breakage:** a backend that diverges from the oracle is a bug in the
  backend, not the oracle, until the spec says otherwise.

## Immediate next actions (pick up here)

1. **M8d-ii — checked-int VM + `big`.** The Rust frontend now lexes/parses v0.2
   (`zownc ast` 45/45). The VM still does wrapping `i64` arithmetic and has no
   `big`/`Res`/`WidthTag`/`Vec`. Implement SPEC §11 in `zown-vm` next, then
   M8d-iii (words: `gr rq hv`, `at`, `??` unwrap, …). On this Windows machine
   there is no MSVC `link.exe`; build with
   `RUSTUP_TOOLCHAIN=stable-x86_64-pc-windows-gnu` (do **not** change
   `rust-toolchain.toml` — macOS/Linux keep the default host).
2. **Density audit of the v0.2 tokens** before M9 freezes the ABI: `at`, `b`,
   `gr rq hv`, `wr st ck`, `vadd…`, `??`, `'`. Each is provisional per SPEC Part
   II's header; confirm or rename them in one SPEC edit + `--bless` pass.
3. **M9 groundwork that needs no Rust:** write the `buf` (byte buffer) type and
   the fat-pointer rules into SPEC Part II as *design-only (M9)* so M9's memory
   model has a spec to implement; decide the native representation of `big`
   (bigint crate vs i128 fast path) and the WASM tags for the new value kinds.
4. **Carry-over (independent):** CI workflow; Ryu/Grisu `$fmt_f64`; auto-discover
   `tele.errors` in the manifest.
