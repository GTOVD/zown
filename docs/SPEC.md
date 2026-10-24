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

**Part II (§11–§20)** is the **v0.2 language surface** — the type primitives
(capabilities, full numerics, SIMD/tensor, network, crypto, UI/GPU), pattern
matching and result values, the no-undefined-behavior catalogue, and linear
bindings — frozen in
the spec and validated in the Python oracle *before* the native compiler is
built (`DESIGN.md` §1). Each section is marked **implemented** (oracle and/or
checker) or **design-only** (with its target milestone). Part II tokens are
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
| int    | `42`                   | **checked 64-bit** signed integer (`-2^63 .. 2^63-1`); a result outside the range is an `OVERFLOW` error, never a silent wrap |
| big    | `42 b` · `$digits$ b`  | arbitrary precision, exact; the explicit opt-in (§11) |
| float  | `3.14`                 | IEEE-754 binary64; whole-valued results that fit `int` collapse back to int |
| str    | `$hello$`              | bounded by `$ ... $`                          |
| block  | `[ ... ]`              | a first-class quotation (deferred code)       |

> **Changed in v0.2 (decision D1, 2026-09-07):** v0.1 made `int` arbitrary
> precision. Both native backends already used 64-bit integers and the
> "overflow is never silent" rule of §11 covers the difference, so the default is
> now a *checked* 64-bit int and `big` is the explicit unbounded type. No program
> in the conformance corpus observed the difference.

**Truthiness:** `0`, `0 b`, `0.0`, `""` (empty string), and the empty block are
false; everything else is true.

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
  `5.` means "push 5, print", while `5.5` is a float. An integer literal must
  fit `int` (`≤ 9223372036854775807`; literals are unsigned — `_` negates at run
  time), else it is a lex-time `OVERFLOW`; write larger values as a digit string
  and convert: `$99999999999999999999$ b`.
- **Identifiers**: `[A-Za-z][A-Za-z0-9]*`. (No `_`; `_` is the negate operator.)
- **Binding**: `:name` — a `:` immediately followed by an identifier.
- **Move**: `'name` — an apostrophe immediately followed by an identifier (v0.2,
  §20). **Capability**: `` `name `` (v0.2, §12).

---

## 3. Naming & binding

Zown keeps the brainstorm's `:name` / `name` / `@` triad, made fully consistent:

| Form     | Meaning                                                              |
|----------|----------------------------------------------------------------------|
| `:name`  | Pop the top value and bind `name` to it.                             |
| `name`   | Push the value bound to `name` (a **borrow**: the binding stays). If unbound but a **builtin word**, run it. |
| `'name`  | **Move** (v0.2, §20): push the value bound to `name` and **unbind** it. `NAME_UNRESOLVED` if there is no live binding (builtins cannot be moved). |
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
| `n`   | to_num    | parse a string into a number (`OVERFLOW` beyond `int`) |
| `b`   | big       | convert an int / whole float / digit string to a `big` (§11) |
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
`NAME_UNRESOLVED`, `DIV_ZERO`, `BOUNDS`, `NOT_CALLABLE`, `UNSUPPORTED`
(v0.1), plus `OVERFLOW`, `NO_MATCH`, `BAD_PATTERN` and the security codes of
§18 (v0.2). `kind` is `lex`, `parse`, `run`, `sec` (security), or `check`
(a static finding from the checker, §12/§16 — emitted *before* execution).

**Check-before-run.** `zown run` is gated by the static checker: a program with
any static finding is **rejected before a single node executes**, the findings
are reported as `check` packets (a JSON array under `--zerr`), and the process
exits with status **2**. Status 1 is a lex/parse/runtime `.zerr`; 0 is success.
The distinct status lets a pipeline route a rejected program back for rework
rather than treating it as a crash. `--unchecked` bypasses the gate for
debugging only. (The reference VM API, `VM.run_src`, is the *runtime* oracle and
is not gated; the conformance runners exercise it directly.)

```json
{
  "zerr": "0.2",
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
node     = int | float | str | block | name | bind | move | cap | op ;
block    = "[" { node } "]" ;
bind     = ":" ident ;
move     = "'" ident ;          (* v0.2, §20 *)
cap      = "`" ident ;          (* v0.2, §12 *)
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

Three scalar kinds, one rule: **overflow is never silent.** The Rust VM
implements this section, including `b`, width policy, and vectors
(M8d-ii/iii). WASM `int` arithmetic still wraps; that is M8d-iv.

| Kind | Representation | Overflow behavior |
|------|----------------|-------------------|
| `int` (default) | checked 64-bit signed | a result outside `[-2^63, 2^63-1]` is a run-time `OVERFLOW` (op `+ - * _ pw ab fl ce rd n`); a literal outside it is a lex-time `OVERFLOW` |
| `big` | arbitrary precision | cannot overflow; exact |
| `float` | IEEE-754 binary64 | overflows to `±inf` (a value); `nan` is a value; converting a non-finite float to an integer traps (§19) |

**Promotion.** `int ∘ int → int` (checked). Any `big` operand makes the result
`big` (exact). Any `float` operand makes the result `float` (a `big` too large
for a float is `OVERFLOW`). `/` keeps an exact integral quotient integral (`10 b
5 /` → `2 b`, `10 5 /` → `2`) and yields a float otherwise. Comparisons and
literal `??` patterns compare by value across the three kinds (`5 b 5 ==` → `1`);
type patterns do not (`[int]` does not match a big).

**`b` — the big constructor — implemented (M8c-i, oracle).** `v b` converts an
int, a whole float, a big (no-op), or a **string of digits** — which is how a
literal beyond the int range is written: `$99999999999999999999$ b`. Whole floats
print as integers only while they fit `int`; larger whole floats keep float
notation (`1e+30`); `inf`, `-inf`, `nan` print as such and `n` parses them.

| Class | Types |
|-------|-------|
| signed int | `i8 i16 i32 i64 i128` |
| unsigned int | `u8 u16 u32 u64 u128` |
| pointer-size | `usize isize` |
| float | `f32 f64` |
| exact | `dec` (decimal), `big` (arbitrary precision — **implemented**, the `b` word), `cx` (complex) |

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

Policy words are integer-only (a fractional float is a `TYPE_MISMATCH`); a `big`
input is reduced like any integer. **The result's type is decided by the width,
not the value:** widths whose entire range fits `int` (`i8…i64`, `u8…u32`) yield
an `int`; the others (`u64`, `i128`, `u128`) yield a `big` — so `5 u64 ck` is
`5 b`, and the type is static. `dec` and `cx` remain design-only.

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
holds uniformly. Capability *scoping* (rate/path sub-scopes) arrives with the
capability type work in M8c+; `rq` remains the runtime check, and the static
checker below rejects the failures it can prove before the program runs.

### Static capability flow — **implemented (M8b, checker)**

`zown check` reports a `` `c rq `` as a compile-time `CAP_DENIED` (kind `check`,
`op: "rq"`, `cap: "c"` — the same packet shape as the runtime error) whenever it
can **prove** the capability is not granted at that point. The checker's contract
is *if this node executes, it definitely faults*: it never reasons about whether
a node is reached (dead branches are still checked), and it never reports a
`rq` that could succeed on some execution. Zero false positives is the invariant
every runnable program in the conformance corpus guards.

Because a grant has **dynamic extent** — `gr` holds the capability only while it
invokes its block — flow is not lexical scope. Two facts are provable:

1. **Exact authority.** Top-level code runs with exactly **zero** grants. A block
   literal that is *anchored* — consumed inline by `@`, `;`, `? @`, `??`, a SIMD
   constructor (`f4 d2 i4 b16`), or `gr` — runs with exactly the enclosing
   authority, plus `c` for the literal form `` `c [ ... ] gr ``. Inside such
   blocks the granted set is still exact, so `c ∉ set` is a definite denial.
2. **Never granted.** Capability values are created only by `` ` `` literals, and
   `gr` needs one on the stack. A literal immediately consumed by `rq`/`hv` can
   never reach `gr`; if no *free* `` `c `` literal exists anywhere in the program,
   `c` can never be granted and every `` `c rq `` is denied — wherever it sits.

Anything else makes authority *unknown* and only rule 2 applies inside: a block
bound to a name, left on the stack, selected by `?` without an immediate `@`, or
granted a capability that arrived through a name. A user binding named `rq`,
`gr`, or `hv` shadows the builtin and is honored (a shadowed `gr` means nothing
can ever be granted).

```
`s rq                              # check: CAP_DENIED — top level holds nothing
`r [ `s rq ] gr                    # check: CAP_DENIED — authority is exactly {`r}
`s [ `s rq ] gr  `s rq             # check: CAP_DENIED on the trailing rq only
[ `s rq ] :send   `s [ send @ ] gr # clean — dynamic extent; send runs under `s
[ `s rq ] :send   `r [ send @ ] gr # check: CAP_DENIED — `s is never granted anywhere
```

Goldens: `conformance/check/` (`python3 conformance/check.py`).

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
42 [ [int]   [ , $an int$ . ]      # type pattern: int/big/float/str/bool/block/cap/width/vec
     [str]   [ up . ]              # body sees the subject (here: uppercase it)
     [7]     [ , $seven$ . ]       # literal pattern (int/float/str), compared by value
     [_]     [ , $other$ . ] ] ??  # _ is the default arm
```

| Pattern | Matches |
|---------|---------|
| `[int]` `[big]` … `[vec]` | a value of that type (`type_name`) |
| `[42]` `[$hi$]` | a value equal to the literal (`5 b` matches `[5]`) |
| `[_]` | anything (default) |

Malformed arms (odd count, a non-block arm, or an unrecognized pattern token)
raise `BAD_PATTERN`. **Destructuring** (`[a b]` over a tuple) stays design-only
until compound value types land — there is no tuple type in v0.2 yet.

### 16.1 Result values — **implemented (M8c-ii, oracle + checker)**

A **result** is the in-language form of a `.zerr`: `ok(value)` or `err(code)`.
It is how "errors are instructions" reaches the program itself, and it is the
concrete form of `DESIGN.md` §2.2's "fallible operations yield `[ok? | data]` and
the data is unreachable until the failure case is consumed" (decision D3: the
consumer is `??`, since `?` is select).

```
[ body ] at            # run body; ok(its single value) or err(the .zerr code it raised)
```

- `at` runs the body on an **isolated stack** (like the SIMD constructors) so its
  stack effect is always `( blk -- res )` on both paths — the ok and err shapes
  are identical, which keeps stack programs predictable. Pass inputs through
  bindings (`5 :x [ x 2 / ] at`). The body must leave exactly one value, else
  `BOUNDS`. Output, bindings, and grants the body performed are not undone.
- Any `.zerr` raised inside — including security packets (`CAP_DENIED`) and
  runtime traps (`OVERFLOW`, `UB_TRAP`, `STACK_OVERFLOW`) — becomes `err(code)`.
- **The payload is reachable only through `??`.** `[ok]` matches an ok result and
  runs its body with the *unwrapped value* on the stack; `[err]` matches an err
  result with the *code string*; `[res]` is the ordinary type pattern (no
  unwrap). To every other operator a result is opaque (`TYPE_MISMATCH`); it
  prints as `ok(5)` / `err(DIV_ZERO)`; its truthiness is its ok-ness, so
  `[ … ] at [then] [else] ? @` branches without exposing the payload.
- **Exhaustiveness is a static rule.** For the literal shape `[ … ] at [ arms ]
  ??` the checker requires the arms to cover both `ok` and `err` — via
  `[ok]`+`[err]`, `[res]`, or a default `[_]` — else a static `NO_MATCH` and the
  program is rejected before it runs (§8). At run time an unhandled err is
  `NO_MATCH` as well.

```
5 :x 0 :y
[ x y / ] at [ [ok]  [ 2 * . ]
               [err] [ $div failed: $ \ + . ] ] ??    # div failed: DIV_ZERO
[ `s rq 1 ] at ! [ $no net$ . ] [ ] ? @              # probe a capability without faulting
```

Inside an `at` body a fault is the *intended* err path, so the checker does not
report capability-flow denials there; well-formedness findings (unresolved names,
malformed arms, exhaustiveness) are still reported — no execution makes them right.

## 17. Reserved-symbol allocation (updated)

Part I §5 reservations stand. Part II claims:

| Symbol | Now reserved for |
|--------|------------------|
| `` ` `` | capability sigil (§12) |
| `'` | move sigil (§20) — **implemented** |
| `"` | reserved for a future *mutable reference* sigil once heap values exist (§20) |
| `~x` words | network/lane primitives (§14) |
| `» « →` | embedded graph DB query pipeline (`DESIGN.md` §6) |
| `??` | pattern-match dispatch (§16, provisional) |

Unclaimed symbols remain lex errors so they stay open for future density wins.

## 18. Security diagnostics (`.zerr` extensions)

Security events use the same structured channel as Part I §8, so the AI control
plane and the self-healing loop act on them uniformly. New recovery codes:

`CAP_DENIED`, `AUTH_FAIL`, `INTEGRITY_FAIL`, `SIG_INVALID`, `RATE_LIMITED`,
`UB_TRAP` (an operation whose result would be undefined or unrepresentable —
domain errors, `nan` → integer — trapped at runtime), plus the runtime traps
`OVERFLOW`, `STACK_OVERFLOW`, `NO_MATCH`, `BAD_PATTERN` (§19).

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

## 19. Defined behavior — the no-UB catalogue — **implemented (M8c-iii, oracle)**

**Zown has no undefined behavior.** Every operation that is undefined,
implementation-defined, or a silent footgun in other languages has exactly one
outcome here: a *defined value*, a *structured runtime trap* (a `.zerr`), or a
*static rejection* (`zown check`). Each row has a conformance golden; a native
backend must reproduce the row or fail with `UNSUPPORTED` — never diverge silently.

| Situation | Outcome | Golden |
|-----------|---------|--------|
| `int` result outside `[-2^63, 2^63-1]` (`+ - * _ ab pw fl ce rd n`) | trap `OVERFLOW` | `errors_v2/int_overflow`, `pw_overflow`, `n_overflow` |
| integer literal beyond `int` | lex trap `OVERFLOW` | `errors_v2/lit_overflow` |
| `big` arithmetic | exact, unbounded (value) | `cases_v2/big_arith` |
| width policy result (`wr st ck`) | `int` for widths inside `int`, `big` for `u64 i128 u128` | `cases_v2/int_checked` |
| division / modulo by zero — int, big, **and float** | trap `DIV_ZERO` | `errors/divzero`, `errors_v2/float_divzero` |
| zero to a negative power | trap `DIV_ZERO` | (unit test) |
| float overflow (`* + pw`) | `±inf` (value) | `cases_v2/float_ieee` |
| `inf - inf` and other IEEE invalid results *from non-finite inputs* | `nan` (value) | `cases_v2/float_ieee` |
| square root of a negative; negative base with fractional exponent | trap `UB_TRAP` (Zown traps where IEEE would create `nan` from *finite* inputs) | `errors_v2/sqrt_negative` |
| `nan` → integer (`fl ce rd`, `b`) | trap `UB_TRAP` | `errors_v2/float_nan_to_int` |
| `±inf` → integer | trap `OVERFLOW` | `errors_v2/float_inf_to_int` |
| whole float beyond `int` | stays a float (`1e+30`) | `cases_v2/float_ieee` |
| `%` with negative operands | result takes the divisor's sign (Python rule), ints and floats | `cases_v2/defined_behavior` |
| string repeat with count ≤ 0 | `""` | `cases_v2/defined_behavior` |
| vector lane index out of range | trap `BOUNDS` | `errors_v2/vec_oob` |
| wrong lane count / `at` body arity | trap `BOUNDS` | `errors_v2/vec_arity`, `res_arity` |
| stack underflow | trap `STACK_UNDERFLOW` | `errors/underflow` |
| **unbounded recursion** (invocations nested past the limit) | trap `STACK_OVERFLOW` at the invoking site; loops (`;`) never consume depth | `errors_v2/recursion`, `cases_v2/defined_behavior` |
| **block nesting past the parser's limit** | parse trap `REPAIR_SYNTAX` | (unit test) |
| type confusion (`+` on a block, `@` on an int, …) | trap `TYPE_MISMATCH` / `NOT_CALLABLE` | `errors/typemismatch`, `notcallable` |
| unresolved name | **static** `NAME_UNRESOLVED` (rejected before running); runtime trap if reached unchecked | `check/name_unresolved`, `errors/unresolved` |
| capability used without a grant | **static** `CAP_DENIED` when provable; runtime trap `CAP_DENIED` otherwise | `check/flow_*`, `errors_v2/caps_denied*` |
| result not handled exhaustively | **static** `NO_MATCH`; runtime trap `NO_MATCH` | `check/res_exhaustive`, `errors_v2/res_unhandled` |
| null / uninitialized value | **impossible** — no null exists; a name is bound or `NAME_UNRESOLVED` | — |
| use-after-free, out-of-bounds memory, data race | **impossible in v0.2** — no raw memory or shared mutable state; the M9 memory model must keep it so (fat pointers, `BOUNDS`) | M9 |
| resource exhaustion (a `big` too large, a non-terminating loop) | *implementation-defined limits*; not UB (the runtime may trap or keep running) | — |

Every trap above is a value when it happens inside `at` (§16.1):
`[ f @ ] :f [ f @ ] at` yields `err(STACK_OVERFLOW)`. The oracle's limits are
128 nested invocations and 256 nested blocks; native runtimes set their own but
the *kind* of outcome is fixed by this table.

## 20. Ownership — linear bindings — **implemented (M8c-iv, oracle + checker)**

`DESIGN.md` §2.2 asks for move/borrow tokens. In a stack language most of that
discipline already exists: **pop is a move, dup/over is a borrow**, and values on
the stack have exactly one owner. Only *bindings* can hold a value indefinitely,
so ownership needs exactly one token (decision D4, 2026-09-07; `!` and `&`
remain logical-not and over):

| Form | Meaning |
|------|---------|
| `name`  | **borrow** — push a copy; the binding stays live (Part I §3, unchanged) |
| `'name` | **move** — push the value and **unbind** `name`; the value now has one owner: the stack |

```
5 :x  x x + .      # 10  borrows
'x .               # 5   moved out — x is unbound from here on
x                  # NAME_UNRESOLVED (static and run time)
7 :x  x .          # 7   rebinding makes it live again
[ 1 2 + ] :f 'f @  # a block moves out and runs; f is gone
```

- A move of a name with no live binding is `NAME_UNRESOLVED` (op `'name`).
  Builtins are not bindings and cannot be moved; after `'s`, a bare `s` falls
  back to the builtin `to_str` exactly as if the binding had never existed.
- **Static use-after-move.** Within one node sequence the checker (§8) tracks
  names moved so far: a later bare `name` (unless it is also a builtin) or a
  second `'name` is a static `NAME_UNRESOLVED` and the program is rejected.
  A `:name` clears the mark; any node that can run code (`@ ; ??`, `gr`, `at`,
  SIMD constructors) clears all marks, since it may have rebound anything.
  Blocks that run immediately at their position (`@`, the `;` condition, the
  `? @` branches, `??` bodies, `gr`, `at`, constructors) inherit the marks; the
  `;` body (its condition ran first) and escaped blocks start clean. Inside `at`
  the fault is the handled path and is not reported. Goldens: `conformance/check/
  move_*`.
- **Why it matters later.** Today every value is immutable, so a move is only a
  rebinding discipline. When the M9 memory model introduces heap values (`buf`,
  fat pointers), `'name` is the transfer of *exclusive* ownership the drop/no-GC
  story needs, and `"` is reserved for the mutable-reference sigil that story
  will add. The rule is fixed now so the checker's flow analysis is already in
  place when memory arrives.
