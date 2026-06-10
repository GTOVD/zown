# Zown Language Specification — v0.1 (Core)

Zown is an **AI-native, token-dense, stack-based** programming language. It is
designed to be written and read primarily by machines: control flow, stack
manipulation, and most operations are single ASCII characters, and the standard
library uses 1–2 character words. A program rarely needs identifiers at all.

This document defines the **v0.1 core** that the reference interpreter in this
repo actually implements (**Part I**, §1–§10). The longer-term vision (native
compilation, fast lanes, embedded graph DB, hot-swap, self-healing, and the full
sovereign-substrate architecture) is tracked in [`ROADMAP.md`](./ROADMAP.md) and
[`DESIGN.md`](./DESIGN.md). Where the original design notes contradicted
themselves, this spec is the authority and the contradictions are called out.

**Part II (§11–§18)** is the **planned v0.2 language surface** — the type
primitives (capabilities, full numerics, SIMD/tensor, network, crypto, UI/GPU) and
pattern matching that must be frozen in the spec and validated in the Python
oracle *before* the native compiler is built (`DESIGN.md` §1). Part II tokens are
**provisional**, pending a density audit; the design intent is stable even where a
specific symbol may change.

> File extension: `.zn` (we avoid `.z`, which collides with gzip's extension).

---

## 1. Execution model

Zown is **concatenative**: tokens are executed left to right against a single
**operand stack**. Literals push themselves; operators pop their inputs and push
their results. This is why variables are mostly unnecessary — intermediate values
live on the stack, costing zero tokens to name.

```
10 20 *        # push 10, push 20, multiply -> stack: [200]
```

### Values

| Type   | Example                | Notes                                        |
|--------|------------------------|----------------------------------------------|
| int    | `42`                   | arbitrary precision                          |
| float  | `3.14`                 | whole-valued results collapse back to int    |
| str    | `$hello$`              | bounded by `$ ... $`                          |
| block  | `[ ... ]`              | a first-class quotation (deferred code)       |

**Truthiness:** `0`, `0.0`, `""` (empty string), and the empty block are false;
everything else is true.

---

## 2. Lexical structure

The lexer is a two-state machine: **code mode** and **literal mode**.

- **Whitespace** (space, tab, CR, LF) separates tokens and is otherwise ignored.
  Symbols are self-delimiting, so `10 20*` and `10 20 *` lex identically.
- **Comments** start with `#` and run to end of line.
- **Strings**: a `$` flips into literal mode; bytes are swept verbatim until the
  next `$`. Escapes inside a string: `\$`, `\\`, `\n`, `\t`, `\r`, `\0`.
- **Numbers**: `[0-9]+` optionally followed by `.` + `[0-9]+`. A trailing `.`
  with no following digit is the **print** operator, not a decimal point — so
  `5.` means "push 5, print", while `5.5` is a float.
- **Identifiers**: `[A-Za-z][A-Za-z0-9]*`. (No `_`; `_` is the negate operator.)
- **Binding**: `:name` — a `:` immediately followed by an identifier.

---

## 3. Naming & binding

Zown keeps the brainstorm's `:name` / `name` / `@` triad, made fully consistent:

| Form     | Meaning                                                              |
|----------|----------------------------------------------------------------------|
| `:name`  | Pop the top value and bind `name` to it.                             |
| `name`   | Push the value bound to `name`. If unbound but a **builtin word**, run it. |
| `@`      | Pop a block and execute it.                                          |

```
[$Hi$.]:h    # bind the block to h
h@           # push h (the block), then invoke it  -> prints "Hi"
5:x x x +    # x=5; push x twice; add -> [10]
```

Resolution order for a bare identifier: **user binding first, then builtin word.**
A user binding therefore shadows a stdlib word of the same name.

---

## 4. Operators (single/double ASCII)

### Arithmetic
| Op  | Meaning           | Notes                                            |
|-----|-------------------|--------------------------------------------------|
| `+` | add / concatenate | if either operand is a string, concatenates      |
| `-` | subtract          | numbers only                                     |
| `*` | multiply / repeat | `str int *` repeats the string                   |
| `/` | divide            | error `DIV_ZERO` on `/0`                          |
| `%` | modulo            | error `DIV_ZERO` on `%0`                          |
| `_` | negate (unary)    | `5 _` -> `-5`                                     |

> **Resolved contradiction:** the notes used `*` for *both* multiply and "loop".
> In Zown, `*` is multiply; looping is the `;` word (see §6).

### Comparison & logic (push `1`/`0`)
| Op   | Meaning            |
|------|--------------------|
| `==` | equal              |
| `!=` | not equal          |
| `<`  | less than          |
| `>`  | greater than       |
| `<=` | less or equal      |
| `>=` | greater or equal   |
| `&&` | logical and        |
| `||` | logical or         |
| `!`  | logical not        |

> **Resolved contradiction:** `=` was listed as both "dup" and "equals". In Zown,
> `=` is **dup**; equality is `==`.

### Stack manipulation
| Op  | Meaning                                  |
|-----|------------------------------------------|
| `=` | dup — duplicate the top value            |
| `,` | drop — discard the top value             |
| `\` | swap — exchange the top two values       |
| `&` | over — copy the second value to the top  |

### I/O
| Op  | Meaning                                   |
|-----|-------------------------------------------|
| `.` | pop and print the value, plus a newline   |

---

## 5. Reserved symbols

These are intentionally **not** used in v0.1 core so they remain free for the
roadmap features they were sketched for:

| Symbol | Reserved for                                  |
|--------|-----------------------------------------------|
| `|`    | atomic pipe (lock-free inter-thread transfer) |
| `~`    | hardware/stream lanes (`~n` net, `~m` input)  |
| `^`    | fork / async spawn to the compute pool        |
| `( )`  | tuples / grouping                             |
| `{ }`  | inline control-flow bodies                    |
| `# ` (inside code mode is a comment)          |               |

A lone `|` is a lex error today (use `||` for or) precisely to keep it open.

---

## 6. Control flow

Control flow is built from **blocks + operators**.

### Conditional — select then invoke
`?` is a **selector**: it pops `else`, `then`, and a condition, and pushes the
chosen block. It does **not** run it — follow with `@`:

```
cond [then] [else] ? @
```

For an if-with-no-else, use an empty else block: `cond [then] [] ? @`.

```
x 1 == [$Yes$.] [$No$.] ? @
```

### Loop — while
`;` pops a **body** block and a **condition** block, then runs the body while the
condition evaluates truthy:

```
[cond] [body] ;
```

```
0:c [ c 3 < ] [ c . c 1 + :c ] ;   # prints 0,1,2
```

---

## 7. Standard library (token-dense words)

Builtins are auto-executing words. Each replaces a verbose legacy method. The
canonical list lives in [`zown/builtins.py`](../zown/builtins.py) (`WORDS`), which
also feeds the manifest generator.

| Word  | Alias     | Effect                                       |
|-------|-----------|----------------------------------------------|
| `ln`  | length    | length of a string (chars) or block (nodes)  |
| `tr`  | trim      | strip surrounding whitespace                 |
| `up`  | upper     | uppercase a string                           |
| `lo`  | lower     | lowercase a string                           |
| `rv`  | reverse   | reverse a string                             |
| `pr`  | print_raw | print with no trailing newline               |
| `ab`  | abs       | absolute value                               |
| `mx`  | max       | max of top two numbers                       |
| `mn`  | min       | min of top two numbers                       |
| `sq`  | sqrt      | square root                                  |
| `pw`  | pow       | `base exp pw` -> base**exp                   |
| `fl`  | floor     | floor                                        |
| `ce`  | ceil      | ceiling                                      |
| `rd`  | round     | round to nearest int                         |
| `s`   | to_str    | convert top value to its string form         |
| `n`   | to_num    | parse a string into a number                 |
| `dp`  | depth     | push the current stack depth                 |
| `rt`  | rot       | rotate top three (`a b c -> b c a`)          |
| `clr` | clear     | clear the whole stack                        |

---

## 8. Diagnostics (`.zerr` packets)

Errors are structured payloads, not prose. Every error carries a deterministic
recovery **code**, the source position, a **stack snapshot**, and a dense
**hint**, so an AI agent can self-heal deterministically. Run with `--zerr` to
emit JSON instead of the human form.

Recovery codes: `REPAIR_SYNTAX`, `STACK_UNDERFLOW`, `TYPE_MISMATCH`,
`NAME_UNRESOLVED`, `DIV_ZERO`, `BOUNDS`, `NOT_CALLABLE`, `UNSUPPORTED`.

```json
{
  "zerr": "0.1",
  "kind": "run",
  "code": "STACK_UNDERFLOW",
  "msg": "`+` needs a value but the stack is empty",
  "op": "+",
  "pos": { "line": 1, "col": 3, "offset": 2 },
  "stack": [],
  "hint": "push a value before this operator",
  "file": "bad.zn"
}
```

---

## 9. Shadow manifest (`<file>.zn.json`)

Raw Zown stays microscopic; the manifest is the architectural index that maps
each short name to an `alias`, a prose `desc`, and an `ai_hint` describing how to
modify it safely. `zown manifest file.zn` scans the file and merges discoveries
into the manifest **without** clobbering descriptions you've already written.

---

## 10. Grammar (EBNF, informal)

```
program  = { node } ;
node     = int | float | str | block | name | bind | op ;
block    = "[" { node } "]" ;
bind     = ":" ident ;
name     = ident ;
int      = digit { digit } ;
float    = digit { digit } "." digit { digit } ;
str      = "$" { char | escape } "$" ;
op       = "+" | "-" | "*" | "/" | "%" | "_"
         | "==" | "!=" | "<" | ">" | "<=" | ">=" | "&&" | "||" | "!"
         | "=" | "," | "\" | "&"
         | "." | "@" | "?" | ";" ;
ident    = letter { letter | digit } ;
```

---

# Part II — Planned language surface (v0.2 design)

Everything below is **design, not yet implemented**. It is specified here so the
native compiler (PLAN M9) is built once with the full type system in hand, and so
the Python oracle (PLAN M7) can validate the semantically-observable parts first.
**Tokens are provisional**, pending a density audit — the *semantics* and the
*shape* are what's being frozen.

Two principles carry over from Part I and govern every addition:

- **Density first.** A symbol beats a word; a 1–2 char word beats a long one. When
  this spec writes a long name (e.g. `net-send`), the real program uses a dense
  token and the **shadow manifest** carries the long name. See `MANIFEST.md` v2.
- **Safe by construction.** Types make illegal states unrepresentable;
  capabilities make unauthorized access untypeable.

## 11. Numeric model

The default integer remains **arbitrary precision** (Part I). Typed contexts may
pin an explicit width; overflow is **never silent** — it is a compile-time error
or an explicit `wrap`/`sat`/`chk` op.

| Class | Types |
|-------|-------|
| signed int | `i8 i16 i32 i64 i128` |
| unsigned int | `u8 u16 u32 u64 u128` |
| pointer-size | `usize isize` |
| float | `f32 f64` |
| exact | `dec` (decimal), `big` (bigint), `cx` (complex) |

#### Fixed-width arithmetic — **implemented (M7c-i, oracle)**

The width names above are **words that push a width tag**. A *policy word* then
reduces the value below it into that width, so overflow is always an explicit
choice rather than a silent surprise:

```
n width wr   # wrap   — two's-complement modulo 2**bits
n width st   # sat    — clamp to the width's [min..max]
n width ck   # check  — pass through, or raise OVERFLOW if it won't fit
```

```
300 u8 wr .      # 44     (300 mod 256)
200 _ i8 st .    # -128   (clamped to i8 min)
256 u8 ck        # zerr[OVERFLOW]: 256 does not fit u8 [0..255]
```

Policy words are integer-only (a fractional float is a `TYPE_MISMATCH`). `dec`,
`big` (the default int already is big), and `cx` remain design-only for now.

### Vector / SIMD types — **implemented (M7c-ii, oracle)**

Lane-typed values that will lower to native SIMD (SSE/AVX/NEON, chosen by target).
A constructor word evaluates a block to produce its lanes:

```
f4   = f32 x4      d2 = f64 x2
i4   = i32 x4      b16 = u8 x16
[ 1 2 3 4 ] i4               # build an i32x4
[ 1 2 3 4 ] i4 [ 10 20 30 40 ] i4 vadd    # i4(11 22 33 44)
[ 1 2 3 4 ] i4 vsum .        # 10   (horizontal sum -> scalar)
[ 7 8 9 10 ] i4 2 vat .      # 9    (lane 2)
```

Elementwise ops are word-form (`vadd vsub vmul`), **not** `v+`/`v*`: a Zown
identifier cannot contain an operator char, so `v+` would lex as `v` then `+`.
Ops require matching vector types (else `TYPE_MISMATCH`); integer lanes wrap to
their width, the same rule as scalar fixed-width ints.

> Oracle limitation: float lanes are stored as f64; the oracle does not model f32
> rounding. Lane count, type compatibility, and integer wrap are the observable
> semantics frozen here — exact f32 behavior is the native backend's job (M9).

### Tensor types

N-dimensional typed arrays over the same lane types, mapping to CPU or GPU memory;
the substrate for on-device ML inference (`DESIGN.md` §8). Design-only for now.

## 12. Capabilities & the security model

**Zero authority by default.** A program holds no capabilities until granted one.
Capabilities are unforgeable, scoped, revocable tokens tracked by the type checker
— you cannot fabricate or smuggle one (`DESIGN.md` §2).

Provisional dense form: a capability sigil **`` ` ``** introduces a 1-char cap
code; the manifest maps the code to its full name + scope.

| Code | Capability | Notes |
|------|-----------|-------|
| `` `s `` | net-send | scopable: `` `s rate:1000/s `` |
| `` `r `` | net-recv | |
| `` `d `` | disk | path-scoped at grant time |
| `` `g `` | gpu-compute | |
| `` `p `` | spawn-process | |
| `` `k `` | secret | gates `Secret[T]` |
| `` `c `` | clock | wall + monotonic |
| `` `x `` | sensor | sub-scoped: `sensor:gps`, camera/mic explicit |
| `` `e `` | secure-enclave | TPM/TrustZone/SGX |

A block's required capabilities live in its **manifest** entry (`caps`), not in the
source — the source stays dense:

```
[ ... ] :get        # manifest: get.caps = ["`s","`r"]
```

A call that uses a capability it was not granted fails to type (or traps with a
`CAP_DENIED` security `.zerr`, §18).

### Runtime mechanism (implemented in the oracle)

The capability *runtime* is live in the Python reference today (PLAN M7a). A
capability token is a first-class value pushed by the `` ` `` sigil; three words
move authority through the stack. A program starts with **zero** grants.

| Word | Form | Effect |
|------|------|--------|
| `gr` | `` `cap [body] gr `` | run `body` with `` `cap `` granted, then restore prior authority |
| `rq` | `` `cap rq `` | assert `` `cap `` is granted; raise `CAP_DENIED` (kind `sec`) if not |
| `hv` | `` `cap hv `` | push `1` if `` `cap `` is granted, else `0` (non-fatal probe) |

```
`s hv .                          # 0  -> no authority by default
`s [ `s rq $send-ok$ . ] gr      # grant net-send for this block only
`s hv .                          # 0  -> authority restored after the block
```

Grants nest and unwind independently. Privileged stdlib operations (net, disk,
GPU, …) will gate on `rq` for their capability, so "zero authority by default"
holds uniformly. Capability *scoping* (rate/path sub-scopes) and static
type-checked flow arrive with M8 (the safety core); today `rq` is a runtime check.

**Hygiene types:** `Secret[T]` (auto-zeroing, never logged/serialized/`.zerr`'d),
constant-time `ct-eq`, a single secure-RNG primitive (distinct type from the
fast non-secure PRNG), and capability-level rate limiting.

## 13. Crypto & identity types — *design-only (M14)*

> Frozen here, **not** in the oracle: faithful `Key`/`Sig`/`NodeID` need real
> Ed25519/X25519/BLAKE3, which a pure stack interpreter cannot exhibit honestly
> (a stand-in would mislead on a security-critical surface). These land with the
> network/security phase (M14); the *semantics* are fixed now so M8's type system
> can reserve them.

First-class types the type system understands (not raw byte arrays):

| Type | Meaning |
|------|---------|
| `Key` | Ed25519/X25519 key (public or, gated by `` `k ``, secret) |
| `Sig` | a signature |
| `Hash` | a BLAKE3 content hash (256-bit) |
| `NodeID` | 128-bit identity derived from a public key (= IPv6 address) |

Defaults are fixed and hard to misuse: BLAKE3 (hash), ChaCha20-Poly1305
(symmetric), Ed25519 (sign), X25519 (exchange). One algorithm per purpose.

## 14. Network primitives (`~` family) & protocol-as-type — *design-only (M11/M14)*

> The `~` family is reserved (Part I §5) but unimplemented: real streams, the
> mesh, and protocol-as-type checking need I/O and the type system, not the
> oracle. The token allocation and semantics are frozen here.

The reserved `~` family (Part I §5) becomes the network/lane surface. A connection
targets an **interface**, not an IP; the runtime resolves which peer provides it
(`DESIGN.md` §3). On-device (`~l`) and remote share one interface.

| Token | Meaning |
|-------|---------|
| `~o` | open a stream to a peer |
| `~s` | send on a stream (needs `` `s ``) |
| `~r` | recv from a stream (needs `` `r ``) |
| `~c` | connect to an interface (resolved via the mesh) |
| `~l` | local same-machine fast path (shared memory) |
| `~w` | world: tunnel/relay for legacy reach |
| `~n` `~m` | network / input real-time lanes (Part I reserved) |

**Protocols are types.** Two programs connect iff their protocol types are
structurally compatible (checked at compile time, else at capability-grant time):

```
# a protocol type (conceptual; dense form TBD)
proto Store [ get:[Hash]->[val?]  put:[Hash val]->[ok|err] ]
`s Store ~c :db        # connect to any peer that speaks Store
```

Content addresses are written `$zown:<hash>$`; mutable refs are
`$zown:<pubkey>$` (latest signed version). `~f` fetch / `~p` publish operate on them.

## 15. Native UI & GPU types — *design-only (M12)*

> Frozen here, not in the oracle: a retained scene graph, typed style values, and
> ZownGPU resources need a renderer. The type vocabulary is fixed so the manifest
> and type system can carry it.

UI is **typed data**, not markup (no HTML/CSS; `DESIGN.md` §4). A layout node is a
dense word + a style block + a children block. Style values are **typed** (`Color`
is a `Color`, not a string) so style errors are compile-time `.zerr`s.

```
bx [ layout:[row gap:8 align:center] pad:[16] bg:#1a1a2e ] [
  tx $Hello Zown$ [ size:24 weight:bold color:#fff ]
  bt $Launch$     [ bg:#7c3aed radius:8 role:button hint:$start$ ] :tap [go]
]
```

| Type | Meaning |
|------|---------|
| `LayoutNode` | a node in the UI tree (retained scene graph) |
| `Color` | typed rgba; carries contrast ratio (a11y warnings) |
| `Font` `Spacing` `Transition` | typed style values |
| `GpuBuffer` `GpuPipeline` `RenderPass` | ZownGPU resources |

Responsive layout is a **type constraint** (`row|col@600`), not a media query.
Accessibility (`role`/`label`/`hotkey`/`hint`) and i18n (translation keys, BiDi,
CLDR plurals) are node-type fields, carried in the manifest. Shaders are written in
a **Zown subset**, not GLSL/WGSL.

## 16. Pattern matching — **implemented (M7d, oracle)**

Structural matching over the type system, dispatching in one expression. The form
is `subject [arms] ??`, where `arms` is a block of `[pattern] [body]` pairs. The
first arm whose pattern matches runs, **with the matched subject left on the
stack** for the body to use; if none matches, a structured `NO_MATCH` is raised
(add a default arm). Patterns are *introspected* — `??` reads the pattern's AST,
it never executes it.

```
42 [ [int]   [ , $an int$ . ]      # type pattern: int/float/str/bool/block/cap/width/vec
     [str]   [ up . ]              # body sees the subject (here: uppercase it)
     [7]     [ , $seven$ . ]       # literal pattern (int/float/str)
     [_]     [ , $other$ . ] ] ??  # _ is the default arm
```

| Pattern | Matches |
|---------|---------|
| `[int]` … `[vec]` | a value of that type (`type_name`) |
| `[42]` `[$hi$]` | a value equal to the literal |
| `[_]` | anything (default) |

Malformed arms (odd count, a non-block arm, or an unrecognized pattern token)
raise `BAD_PATTERN`. **Destructuring** (`[a b]` over a tuple) stays design-only
until compound value types land — there is no tuple type in v0.2 yet.

## 17. Reserved-symbol allocation (updated)

Part I §5 reservations stand. Part II claims:

| Symbol | Now reserved for |
|--------|------------------|
| `` ` `` | capability sigil (§12) |
| `~x` words | network/lane primitives (§14) |
| `» « →` | embedded graph DB query pipeline (`DESIGN.md` §6) |
| `??` | pattern-match dispatch (§16, provisional) |

Unclaimed symbols remain lex errors so they stay open for future density wins.

## 18. Security diagnostics (`.zerr` extensions)

Security events use the same structured channel as Part I §8, so the AI control
plane and the self-healing loop act on them uniformly. New recovery codes:

`CAP_DENIED`, `AUTH_FAIL`, `INTEGRITY_FAIL`, `SIG_INVALID`, `RATE_LIMITED`,
`UB_TRAP` (a would-be-undefined operation trapped at runtime).

```json
{
  "zerr": "0.2",
  "kind": "sec",
  "code": "CAP_DENIED",
  "msg": "operation requires net-send; not granted",
  "op": "~s",
  "cap": "`s",
  "node": "zown:abc123...",
  "hint": "grant `s in the module manifest, or route via a capability holder"
}
```
