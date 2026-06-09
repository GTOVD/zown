# The Shadow Manifest

Zown source is deliberately microscopic — names are 1–2 characters and most logic
needs no names at all. The **shadow manifest** is the companion file that carries
the *meaning* the source omits, so an AI (or a human) can trace a cryptic symbol
back to a clear intent without inflating the code itself.

- One manifest per source file: `path/to/app.zn` → `path/to/app.zn.json`.
- Generated/refreshed with `zown manifest app.zn`.
- Regeneration **never clobbers** prose you've written: existing `alias`, `desc`,
  and `ai_hint` for a symbol are preserved; only newly discovered symbols are
  scaffolded.

## Format

```json
{
  "language": "Zown v0.1",
  "source": "app.zn",
  "symbols": {
    "n": {
      "type": "block",
      "alias": "net_poll",
      "desc": "High-priority network lane reading raw packets into a lock-free queue.",
      "ai_hint": "Must never block. Keep this block under ~15 tokens to protect the fast lane."
    },
    "p": {
      "type": "value",
      "alias": "player_count",
      "desc": "Current connected players.",
      "ai_hint": ""
    },
    "ln": {
      "type": "builtin",
      "alias": "length",
      "desc": "length of a string (chars) or block (nodes)",
      "ai_hint": ""
    }
  }
}
```

### Fields

| Field     | Meaning                                                                 |
|-----------|-------------------------------------------------------------------------|
| `type`    | `value`, `block`, or `builtin` (inferred at generation time).           |
| `alias`   | A human/AI-readable name for the symbol.                                |
| `desc`    | One-line description of what it holds or does.                          |
| `ai_hint` | Guidance for an agent modifying it (constraints, perf budget, dangers). |

## Workflow

1. Write dense Zown.
2. Run `zown manifest app.zn` to scaffold entries for every binding and every
   stdlib word the file uses.
3. Fill in `alias` / `desc` / `ai_hint` for your own bindings (builtins are
   documented automatically from the stdlib).
4. Feed the manifest alongside the source whenever an AI edits the codebase: the
   model reasons over the manifest but writes pure, dense Zown.

---

## Manifest v2 (planned)

The v0.1 manifest above carries *meaning*. As Zown grows into the sovereign
substrate ([`DESIGN.md`](./DESIGN.md)), the manifest also carries *authority*,
*provenance*, and *telemetry* — without inflating the source. This makes the
manifest the **single built-in few-shot training signal and audit record** that
ships with every program. Planned alongside `SPEC.md` Part II (PLAN M7).

### Per-symbol additions

```json
{
  "language": "Zown v0.2",
  "source": "app.zn",
  "symbols": {
    "get": {
      "type": "block",
      "alias": "http_get",
      "desc": "Fetch a content-addressed resource from the mesh.",
      "ai_hint": "Idempotent. Safe to retry. Keep under the net lane budget.",
      "caps": ["`s", "`r"],
      "sec":  { "ct": false, "secret": false },
      "tele": { "latency": true, "errors": ["INTEGRITY_FAIL", "AUTH_FAIL"] },
      "i18n": { "keys": ["err.fetch.timeout"] }
    }
  }
}
```

| Field  | Meaning |
|--------|---------|
| `caps` | capability codes this symbol requires (`SPEC.md` §12). Zero authority by default; this is the **only** place authority is declared. |
| `sec`  | security flags: `ct` (must be constant-time), `secret` (handles `Secret[T]`; never logged). |
| `tele` | telemetry the runtime emits for this symbol: latency tracking, the `.zerr` codes it can raise — consumed by the AI control plane (`DESIGN.md` §8). |
| `i18n` | translation keys referenced by this symbol (BiDi/CLDR resolution at runtime). |

### Module-level provenance block (planned)

Distribution and verified updates (`DESIGN.md` §7) live in a top-level `module`
block — the signed, content-addressed record that makes "this is the original,
uninfected version" a cryptographic fact:

```json
{
  "module": {
    "content": "zown:7f3a9c...",      // BLAKE3 of the module bundle
    "parent":  "zown:def456...",      // previous version (null = genesis)
    "author":  "zown:node:abc123...", // NodeID (derived from pubkey)
    "sig":     "ed25519:...",         // author's signature over this record
    "ver":     "1.2.4",
    "ts":      1769990400,
    "log":     "fix INTEGRITY_FAIL edge case in fetch",
    "deps":    [{ "name": "crypto", "content": "zown:..." }],
    "patch_of":"zown:...",            // if a SemanticPatch, what it patches
    "tier":    1                      // trust tier that applied it (SPEC §, DESIGN §7.4)
  }
}
```

A node verifies `content` (hash matches the bytes) and `sig` (author intended
exactly these bytes) — plus transparency-log inclusion — **before running the
module**. Verification is not optional; it is part of the runtime.

### Why this stays dense

None of these fields appear in the `.zn` source. The code remains microscopic;
authority, provenance, and telemetry are *out-of-band metadata* the runtime and
the AI consume. Regeneration still **never clobbers** prose or hand-tuned fields;
it only scaffolds newly discovered symbols and merges discovered `caps`.
