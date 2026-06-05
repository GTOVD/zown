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
