# Zown Roadmap

Zown is an AI-native, token-dense language **and** the self-contained substrate
built on it: a single runtime where frontend, backend, API, database, networking,
security, graphics, and distribution are all native — no external libraries, no
frameworks, no servers. Every device that runs it becomes a peer in a
decentralized mesh. The full architecture is in [`DESIGN.md`](./DESIGN.md); this
roadmap turns it into ordered phases.

**Phases 0–6 of the original compiler track are done and run today** (reference
interpreter, Rust toolchain, IR, and a complete WASM backend). The guiding rule
stands: *never let an unbuilt feature block a buildable one* — and one new rule
joins it: ***design the security/type/network/graphics SPEC before the native
compiler*** (see Phase 1).

The five invariants every phase is judged against: **token density**, **meaning
in the manifest**, **errors as instructions**, **safe by construction**, and
**zero-install**. (Details in `DESIGN.md` §0.)

---

## Phase 0 — Reference semantics + WASM backend (DONE ✅)

The language runs today, two ways kept byte-identical by a conformance suite.

- [x] Two-state lexer, `$`-strings, `#` comments; parser → AST with first-class blocks
- [x] Stack VM: arithmetic, comparison, logic, stack ops, select/while/invoke
- [x] Token-dense stdlib, `.zerr` diagnostics, shadow manifest, `zown` CLI
- [x] Rust toolchain (`zownc`): lexer/parser/AST/IR/VM at full parity with the oracle
- [x] **WASM backend**: full v0.1 → `.wat` + binary `.wasm`, all 13 conformance
      cases run under `wasmtime` (see `WASM.md`)

> The Python implementation remains the **reference oracle**; the Rust toolchain
> is differentially tested against it.

---

## Phase 1 — Design freeze: SPEC v0.2 + oracle (the pivot)

**Before the native compiler**, fix the decisions that shape the ABI, type system,
and codegen — then validate them in the Python reference so the compiler is built
once, correctly. No new backend in this phase; semantics first.

- Capability types (zero authority by default; unforgeable, scoped, revocable).
- Full numeric model (`i8…ix u8…ux`, `float`, `b`, and design-only `iz`/`uz`/`dc`/`cx`; explicit
  overflow), SIMD/vector + tensor types, crypto types (`Key Sig Hash NodeID`).
- Network type primitives (`~` family), protocol-as-type compatibility.
- Native UI type primitives (typed layout tree; no HTML/CSS), GPU types.
- Pattern matching over the type system.
- Extended shadow-manifest schema (capabilities, security, telemetry, provenance).
- Security `.zerr` event extensions.

These are added to `SPEC.md` (Part II) and exercised in the Python oracle.

---

## Phase 2 — Safety core (the "unhackable" foundation)  🔄

Make the guarantees real, in the type checker and runtime.

- [x] Static checker (`zown check`) with a soundness contract (no false
      positives): name resolution, `??` arm shape, and **static capability flow**
      (a `rq` the checker can prove unsatisfiable is a compile-time `CAP_DENIED`)
      — PLAN M8a/M8b, goldens in `conformance/check/`.
- [ ] Result values consumed through `??`; the no-undefined-behavior catalogue
      with a conformance case per row (PLAN M8c).
- [ ] The Rust toolchain reaches v0.2 and ports the checker (PLAN M8d).

- Fat-pointer descriptors `[base | bounds | perms]` as the only reference kind.
- Compile-time null elimination: fallible ops yield `[ok? | data]`; `?` must
  consume it before the data is reachable.
- Ownership tokens `!` (move) / `&` (borrow), checked at parse/lowering time;
  linear stack-frame allocation; no GC.
- **No undefined behavior**: anything otherwise-undefined is a compile-time `.zerr`
  or a structured runtime trap.
- Capability flow enforced by the type system; secure-memory `Secret[T]`,
  constant-time ops, secure RNG, capability-level rate limiting.

Hard problems: borrow analysis on a stack language; bounds-check elision.

---

## Phase 3 — Native backend + performance

Compile the AST→IR to real targets, built with the full type system.

- **Cranelift** first (millisecond JIT for hot-swap + dev loop), **LLVM** later
  (release AOT, `-O`, bare metal). One IR feeds both — the Wasmtime architecture.
- `-o .exe`/ELF/Mach-O desktop; SIMD lowering; zero-copy I/O (`io_uring`/IOCP);
  **deterministic builds** (bit-identical output — content-hash verification needs it).
- Differential testing: native output matches the oracle.

Hard problems: stack→SSA lowering; calling convention for blocks/quotations.

---

## Phase 4 — Concurrency & dynamic fast lanes (DONE ✅)

`~`, `^`, and `|` are real. The data plane (lanes) runs before the control
plane (one compute job per tick). SPEC §22 is the contract.

- [x] Runtime queries hardware topology (`hw`, `ZOWN_CPUS`); **application-defined**
      real-time lane counts (`lc`). Slots come off the compute pool. Lanes that
      do not fit still run every tick, as fibers, so they are not skipped.
- [x] `^` queues a block on a work-stealing pool; `|` sends on the lane pipe
      (`pm` / `pn` pull). The observable schedule is the trace, not a wall clock.
- [x] `zown tune` and `zown tune N` print the grant and rebalance the same way
      `lc` does.

OS `SCHED_FIFO` is not part of the trace. A later host may pin threads, and it
must produce the same letters.

---

## Phase 5 — Batteries: the zero-install stdlib

One correct implementation of everything an app needs (see `DESIGN.md` §10).
The first slice is in: `mx`, `mn`, and `cl` are Zown (`zown/std/core.zn`),
and `zown test` / `zown fmt` run. SPEC §23 names the rest and does not stub it.

- [x] Host I/O (SPEC §23). `gd` `fr` `fw` are path-scoped disk. `ac` `ag` are
      the program's arguments. `en` looks up one variable. `tm` reads the clock.
      `ZOWN_TICK` and `ZOWN_WALL` pin the golden.
- [x] ChaCha20-Poly1305 (`ae` `ad`), Ed25519 (`es` `ev`), and X25519 (`xk`).
      BLAKE3 `h3` was already in. RFC test vectors. Keys are `buf`s.
- [ ] Collections and the rest of the numbers. Names are frozen in SPEC §23.
- [x] Developer toolchain, the part that can be honest today: `zown test`
      (goldens, plus `ts` / `pp`) and `zown fmt` (canonical whitespace).
      LSP, the doc generator, and a mesh REPL are still §23.
- [x] Begin the self-host migration: `std/core.zn` is loaded by both VMs.
      Host intrinsics remain for words the language cannot express (`ab`,
      strings, `sq`, `pw`, …).

---

## Phase 6 — ZownNet: the decentralized mesh

Replace the location-addressed internet with an identity/content-addressed peer
mesh (see `DESIGN.md` §3). The running slice is SPEC §24.

- [x] Content is addressed by **BLAKE3** (`h3`). `~p` / `~f` store and read a
      bundle. `zown mesh` moves one bundle between two IPv6 UDP peers on
      `[::1]`, which does not use the internet.
- [ ] Crypto identity is an Ed25519 key whose IPv6 address is
      `BLAKE3(pubkey)`. Today `ua` is an IPv6 ULA from a BLAKE3 node id, not a key.
- [ ] **ZownTransport** (QUIC-equivalent): encrypted-by-default, multiplexed,
      0-RTT, mutual auth.
- [ ] Multicast `ff02::5a77` is the announce address. This host does not deliver
      it. Kademlia and gossip are not started.
- [ ] NAT traversal, relays, and `~l` as a shared-memory fast path.

Hard problems still open: congestion control, DHT churn, relay incentives.

---

## Phase 7 — Native UI, ZownGPU, audio, input

Everything a frontend needs, native (see `DESIGN.md` §4–§5). The running slice
is SPEC §25: a typed tree and a PPM of its rectangles.

- [x] Layout tree with no markup: `co`, `tx`, `bt`, `rw`, `cm`, and `ly`.
      `zown draw` paints the rectangles. Contrast is marked `ok` or `low`.
- [ ] ZownGPU, shaders, text shaping, animation, input delivery, audio, PDF.
      Not started. The picture is not a font and not a GPU.

---

## Phase 8 — Embedded store + distribution

Drop the external DB; make the mesh the version-control system (see `DESIGN.md`
§6–§7).

- [x] In-memory graph and the query tokens `»` `«` `→` (SPEC §26). `gv` `ge`
      `gs` `gl` `gh` `sb` `so`. `zown keep` fsyncs a BLAKE3 bundle and rejects
      a changed one.
- [ ] Memory-mapped graph and columnar stores; lazy schema evolution; CRDTs.
- [ ] Signed module-version chain, transparency log, inclusion proofs.
- [ ] Semantic patch protocol, four-tier trust, signed bundles, hot-swap.
      Not started. The seal that runs is a hash, not a signature.

Hard problems: mmap crash-consistency; transparency-log scaling.

---

## Phase 9 — AI control plane + rolling execution

The "living software" + "AI manages every instance" layer (see `DESIGN.md` §8).

- [x] Logical log (SPEC §27): spans, notes, and every `.zerr`, tagged with a
      content hash, a version, and a trace id. `wn` / `ob` / `df` are the
      declared want. `rm` remembers a note for a fault code. `zown watch`
      prints the log.
- [ ] A fleet, traces that cross a network, hot-swap, semantic patches.
- [ ] On-device ML inference and federated updates.
      Not started. The note on a fault is not a patch.

Hard problems: safe live state migration; sandboxing remediation; trustworthy
local inference.

---

## Phase 10 — Self-hosting (the endgame)

- [x] The lexer is Zown (`zown/std/lex.zn`). `zown stage` checks it against
      the host token dump. The fixed point is the lexer's own source.
      `zownc native` runs that program too (SPEC §28).
- [x] The parser is Zown (`zown/std/parse.zn`). `zown stage parse` checks it
      against the host AST, including on its own source. A leaf keeps the raw
      lexeme. `zownc native` runs that program too.
- [x] The lowering is Zown (`zown/std/lower.zn`). `zown stage lower` prints
      the IR text `zownc ir` prints, including on the lowerer's own source.
      `zownc native` runs that program too.
- [x] The print-subset boot sector is emitted by Zown (`zown/std/emit.zn`).
      Stage-0 compiles that program. The image matches `zownc bare` and boots
      in QEMU. The sector is not itself a compiler. Stage-0 still emits every
      other executable.

---

## Phase 11 — Zown OS / bare metal

- [x] `zownc bare` writes a 512-byte real-mode boot sector (SPEC §29).
      String prints land in the VGA text buffer. `int 0x20` stores `0xB1`
      at `0x8000`. QEMU reads both back.
- [ ] Long mode, a hardware IRQ, and a microkernel.
      Not started. The sector halts after the one software interrupt.

The work after M17 is mapped as **M18–M32** in [`PLAN.md`](./PLAN.md). `buf`
(M18) through host I/O (M20) and crypto (M21) are done. Next is WASM v0.2
(M22). The byte-identical stage-2 compiler is M23.

---

## Cross-cutting principles

1. **Token density is a feature, not an accident** — audit every new feature; a
   symbol beats a word, the manifest carries the long name.
2. **The manifest is how we keep meaning** — and it is the built-in few-shot
   training signal that ships with every program.
3. **Errors are instructions** — structured, actionable `.zerr`, security included.
4. **Safe by construction** — zero authority by default, no undefined behavior.
5. **Zero-install** — if its absence forces an external download, it belongs in Zown.
6. **The reference VM is the oracle** — every backend is validated against it.
7. **Design before the compiler** — ABI-shaping decisions are frozen in the SPEC
   and proven in the oracle first.
