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

**Sequencing rule (new):** the security/type/network/graphics SPEC is frozen and
validated in the Python oracle **before** the native compiler (M7), because those
decisions shape the ABI and codegen. See `DESIGN.md` §1.

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
| M5 | IR + lowering | ✅ | `zown-ir`; lossless round-trip on all 16 programs |
| M6 | WASM backend (`-o .wasm`) | ✅ | Full v0.1 -> `.wat` + binary `.wasm`; all 13 cases run in wasmtime |
| M7 | **Design freeze: SPEC v0.2 + oracle** | 🔄 | M7a capabilities ✅, M7b manifest v2 ✅; remaining: numerics, SIMD/tensor, net/UI/crypto types, match |
| M8 | Safety core (type & memory model) | ⬜ | fat ptrs, `! & ?`-tuple, no-UB, capability flow in the checker |
| M9 | Native backend + perf (Cranelift→LLVM) | ⬜ | `-o .exe`, SIMD, zero-copy I/O, deterministic builds |
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
    zown-codegen/     # LLVM/Cranelift backend (M9)
    zown-types/       # type & capability checker (M8)
    zown-net/         # ZownNet: transport, DHT, gossip, content addressing (M12)
    zown-ui/          # layout engine + ZownGPU bindings (M13)
    zown-store/       # embedded graph+columnar store, CRDTs (M14)
    zown-mesh/        # version chain, transparency log, patch protocol, sync (M14)
    zown-ctl/         # AI control plane: telemetry, intent, hot-swap (M15)
    zown-cli/         # the `zownc` binary (driver)
  Cargo.toml          # workspace manifest
conformance/          # language-agnostic golden tests (.zn + expected stdout/.zerr)
std/                  # standard library written in Zown (self-host prep, M11+)
examples/
docs/                 # DESIGN / SPEC / ROADMAP / PLAN / MANIFEST / WASM / IR
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
      `compare`, `logic`, `stackops`, `strings`, `words_str` run in wasmtime and
      match the goldens.
- [x] **M6c — blocks + control.** Operand stack moved to linear memory (`$sp`) so
      blocks (compiled to `() -> ()` functions in a `funcref` table) share it.
      `[ … ]`, `@`→`call_indirect`, `?`→`select`, `;`→`block`/`loop`, `:bind`/name
      load via per-name globals. `hello`, `select`, `while`, `fib`, `fizzbuzz` run
      in wasmtime and match the goldens — **10/13** cases now green.
- [x] **M6d — floats + binary.** Float values (tag `1`, `f64` bit pattern) with
      int→float promotion; `/ % _`, comparisons, and the math words `sq pw ab mx
      mn fl ce rd`, plus `n s dp clr pr`. `$fmt_f64` renders floats per the
      oracle's display rule. Binary `.wasm` via the `wat` crate (`build -o x.wasm`).

**Acceptance (M6 overall): MET.** All 13 conformance cases compile and match the
goldens under wasmtime, in **both** `.wat` and binary `.wasm`
(`conformance/wasm_parity.py`: 13 ok, 0 fail, 0 skip).

**Known limitation:** float-to-decimal is exact for dyadic values but not yet a
shortest round-tripping formatter (Ryu/Grisu) — see `docs/WASM.md`.

---

### M7 — Design freeze: SPEC v0.2 + oracle  🔄 (in progress)
**Goal:** freeze the ABI-shaping language decisions and prove them in the Python
oracle **before** any native codegen. Nothing here needs a new backend.

Built in slices. v0.2 semantics are validated against the oracle only; the corpus
lives in `conformance/cases_v2/` + `errors_v2/` (run `python3 conformance/v2.py`),
kept separate so the Rust `zownc` parity runners stay green until they implement
v0.2.

- [x] **M7a — capabilities (the security core).** `` ` `` capability sigil lexes
      to a `Cap` token; zero authority by default; `gr` (grant-scope), `rq`
      (require → `CAP_DENIED`, kind `sec`, with a `cap` field), `hv` (non-fatal
      probe). Security `.zerr` codes added (`CAP_DENIED`, `AUTH_FAIL`,
      `INTEGRITY_FAIL`, `SIG_INVALID`, `RATE_LIMITED`, `UB_TRAP`). 5 v0.2
      conformance cases + 8 unit tests; v0.1 behavior and Rust parity unchanged.
- [x] **M7b — manifest v2.** `zown/manifest.py` emits the v2 shape: per user
      symbol `caps` (discovered from the bound block's body + merged with authored
      ones), `sec`/`tele`/`i18n` (scaffolded), and a module provenance block
      (crypto fields null until M14). Builtins keep the lean v1 shape; the
      never-clobber merge is preserved. 3 unit tests (`MANIFEST.md` v2).
- [ ] **M7c — numeric model.** Full width set (`i8…i128 u8…u128 f32 f64 dec big
      cx`) with explicit overflow (`wrap`/`sat`/`chk`); SIMD/vector + tensor types.
- [ ] **M7d — pattern matching** + the remaining type primitives in `SPEC.md`
      Part II (crypto `Key Sig Hash NodeID`, network `~`-family + protocol-as-type,
      UI/GPU types) specified and validated where observable.

**Acceptance:** `SPEC.md` Part II complete and internally consistent; the oracle
implements every semantically-observable addition; `conformance/v2.py` green.

**Risks:** scope creep. Mitigation: types/semantics only — implementations are
M8+. Anything that cannot be observed in the oracle is documented, not coded.

---

### M8 — Safety core (type & memory model)
**Goal:** make the "unhackable" guarantees real in the checker + runtime.

**Tasks**
- [ ] Fat-pointer descriptors `[base|bounds|perms]` as the only reference type.
- [ ] Compile-time null elimination: fallible ops yield `[ok? | data]`; `?` must
      consume before data is reachable.
- [ ] Ownership tokens `!` (move) / `&` (borrow) checked during parse/lowering.
- [ ] Linear stack-frame allocation; drop reclaims; no GC.
- [ ] **No undefined behavior**: otherwise-undefined ops are compile-time `.zerr`
      or structured runtime traps.
- [ ] **Capability flow** in the type checker; `Secret[T]` (auto-zero), `ct-eq`,
      secure RNG, capability-level rate limiting.

**Acceptance:** ownership/bounds/capability violations are compile-time `.zerr`s;
a fuzzer finds no use-after-free / OOB; an over-privileged call fails to type.

---

### M9 — Native backend + performance
**Goal:** real desktop binaries, built with the full type system.

**Decision:** **Cranelift** first (pure-Rust, fast builds, ms JIT for hot-swap);
add **LLVM** (`inkwell`) later for `-O` and bare metal. One IR feeds both.

**Tasks**
- [ ] IR → Cranelift IR → object → link to `x.exe`/ELF/Mach-O.
- [ ] Runtime shim (entry, print, alloc) as a tiny static lib.
- [ ] SIMD lowering (SSE/AVX/NEON by target); zero-copy I/O (`io_uring`/IOCP).
- [ ] **Deterministic builds** (bit-identical output — content hashing depends on it).
- [ ] Conformance run for the native target.

**Acceptance:** native binaries pass conformance on this machine (macOS/arm64);
two builds of the same source are byte-identical.

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

## Conventions

- **Density audit:** any new surface syntax must justify its token cost; prefer a
  symbol over a word; update `SPEC.md` and the manifest docs together.
- **Oracle discipline:** never change observable semantics without first updating
  the conformance suite (and a note in `SPEC.md` on resolved ambiguities).
- **Commit style:** imperative, scoped (`lexer: …`, `vm: …`, `docs: …`).
- **No silent breakage:** a backend that diverges from the oracle is a bug in the
  backend, not the oracle, until the spec says otherwise.

## Immediate next actions (pick up here)
1. **M7a (capabilities) and M7b (manifest v2) are done** — the `` ` `` sigil,
   `gr`/`rq`/`hv`, the security `.zerr` codes, and the v2 manifest generator
   (caps discovery + `sec`/`tele`/`i18n` + provenance block) all run in the oracle.
2. **M7c (numeric model) is next:** full width set (`i8…i128 u8…u128 f32 f64 dec
   big cx`) with explicit overflow (`wrap`/`sat`/`chk`) and SIMD/vector + tensor
   types, validated in the oracle (`cases_v2/`). Then **M7d** (pattern matching +
   remaining type primitives: crypto, network `~`-family, UI/GPU).
3. **M8 (safety core):** type & capability checker — fat pointers, `[ok?|data]`,
   `! &` ownership, no-UB, static capability flow (today `rq` is a runtime check).
   Lands before codegen so it shapes the ABI.
4. **M9 (native backend):** Cranelift first (pure-Rust, ms JIT for hot-swap), LLVM
   later for `-O`/bare metal. Reuse the IR + tagged-value model; SIMD; deterministic.
5. **Carry-over (independent):** upgrade `$fmt_f64` to a shortest round-tripping
   formatter (Ryu/Grisu) so arbitrary floats match the oracle, not just dyadic ones.
