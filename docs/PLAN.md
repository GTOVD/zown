# Zown Development Plan (Working Doc)

This is the **living engineering plan** for Zown. `ROADMAP.md` is the high-level
"what and why"; this doc is the "how, in what order, and what done looks like".
It is meant to be picked up cold in any future session: read this top to bottom
and you know exactly where we are and what the next concrete task is.

**How to use this doc**
- The **Status Board** is the single source of truth for progress. Update it.
- Each milestone has: *Goal → Tasks → Acceptance criteria → Risks*.
- Keep tasks small enough to finish in one focused session.
- The **reference interpreter (Python) is the oracle**: every native feature must
  produce identical observable behavior, verified by the conformance suite.

Last updated: 2026-06-05.

---

## North Star

A hyper-dense, AI-native language that:
1. Minimizes tokens per feature (1–2 char everything) so huge programs fit in a
   context window.
2. Compiles to native speed (LLVM) and the web/sandbox (WASM), and down to bare
   metal (kernels).
3. Carries meaning out-of-band (shadow manifest) and fails with actionable,
   structured diagnostics (`.zerr`).
4. **Eventually self-hosts**: the Zown compiler is written in Zown. This is the
   ultimate proof the language can "build anything without external languages."

---

## Status Board

Legend: ✅ done · 🔄 in progress · ⏳ next up · ⬜ not started

| # | Milestone | State | Notes |
|---|-----------|-------|-------|
| M0 | Reference interpreter (Python) | ✅ | lexer/parser/VM/stdlib/CLI/manifest/.zerr/tests |
| M1 | Spec hardening + conformance suite | ✅ | 13 cases + 7 error cases, golden runner (`conformance/`) |
| M2 | Rust toolchain + `zownc` skeleton | ✅ | Rust 1.96 installed; `zownc lex` works + tests pass |
| M3 | Rust frontend (lexer+parser+AST) parity | ✅ | `zownc ast` == Python `zown ast` on all 16 programs |
| M4 | Tree-walking Rust VM parity | ✅ | `zownc run` == oracle on all 20 conformance cases |
| M5 | Bytecode + register/stack IR | ⏳ | stable IR the backends consume |
| M6 | WASM backend (`-o .wasm`) | ⬜ | runs in wasmtime + browser |
| M7 | Native backend via LLVM/Cranelift (`-o .exe`) | ⬜ | desktop binaries |
| M8 | Type & memory model (fat ptrs, ownership) | ⬜ | `! & ?`-tuple, bounds, no GC |
| M9 | Stdlib expansion + `std` in Zown | ⬜ | begins the self-host migration |
| M10 | Concurrency: dynamic fast lanes (`~ ^ \|`) | ⬜ | app-defined lane counts |
| M11 | Embedded graph DB (`» « →`) | ⬜ | mmap object graph, lazy schema evo |
| M12 | Rolling hot-swap + self-healing loop | ⬜ | zero-downtime swap, `.zerr` remediation |
| M13 | Communication mesh (WASI components, `~w`) | ⬜ | containerless, zero-config tunnel |
| M14 | **Self-hosting**: compiler rewritten in Zown | ⬜ | the endgame |
| M15 | Bare-metal target + toy OS/kernel demo | ⬜ | `--target=bare`, VGA "hello" |

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
        │  Lowering →    │                        │  Python ref VM  │
        │   Zown IR      │                        │   (the oracle)  │
        └───┬───────┬────┘                        └─────────────────┘
            │       │
   ┌────────▼─┐  ┌──▼─────────┐
   │ WASM emit│  │ LLVM/Cran. │   → .wasm / .exe / .bin(bare)
   └──────────┘  └────────────┘
```

- **One frontend, multiple backends.** Lower the AST to a small, explicit **Zown
  IR**; every backend consumes IR, never the AST directly.
- **The Python VM is retired only after M14**, and even then kept as a spec
  oracle for regression testing.

---

## Repo layout (target)

```
zown/                 # Python reference implementation (the oracle) — keep
zownc/                # Rust workspace (the real toolchain)
  crates/
    zown-lexer/       # lexer (zero-copy spans, later SIMD)
    zown-parser/      # parser -> AST
    zown-ast/         # AST + node defs shared across crates
    zown-ir/          # Zown IR + lowering from AST
    zown-vm/          # tree-walking interpreter (parity bring-up)
    zown-wasm/        # WASM backend
    zown-codegen/     # LLVM/Cranelift backend
    zown-cli/         # the `zownc` binary (driver)
  Cargo.toml          # workspace manifest
conformance/          # language-agnostic golden tests (.zn + expected stdout/.zerr)
std/                  # standard library written in Zown (self-host prep, M9+)
examples/
docs/                 # SPEC / ROADMAP / PLAN / MANIFEST
tests/                # Python-side tests
```

---

## Milestones in detail

### M1 — Spec hardening + conformance suite  ⏳ (do this next)
**Goal:** freeze v0.1 semantics as language-agnostic golden tests so the Rust
port has an exact target.

**Tasks**
- [ ] Create `conformance/` with cases as triples: `name.zn`, `name.out`
      (expected stdout), and optional `name.zerr.json` (expected error packet,
      compared on `code`/`op` not message text).
- [ ] Port every Python unit-test scenario into conformance cases.
- [ ] Add a runner `conformance/run.py` that executes each `.zn` through the
      Python VM and diffs output; later the same cases run through `zownc`.
- [ ] Document any semantic edge cases discovered (number/print `.` ambiguity,
      string `+`/`*` overloads, truthiness, select-then-invoke arity).

**Acceptance:** `python3 conformance/run.py` is green; cases cover every operator,
every stdlib word, and at least one error per recovery code.

**Risks:** under-specifying error semantics. Mitigation: compare codes, not prose.

---

### M2 — Rust toolchain + `zownc` skeleton  🔄
**Goal:** a compiling Rust workspace with a CLI that can at least lex+print.

**Tasks**
- [x] Install Rust (stable 1.96) via rustup.
- [x] Scaffold `zownc/` workspace + crates (`zown-lexer`, `zown-cli`).
- [x] `zownc lex file.zn` prints the token stream (first real parity target).
- [x] Rust lexer unit tests (incl. hello-world token shape == Python).
- [ ] Add `zown-parser`, `zown-ast`, `zown-ir`, `zown-vm` crates as M3/M4 begin.
- [ ] CI: `cargo test` + run conformance against the (initially partial) Rust impl.

**Acceptance:** `cargo build` succeeds; `zownc --help` and `zownc lex` work. ✅

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

**This milestone completes the rewrite of today's language in Rust.** Known v0.1
gaps to revisit: ints are `i64` (oracle is bigint) and runtime errors omit `pos`
(AST has no spans yet) — both tracked for a later pass; neither affects current
conformance.

---

### M5 — Zown IR + lowering
**Goal:** a small, explicit IR the backends share.

**Tasks**
- [ ] Define IR ops (push/const/call-word/binop/cmp/branch/loop/invoke/bind/load/store).
- [ ] Lower AST → IR; blocks become IR functions/closures with explicit arity.
- [ ] IR interpreter (sanity check) must match the tree-walker.
- [ ] IR pretty-printer + `zownc ir x.zn`.

**Acceptance:** IR interpreter passes conformance; IR is documented in `docs/IR.md`.

---

### M6 — WASM backend
**Goal:** `zownc build x.zn -o x.wasm` runs in wasmtime and the browser.

**Tasks**
- [ ] Emit a WASM module (start with `.wat` for debuggability, then binary).
- [ ] Linear-memory layout for strings/data segment; host `print` import.
- [ ] Map IR → WASM stack ops (natural fit; Zown is already stack-based).
- [ ] wasmtime-based conformance run for the WASM target.

**Acceptance:** every non-host-specific conformance case passes under wasmtime.

**Risks:** strings/host I/O via WASI. Mitigation: define a tiny host ABI first.

---

### M7 — Native backend (LLVM or Cranelift)
**Goal:** real desktop binaries.

**Decision:** start with **Cranelift** (pure-Rust, fast builds, easy embedding);
add **LLVM** (`inkwell`) later for `-O` release-grade optimization + bare metal.

**Tasks**
- [ ] IR → Cranelift IR → object → link to `x.exe`/ELF/Mach-O.
- [ ] Runtime shim (entry, print, alloc) as a tiny static lib.
- [ ] Conformance run for native target.

**Acceptance:** native binaries pass conformance on this machine (macOS/arm64).

---

### M8 — Type & memory model
**Goal:** make the safety story real (Phase 1 of ROADMAP).

**Tasks**
- [ ] Fat-pointer descriptors `[base|bounds|perms]` as the only reference type.
- [ ] Compile-time null elimination: fallible ops yield `[ok? | data]`; `?` must
      consume before data is reachable.
- [ ] Ownership tokens `!` (move) / `&` (borrow) checked during parse/lowering.
- [ ] Linear stack-frame allocation; drop reclaims; no GC.

**Acceptance:** ownership/bounds violations are compile-time `.zerr`s; a fuzzer
finds no use-after-free / OOB in generated code.

---

### M9 — Stdlib in Zown (self-host prep)
**Goal:** start moving capability out of Rust and into `std/*.zn`.

**Tasks**
- [ ] Identify stdlib words expressible in pure Zown; reimplement them in `std/`.
- [ ] Bootstrap loader: compiler links `std/` automatically.
- [ ] Keep Rust intrinsics only for true primitives (syscalls, memory).

**Acceptance:** a meaningful subset of `WORDS` is defined in Zown and passes the
same conformance cases.

---

### M10–M13 — Systems layers
Concurrency fast lanes (`~ ^ |`, app-defined lane counts, work-stealing), the
embedded mmap graph DB (`» « →`, lazy schema evolution), rolling hot-swap +
self-healing remediation loop, and the WASI-component communication mesh with
zero-config tunneling (`~w`). Each gets its own detailed breakdown when M8 lands.
See `ROADMAP.md` Phases 3–6 for scope.

---

### M14 — Self-hosting (the endgame)
**Goal:** the Zown compiler is written in Zown.

**Strategy (bootstrap chain):**
1. Rust compiler (`zownc`) is the **stage-0** compiler.
2. Write the Zown frontend (lexer/parser/lowering) **in Zown**; compile it with
   stage-0 → **stage-1** compiler binary.
3. Use stage-1 to compile the same Zown sources → **stage-2**.
4. **Fixed-point check:** stage-1 and stage-2 binaries must be identical. When
   they match, Zown self-hosts.
5. Rust `zownc` is retained only as a bootstrap + differential oracle.

**Prereqs:** M4 (semantics), M6/M7 (a backend that can emit the compiler binary),
M8 (memory model strong enough to write a compiler), M9 (stdlib in Zown).

**Acceptance:** `stage1 == stage2` (byte-identical), and stage-2 passes the full
conformance suite.

---

### M15 — Bare metal / toy OS
**Goal:** prove "build anything," including kernels.

**Tasks**
- [ ] `--target=bare` (x86_64-unknown-none): no std, freestanding.
- [ ] MMIO + a bootable image that writes to the VGA text buffer.
- [ ] Minimal interrupt handler demo.

**Acceptance:** a `.bin` boots in QEMU and prints to screen.

---

## Conventions

- **Density audit:** any new surface syntax must justify its token cost; prefer a
  symbol over a word; update `SPEC.md` and the manifest docs together.
- **Oracle discipline:** never change observable semantics without first updating
  the conformance suite (and a note in `SPEC.md` on resolved ambiguities).
- **Commit style:** imperative, scoped (`lexer: …`, `vm: …`, `docs: …`).
- **No silent breakage:** a backend that diverges from the oracle is a bug in the
  backend, not the oracle, until the spec says otherwise.

## Immediate next actions (pick up here)
1. **M5:** define the Zown IR (`zown-ir` crate) + lowering from AST. Ops:
   push/const, call-word, binop, cmp, branch/select, loop, invoke, bind, load,
   store. Blocks become IR closures with explicit arity. Add `zownc ir <file>`.
2. **M5 gate:** an IR interpreter must pass the same conformance suite as the
   tree-walker (a second oracle agreement point before codegen).
3. **Then M6 (WASM):** lower IR → `.wat`/`.wasm`; run conformance under wasmtime.
   Zown is already stack-based, so IR→WASM stack ops are a near-direct mapping.
