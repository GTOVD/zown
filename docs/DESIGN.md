# Zown Platform Design — the sovereign computing substrate

This is the **north-star architecture** for Zown beyond the v0.1 core. `SPEC.md`
defines the language that runs today; `ROADMAP.md`/`PLAN.md` sequence the work;
this document describes the *whole machine* we are building toward and the design
decisions that bind it together.

Zown is not "just a language." The goal is a single self-contained substrate
where everything a modern application needs — frontend, backend, API, database,
networking, load balancing, auto-scaling, security, and distribution — is native
to the language and its runtime, with **zero external libraries or frameworks**.
Every device that runs the Zown runtime becomes a peer: a compute node, a storage
node, a relay, and a host. Applications are content-addressed, cryptographically
verified, and distributed across the mesh with no central servers.

> The closest prior art is Plan 9's "everything is a coherent system" ambition,
> Erlang/BEAM's fault-tolerant distributed runtime, and the capability-secure
> microkernel (seL4) lineage — recombined for an **AI-native, token-dense** world.

---

## 0. The five invariants

Every feature below is judged against these. They are the soul of the project.

1. **Token density is a feature.** New surface syntax must pay its way: prefer a
   symbol over a word, a 1–2 char word over a long one. The shadow manifest, not
   verbose identifiers, carries human/AI-readable meaning. (When this doc shows a
   verbose name like `cap:net-send`, the real program uses a dense token and the
   manifest maps it back.)
2. **Meaning lives in the manifest.** Density without the manifest would blind the
   AI. Every symbol — and now every capability, version, and telemetry stream —
   resolves through the manifest. The manifest *is* the built-in few-shot training
   signal that ships with every program.
3. **Errors are instructions.** `.zerr` packets are structured, machine-actionable
   recovery data, never prose. Security violations extend the same channel.
4. **Safe by construction.** No undefined behavior, ever. Nothing has authority it
   was not explicitly granted. These are type-system and runtime guarantees, not
   conventions.
5. **Zero-install.** A Zown application needs nothing but the Zown runtime — no
   dependency install, no env setup, no config files. The manifest declares
   everything; the runtime fetches and verifies it from the mesh at first run.
   *Litmus test for any proposed feature: "Would its absence force a developer to
   download something external?" If yes, it belongs in Zown.*

---

## 1. The design-before-compiler principle

The most important sequencing decision: **the security model, capability system,
type system, network primitives, and graphics model go into the SPEC and the
Python reference oracle *before* the native compiler is built.**

These decisions shape the ABI, the type system, and codegen. Bolting security or
capabilities on after the compiler exists means breaking changes and exactly the
vulnerabilities we are trying to design out. The WASM backend (M6, done) was the
proving ground for lowering; the *native* compiler is built once, correctly, with
the full picture in hand.

```
Finalize SPEC v0.2 (types + capabilities + net + ui + crypto)
        │
Validate new semantics in the Python reference (the oracle)
        │
Build the native compiler with full type-system knowledge
        │
Build the batteries stdlib in Rust behind stable Zown FFI
        │
Port the stdlib from Rust to Zown as the language matures
        │
Self-host: the Zown compiler, written in Zown
```

---

## 2. Security — the "unhackable" core

Security and graphics are the two pillars Zown places at the *center* of the
design, not the edges. The security model combines four ideas that, together,
eliminate whole classes of attack. All four are present from the start.

### 2.1 Capabilities are the type system

Nothing has access to anything by default. A program begins with **zero**
authority and is granted only unforgeable **capability tokens** for exactly what
it needs: network send/recv, disk read/write (scoped to a path), GPU compute,
process spawn, clock, secrets, secure enclave. Capabilities flow through the
stack explicitly and are tracked by the type checker — you cannot fabricate or
smuggle one. This is the WASM Component Model's proven model, made native.

Dense form (provisional, pending density audit): capabilities are values created
from a capability sigil; the manifest maps each 1-char code to its full name and
scope. A block declares what it consumes in its manifest entry's `caps` field.

```
# conceptual: a function that needs net send + recv, declared in its manifest.
# the AI/human reads `caps:[net-send, net-recv]` from the manifest;
# the source stays dense.
[ ... ] :get        # manifest: get.caps = ["~s","~r"]
```

The runtime and the AI control plane can **audit exactly what every running
instance can touch** — capabilities are visible, scoped, and revocable.

### 2.2 No undefined behavior — ever

Every memory-corruption CVE in history traces to undefined behavior. Zown's
fat-pointer model (`[base | bounds | perms]`, the only reference kind) plus
compile-time null elimination (`?` must consume a `[ok? | data]` tuple before the
data is reachable) means: **if an operation could be undefined, it is either a
compile-time `.zerr` or a structured runtime trap.** There is no third option.
Buffer overflows, use-after-free, integer-overflow-by-accident, and null deref
cease to be reachable states. (See `ROADMAP` Phase "Safety core".)

### 2.3 Cryptographic identity for everything

Every module, process, and node has an **Ed25519 keypair**. Module-to-module and
node-to-node calls are signed. You cannot impersonate a module without its private
key, which kills confused-deputy and man-in-the-middle attacks inside the mesh.
Key rotation is a signed record (the old key signs a pointer to the new key) with
a grace period, handled by the control plane.

### 2.4 Minimal trusted computing base

The part that *must* be correct is kept as small as possible: capability
enforcement, memory isolation, cryptographic verification, and the scheduler.
Everything else — graphics, networking, the database — runs as a
capability-constrained module verified against that tiny core. This is the seL4
microkernel philosophy applied to a language runtime, and it is the eventual
shape of the Zown OS.

### 2.5 Security hygiene primitives (stdlib)

- **Secret[T]**: auto-zeroing memory; never appears in a stack trace, log, or
  `.zerr`. Backed by a dedicated `secret` capability class.
- **Constant-time ops**: `ct-eq` and friends the compiler guarantees not to
  optimize into timing side channels.
- **Secure RNG**: one primitive over the OS CSPRNG (`getrandom`/`BCryptGenRandom`).
  A separate, differently-typed fast PRNG exists for games/sims — you cannot mix
  them up.
- **Rate limiting as a capability**: `~s rate:1000/s` — enforced by the runtime at
  the capability boundary, not in app code, preventing abuse and accidental DDoS.
- **Hardware security**: when present, TPM / TrustZone / SGX seal private keys in
  the enclave (gated by a `secure-enclave` capability).
- **Security `.zerr` events**: capability violations, auth failures, and integrity
  failures emit structured security packets the control plane acts on.

---

## 3. ZownNet — the decentralized network, built in

Zown replaces the entire location-addressed, server-dependent internet model with
an **identity-addressed, content-addressed peer mesh**. Bootstrap on existing
transports via FFI, then replace each layer with native Zown.

```
Today's internet:  URL → DNS → IP → Server     (where something is)
ZownNet:           CryptoID / ContentHash → Peer (what something is)
```

### 3.1 Layer 1 — cryptographic identity → IPv6

A node generates an Ed25519 keypair on first launch. Its **IPv6 address is
derived from the public key** (`BLAKE3(pubkey)[..128]`, in the `fc00::/7` range).
Consequences: every device has a globally-unique address with no DHCP/registrar;
identity is verifiable by checking the address matches the key; moving networks
does not change identity. This is the Yggdrasil/cjdns approach, proven to scale.
**IPv6-only** is the deliberate choice (NAT disappears, address space is effectively
infinite).

### 3.2 Layer 2 — ZownTransport (a QUIC-equivalent, eventually native)

Over raw UDP (Rust FFI first, Zown syscalls later), Zown implements its own
transport inheriting QUIC's best ideas: **encrypted by default** (X25519 +
ChaCha20-Poly1305, no plaintext mode), multiplexed streams, 0-RTT reconnect to
known peers, and its own reliability/congestion control. mTLS-equivalent mutual
authentication on every connection, fed by the module identities of §2.3.

Dense network primitives live in the reserved `~` family (see SPEC Part II):
`~o` open, `~s` send, `~r` recv, `~c` connect-to-interface, `~l` local (same-machine
fast path), `~w` world (tunnel/relay).

### 3.3 Layer 3 — peer discovery (gossip mesh, no DNS)

- **Local**: nodes announce themselves over multicast IPv6; same-network peers
  add each other with zero config.
- **Global**: a **Kademlia DHT** (as in BitTorrent/IPFS) distributes routing in
  `O(log n)` hops.
- **Gossip**: liveness, hosted content, and offered services spread epidemically.

No root servers, no registrar. A new device boots, announces, joins the DHT, and
is immediately reachable by its cryptographic address.

### 3.4 Layer 4 — content addressing & distributed hosting

Content is addressed by **hash**, not location: any peer holding it can serve it.
Downloaded content is optionally re-hosted, so popular content/apps replicate
automatically (BitTorrent's seeding model for *everything*). A "site" or an app is
a signed, content-addressed bundle; share the hash and anyone with Zown can run it
— no app store, no CDN, no installation. Mutable references are an identity key +
the latest signed version.

### 3.5 Layer 5 — physical reality

IPv6 removes NAT for the long term. During the transition, hole-punching plus
volunteer **relay nodes** (which see only end-to-end-encrypted bytes) bridge
legacy IPv4. **Offline mesh**: device-to-device over WiFi-Direct/Bluetooth/LAN
with no internet at all — a building or neighborhood can run a full Zown mesh
disconnected from the world.

### 3.6 Same model on-device and across the world

Processes on one machine talk through the same typed interfaces, optimized to
shared memory / local fast paths (`~l`). From the application's view there is no
difference between a peer next door and one across the planet — the runtime picks
the transport. **Protocols are types**: two programs connect iff their protocol
types are structurally compatible (checked at compile or capability-grant time).

---

## 4. Native UI — no HTML, no CSS

UI is **data**: a typed declarative layout tree, the SwiftUI/Compose/Flutter model
expressed in Zown's stack form. There is no markup language, no string parsing, no
global cascading namespace. A node is self-contained: a box model, typed style
values, and children. Because the tree is structured data, the AI control plane
can read, diff, and hot-swap UI the same way it edits code.

```
# conceptual (dense tokens TBD); manifest carries the long names.
bx [ layout:[row gap:8 align:center] pad:[16] bg:#1a1a2e ] [
  tx $Hello Zown$ [ size:24 weight:bold color:#fff ]
  bt $Launch$    [ bg:#7c3aed radius:8 ] :tap [launch]
]
```

- **Typed style.** A `Color` is a `Color`, not a maybe-valid hex string. Style
  errors are compile-time `.zerr`s. A `Color` carries its contrast ratio so the
  compiler can warn on inaccessible combinations.
- **Responsive as a type constraint**, not media queries: `row|col@600` adapts at
  render time.
- **Animation/transition built in**, not a library.
- **Retained mode** (a live scene graph the AI can inspect/modify), not immediate
  mode (opaque per-frame draw calls).

### 4.1 ZownGPU — a WebGPU-equivalent

The right abstraction level is WebGPU (over Vulkan/Metal/DX12) — low enough for
game-quality performance, high enough to be portable and AI-inspectable. The
layout tree lowers to GPU command buffers. Bootstrap on `wgpu` via FFI; later
implement the ZownGPU API in Zown; ultimately write per-platform backends
(Vulkan/Metal/DX12) as Zown modules. **Shaders are written in a Zown subset**, not
GLSL/WGSL — the compiler emits GPU IR.

Text rendering (shaping/hinting) wraps a HarfBuzz-class implementation in the
stdlib — text is too hard to write from scratch and too essential to omit.

### 4.2 Accessibility and i18n are not optional

Accessibility semantics are built into the layout node types (role, label,
hotkey, hint); the layout engine emits an accessibility tree alongside the render
tree. The same data also lets the AI reason about UI structure. i18n is
first-class: bidirectional text, locale-aware number/date formatting, CLDR plural
rules, and a translation-key system that rides in the manifest.

---

## 5. Audio, input, and physical output

To rebuild *any* existing application, these must be native:

- **Audio**: playback + capture, a Zown signal-graph pipeline (mix/EQ/reverb/
  compress), spatial audio, MIDI, and a low-latency real-time lane (see fast
  lanes). Opus is the Zown-native codec.
- **Input**: unified pointer events (mouse/touch/stylus) with gestures, full
  Unicode keyboard + IME, gamepad with haptics, and sensors (GPS/accel/gyro/…),
  each gated by a capability (`sensor:gps`). Camera/microphone are
  explicit-grant-only.
- **Output**: printing (IPP, printer discovery over the mesh) and export of any
  layout tree to PNG/JPEG/SVG/**PDF** — every Zown app can screenshot itself
  because the scene graph serializes to any render target.
- **Lifecycle/power**: declared power profiles, foreground/background/suspended
  states with offline-first state persistence, and peer-to-peer **notifications**
  that wake an app through the mesh (no notification server).

---

## 6. Embedded storage — databases live on the machine

No external database. Two engines, both memory-mapped (on-disk layout == in-RAM
layout, zero parse on load, à la LMDB):

- A **graph store** (objects + typed edges) for most application data, with
  **lazy schema evolution** (migrate a record only when next touched — no global
  lock, no downtime).
- A **columnar store** for analytics/aggregations.

The query language is **stack-based and token-dense** (`»` open, `«` close, `→`
edge-follow), not SQL. **CRDT types** in the stdlib give conflict-free
distributed sync, so storage is local-first and mesh-replicated: any write can
optionally announce to the DHT, making the mesh a *mode* of every operation rather
than a separate API.

---

## 7. Distribution, versioning, and verified updates

The mesh *is* the version-control system. Everything below is cryptographically
verified so an infected or tampered artifact is rejected automatically — the same
hash+signature guarantee Git/IPFS/Cargo rely on, baked into the runtime so
verification is **not optional**: it happens before any module executes.

### 7.1 The guarantee

```
hash matches  → "this is exactly the published content, bit-for-bit"
sig verifies  → "this exact author intended to publish this exact content"
```

### 7.2 Signed module-version chain (Git's object model, signed at every step)

Each `ModuleVersion` is a signed record: content hash, parent hash, author NodeID,
signature, timestamp, semver, changelog, exact dependency hashes, required
capabilities. You can trace any module to its genesis; forks (two versions
claiming one parent) are a detectable, logged security event.

### 7.3 Transparency log (prevents targeted attacks)

A distributed, append-only, hash-chained log records **every** published update.
A node refuses to run an update that is not in the log (it can request an
inclusion proof) and broadcasts a security alert. This makes a key-compromise
"targeted update to one victim" impossible to hide — everything is publicly
auditable, and a compromised key can be revoked via gossip.

### 7.4 The AI patch protocol (novel)

The Phase-5 self-healing loop becomes a distributed patch system. Patches are
**semantic AST diffs**, not text diffs — they apply regardless of formatting, the
AI can reason about what they *do*, and the compiler verifies the patched result
compiles, matches a declared result hash, and passes tests before applying. A
`SemanticPatch` carries: target hash, declared result hash, kind
(security/bugfix/perf/feature), AI confidence, structured changes, required tests,
author, signature, and any capability delta.

**Trust hierarchy** bounds the blast radius of even a compromised AI:

| Tier | When it applies |
|------|-----------------|
| 1 — auto-apply silently | security fix, confidence ≥ 0.95, **no capability change**, tests pass, trusted author, verified in log |
| 2 — notify, apply after delay | bugfix, confidence ≥ 0.80, no capability change, user can cancel |
| 3 — explicit approval | any capability change, new deps, major bump, features, confidence < 0.80 |
| 4 — never auto-apply | changes to crypto/security code, the manifest schema, or the capability system itself |

Distribution: sign → publish to the transparency log → gossip to nodes running the
target → **each node independently** verifies log inclusion, signature, compiles
the patch, checks the result hash, runs tests, and only then hot-swaps. No central
authority blesses a patch; a malicious patch that fails tests on honest nodes is
auto-quarantined.

### 7.5 Offline sync — GitHub and USB are first-class

A **SyncBundle** is a portable, self-verifying snapshot: header (+ creator sig),
the manifest chain, all referenced content blobs, the relevant transparency
entries, and inclusion proofs. Any medium that carries bytes can sync Zown — USB,
email, QR, GitHub — because the receiver verifies everything independently; an
infected USB cannot poison a node. GitHub acts as a bridge (export the signed
manifest history as a Git repo; CI can publish signed releases to the DHT) so the
mesh history is auditable even without running a node.

### 7.6 Conflict resolution without a central authority

When histories diverge: higher-trust signature wins (author > AI > community);
same trust → earlier *signed* timestamp wins; security patches outrank feature
updates; capability expansions never auto-resolve. Both histories are preserved
(a merge points to both parents).

---

## 8. The AI control plane

For an AI to genuinely manage every instance in real time, the protocol is
designed in now, not discovered later. Two architecturally separate planes:

- **Data plane** (critical path): request routing, load balancing — every request
  flows through it.
- **Control plane** (off the critical path): scaling decisions and AI management —
  control signals only. The fast lanes (§9) keep them from contending, so AI
  management never injects latency into request handling.

**Telemetry** flows continuously in the shadow-manifest format: per-capability
resource use, request-latency percentiles per interface, error rates by `.zerr`
type, the live connection/dependency graph, and the capability audit log.

**Intent, not commands.** The AI writes *declarative desired state* ("10 healthy
`StorageNode` in us-east, p99 < 50ms"), never imperative ("start instance 7"). The
runtime moves toward desired state via hot-swap (code) and the mesh routing table
(traffic). Plus full **distributed tracing** (a trace ID per request, a span per
hop) and **structured metrics/logs** (typed, each tagged with the module version +
content hash that produced it, so the AI can correlate a log pattern with the
patch that fixed it).

**On-device ML inference** (ONNX, tensor types, quantized local LLMs) means the
managing AI runs *locally* — private, reliable, fast — not behind a cloud API.
Federated-learning hooks let nodes improve shared models by exchanging gradients,
never raw data.

The self-healing loop end to end: error → `.zerr` packet → AI reads packet + stack
snapshot + manifest → AI emits a `SemanticPatch` → compiled via the fast JIT →
verified + hot-swapped → telemetry confirms the fix → patch committed to the
module's manifest as a known remediation.

---

## 9. Performance architecture

- **Two compilers, one IR.** **Cranelift** for millisecond JIT (hot-swap, dev
  loop) and **LLVM** for release AOT — the Wasmtime architecture, proven.
- **No GC pauses.** The linear/ownership memory model (§2.2) gives deterministic
  frame timing for games and interrupt-safe timing for an OS, with per-context
  allocators (arena for requests, pool for game objects, bump for temp work).
- **SIMD as a first-class type** (`f32x4`, `u8x16`, …) lowering to SSE/AVX/NEON;
  the compiler picks the instruction set per target.
- **Zero-copy I/O** via `io_uring` (Linux) / IOCP (Windows): all I/O (network,
  disk, GPU submission) batched through completion rings, no kernel/userspace
  copies.
- **Linear memory layout** for the DB and scene graph (mmap, cache-line aligned,
  zero parse on load).
- **Profile-guided hot-swap**: the runtime profiles continuously; the AI triggers
  recompiles with better inlining/vectorization for hot paths and size-optimization
  for cold ones, live.
- **Fast lanes** (the `~ ^ |` reserved family): application-defined real-time
  lanes (audio, input, network) pinned off the compute pool, with `^` fork to a
  work-stealing pool and `|` lock-free pipes.

---

## 10. The batteries — one correct implementation of everything

The stdlib provides *one correct, well-chosen* implementation of each capability —
not the most configurable, the **correct** one, hard to misuse. Selected defaults:

| Area | Zown's choice |
|------|---------------|
| Hashing | BLAKE3 (parallel, fast) |
| Symmetric crypto | ChaCha20-Poly1305 (constant-time, no AES-NI dependence) |
| Signatures | Ed25519 · key exchange X25519 (no parameters to get wrong) |
| Serialization | Zown-native dense format (= the manifest format) + JSON/MsgPack interop |
| Database | embedded graph + columnar store; stack query language (no SQL) |
| HTTP | HTTP/3 over QUIC native; HTTP/1.1 legacy only; typed RPC over QUIC |
| Media | Opus (audio), AV1 (video) — open, patent-clear |
| Numbers | full width set `i8…i128 u8…u128 f32 f64 usize isize decimal bigint complex`; overflow is explicit, never silent |
| Collections | `List Map Set Queue RingBuf BTree Trie BloomFilter PriorityQ BitVec` |
| Strings | UTF-8 only; Unicode normalization is a named op, never silent |

> No MD5/SHA-1/RSA/AES-CBC. One algorithm per purpose.

### 10.1 Developer toolchain (ships with the language)

- **Testing** is a first-class construct (`zown test`), incl. property-based
  testing; results are structured so the control plane can analyze failure
  patterns across the mesh.
- **Deterministic builds**: same source + deps ⇒ bit-identical output, always.
  This is non-negotiable — content-hash verification depends on it.
- **LSP** (autocomplete/jump/rename/inline `.zerr`) fed by the shadow manifest.
- **Canonical formatter** (one style, no config) so AI-generated and human-written
  Zown are indistinguishable.
- **Doc generator** reading the manifest (no separate doc language).
- **Mesh-aware REPL** that can attach to and inspect a live remote node.

### 10.2 Runtime fundamentals

Filesystem (capability-scoped to paths), wall + monotonic clocks and timers
(capabilities), env/args/config, **secret injection as its own capability class**
(never logged/serialized), signal handling + graceful shutdown, and subprocess
management (for interop during the rebuild-everything transition).

---

## 11. Bootstrapping order (pragmatic)

Use existing implementations as scaffolding; replace each layer with native Zown
as the language matures. Nothing is locked to QUIC/WebGPU/wgpu forever — they are
reference implementations to learn from and supersede.

```
SPEC v0.2          define every type primitive + module interface (costs only thinking)
   │
Python oracle      validate UI/network/capability semantics before any native code
   │
Native compiler    Cranelift/LLVM with the full type system; SIMD; deterministic
   │
Rust stdlib        ZownTransport, DHT, gossip, content addressing, UI engine,
   │                ZownGPU(wgpu), crypto, DB — behind stable Zown FFI
   │
Zown stdlib        port each Rust module to Zown; same conformance tests
   │
Self-hosting       the Zown compiler written in Zown (fixed-point: stage1 == stage2)
   │
Zown OS            microkernel in Zown: hardware abstraction + capability
                   enforcement + isolation; everything else is Zown modules
```

### 11.1 What to rebuild first (once the compiler exists)

1. The tools to build everything else: editor, build system, manifest-aware
   package manager.
2. A small web server + HTTP/3 client (validates the network stack).
3. SQLite-class embedded DB functionality (validates the storage engine).
4. A browser *engine* that renders to Zown UI and runs Zown instead of JS — a
   WASM-hosted runtime first, a standalone browser much later (a full standalone
   browser is a multi-decade effort; do it after self-hosting).
5. Increasingly complex applications, each a product *and* a completeness test.

The goal is not feature-parity overnight; it is to demonstrate Zown expresses the
same programs with **less code, more speed, and stronger security guarantees**.

---

## 12. What this looks like done

A single binary. No install, no configuration. Boot any device with the Zown
runtime: it generates its cryptographic identity, announces to the local mesh,
joins the global DHT, and can immediately run any Zown application shared as a
content hash — verified bit-for-bit against its signed manifest. The device is at
once a compute node, a storage node, a relay, and a host; it contributes to the
mesh by existing. No servers anywhere. An AI manages the whole fleet by declaring
intent and reading telemetry, patching bugs in real time and broadcasting verified
fixes across the mesh. That device is what the SPEC we write today is the blueprint
for.
