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

- [ ] Crypto suite (BLAKE3, ChaCha20-Poly1305, Ed25519, X25519), collections, complete
      numerics, UTF-8 strings, time/clocks, filesystem, env/secrets. Names are
      frozen in SPEC §23; none of those words exist yet.
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

Everything a frontend needs, native (see `DESIGN.md` §4–§5).

- Typed declarative **layout tree** (no HTML/CSS), retained scene graph, fl/grid
  layout, typed style, responsive-as-constraint, animation.
- **ZownGPU** (WebGPU-equivalent; wgpu via FFI → native Vulkan/Metal/DX12 later);
  shaders in a Zown subset; text shaping in stdlib.
- Audio signal-graph + low-latency lane; unified input + sensors (capability-gated);
  **accessibility** and **i18n** built into the node types; PDF/PNG export; print.

---

## Phase 8 — Embedded store + distribution

Drop the external DB; make the mesh the version-control system (see `DESIGN.md`
§6–§7).

- Memory-mapped **graph + columnar** stores; lazy schema evolution; CRDTs for sync.
- Token-dense query pipeline (`» « →`).
- Signed **module-version chain**, distributed **transparency log**, **semantic
  patch protocol** with the four-tier trust hierarchy, signed **SyncBundles**
  (USB/GitHub), conflict resolution without a central authority.

Hard problems: mmap crash-consistency; transparency-log scaling.

---

## Phase 9 — AI control plane + rolling execution

The "living software" + "AI manages every instance" layer (see `DESIGN.md` §8).

- Continuous **telemetry** in the manifest format; distributed tracing; structured
  metrics/logs tagged with module version + content hash.
- **Declarative intent** API (desired state, never imperative commands).
- Zero-downtime **hot-swap**; the **self-healing loop** (`.zerr` → AI → semantic
  patch → verify → swap → confirm → record); broadcast verified fixes across the mesh.
- **On-device ML inference** (ONNX, tensors, quantized local LLMs) so the managing
  AI runs locally; federated-learning hooks.

Hard problems: safe live state migration; sandboxing remediation; trustworthy
local inference.

---

## Phase 10 — Self-hosting (the endgame)

The Zown compiler written in Zown. Bootstrap chain: Rust `zownc` (stage-0) →
Zown-in-Zown compiled by stage-0 (stage-1) → stage-1 recompiles itself (stage-2);
**fixed point** when `stage1 == stage2` byte-for-byte. The conformance suite
guards the whole chain (a "trusting trust" bootstrap).

---

## Phase 11 — Zown OS / bare metal

Prove "build anything," including kernels: `--target=bare`
(`x86_64-unknown-none`), MMIO + a bootable image, a minimal interrupt handler, and
ultimately a **microkernel in Zown** whose only trusted core is hardware
abstraction + capability enforcement + isolation — everything else a Zown module.

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
