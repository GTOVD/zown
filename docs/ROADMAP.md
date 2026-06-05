# Zown Roadmap

The design conversation laid out a complete vision: a hyper-dense, AI-native
language that compiles to bare metal and WASM, with built-in concurrency "fast
lanes", an embedded graph database, zero-downtime hot-swap, self-healing errors,
and a containerless communication mesh.

That is a multi-year systems project. This roadmap turns it into ordered phases.
**Phase 0 is done and runs today** (this repo). Each later phase lists what it
adds and the hard problems it must solve. The guiding rule: *never let an
unbuilt feature block a buildable one.*

---

## Phase 0 — Reference semantics (DONE ✅)

A working language you can run right now, defining the source of truth for
syntax and semantics before any native backend exists.

- [x] Two-state lexer (code/literal), `$`-bounded strings, `#` comments
- [x] Parser → nested AST with first-class blocks
- [x] Stack VM: arithmetic, comparison, logic, stack ops, select/while/invoke
- [x] Token-dense stdlib (1–2 char words)
- [x] Structured `.zerr` diagnostics (recovery code + stack snapshot + hint)
- [x] Shadow manifest generator (`<file>.zn.json`)
- [x] `zown` CLI: run / check / ast / manifest / repl
- [x] Tests + example programs (hello, fib, fizzbuzz)

> Implementation language: Python. It is the fastest path to nail down semantics
> and gives us a **reference oracle** to differentially test the native compiler
> against later.

---

## Phase 1 — Type & memory model

Make safety guarantees real before generating native code.

- Fat-pointer descriptors `[base | bounds | perms]` as the only reference kind.
- Compile-time **null elimination**: fallible ops push a conditional tuple
  `[ok? | data]`; the `?` operator must consume it before the data is touchable.
- Symbolic ownership: `!` consume (move), `&` borrow — checked at parse time.
- Linear stack-frame allocation: dropping a descriptor reclaims memory; no GC.

Hard problems: borrow analysis on a stack language; bounds-check elision.

---

## Phase 2 — Native backend (the big one)

Compile the Phase-0 AST to real targets. Build the frontend once, emit two IRs.

- `zown build x.zn -o x.wasm` → `wasm32-wasi` (browser / edge / sandbox).
- `zown build x.zn -o x.exe`  → LLVM IR → native (desktop).
- `zown build x.zn -o x.bin --target=bare` → `x86_64-unknown-none` (kernels).
- Differential testing: native output must match the Phase-0 interpreter.

Likely re-implemented in **Rust** (LLVM + Cranelift/Wasmtime ecosystem). The
Python VM stays as the spec oracle and a dev-loop fast path.

Hard problems: stack-machine → SSA lowering; string/data-segment layout;
calling convention for blocks/quotations.

---

## Phase 3 — Concurrency & dynamic fast lanes

Make the `~ ^ |` reserved symbols real.

- Runtime queries hardware topology (N logical cores) at init.
- **Application-defined** lane counts (not a single fixed fast lane): code/engineer
  declares how many real-time lanes it wants; the scheduler honors them if cores
  exist, else time-slices (cooperative fibers) and degrades to `^` async.
- `~n` network lane, `~m` input lane, pinned off the compute pool.
- `^` fork to a work-stealing compute pool; `|` lock-free atomic pipe.
- `zown tune <app> --steal-thread=<svc>` to rebalance live.

Hard problems: portable topology detection; WASM threads/atomics; priority
scheduling without OS-specific code.

---

## Phase 4 — Embedded storage engine

Drop the external DB dependency.

- Memory-mapped object graph: on-disk layout == in-RAM layout (zero parse on load).
- Token-dense query pipeline using the stack model: `»` open, `«` close,
  `→` edge-follow.
- **Lazy schema evolution**: version-tag structures; migrate a record only when
  it is next touched (no global lock, no downtime).

Hard problems: durability/crash-consistency of mmap writes; index structures.

---

## Phase 5 — Rolling execution & self-healing loop

The "living software" layer.

- Zero-downtime hot-swap: compile new module on a spare core, hand off state via
  an atomic pipe, redirect the execution vector, retire the old thread.
- Autonomous remediation: on `.zerr`, try a deterministic terminal fix first;
  escalate to the on-machine AI with the packet + manifest only if reasoning is
  needed; then issue the recompile/hot-swap command.
- Native test framework: AI-authored single-token unit tests beside the code.

Hard problems: safe live state migration; sandboxing the remediation subshell.

---

## Phase 6 — Communication mesh

Collapse containers + networking into one layer.

- WASI Component Model: apps link by matching import/export interfaces in memory
  (`~l` local, zero-copy) instead of opening local sockets.
- Built-in zero-config public tunneling (`~w`): outbound QUIC stream to an edge
  gateway, no router/firewall config.
- Autonomic control-plane API for an OS-level AI to poll health and rebalance.

Hard problems: capability-based security across the mesh; the edge gateway.

---

## Cross-cutting principles

1. **Token density is a feature, not an accident** — every keyword that becomes a
   symbol is a permanent win for context-window fit. Audit new features for it.
2. **The manifest is how we keep meaning** — density without the shadow manifest
   would blind the AI. They evolve together.
3. **Errors are instructions** — keep diagnostics structured and actionable.
4. **The reference VM is the oracle** — every backend is validated against it.
