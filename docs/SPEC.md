# Zown Language Specification — v0.1 (Core)

Zown is an **AI-native, token-dense, stack-based** programming language. It is
designed to be written and read primarily by machines: control flow, stack
manipulation, and most operations are single ASCII characters, and every prebuilt
word is one character, or two, or three — never longer, and never a second name
for something another token already means. A program rarely needs identifiers at all.

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
| `+` | add               | numbers only. Two strings concatenate with `ct`  |
| `-` | subtract          | numbers only                                     |
| `*` | multiply          | numbers only. `str count rp` repeats a string    |
| `/` | divide            | error `DIV_ZERO` on `/0`                          |
| `%` | modulo            | error `DIV_ZERO` on `%0`                          |
| `_` | negate (unary)    | `5 _` -> `-5`                                     |

> **Resolved contradiction:** the notes used `*` for multiply, for looping, and
> for repeating a string. In Zown, `*` is multiply only. Looping is `;` (§6).
> Repeating a string is the word `rp`. `+` is add only; joining two strings is
> the word `ct`. A dot between two digit runs is a float literal (`5.5`); the
> print operator is the token `.` only when it is not part of that literal
> (`5.` prints 5).

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

### Lanes (SPEC §22)
| Op | Meaning |
|----|---------|
| `~m` | register the block underneath as the input lane |
| `~n` | register the block underneath as the net lane |
| `^` | queue the block underneath on the compute pool |
| `\|` | send the top value into the running lane's pipe |

`||` is still logical or. The lexer takes the two-character operator first, so `|` is the pipe and `||` is or. Any `~name` other than `~m` and `~n` is one token and `UNSUPPORTED` at run time (§14).

---

## 5. Symbols kept open

`|`, `~`, and `^` are the lane operators (§22). The characters below are still unused. Writing one is a lex error, so it stays free:

| Symbol | Reserved for |
|--------|----------------|
| `( )` | tuples / grouping |
| `{ }` | inline control-flow bodies |
| `#` | a comment to the end of the line |

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
| `mx`  | max       | max of the top two numbers. Defined in `std/core.zn` |
| `mn`  | min       | min of the top two numbers. Defined in `std/core.zn` |
| `sq`  | sqrt      | square root                                  |
| `pw`  | pow       | `base exp pw` -> base**exp                   |
| `fl`  | floor     | floor                                        |
| `ce`  | ceil      | ceiling                                      |
| `rd`  | round     | round to nearest int                         |
| `s`   | to_str    | convert top value to its string form         |
| `n`   | to_num    | parse a string into a number (`OVERFLOW` beyond `int`) |
| `b`   | big       | convert an int / whole float / digit string to a `big` (§11). Not a vector prefix |
| `dp`  | depth     | push the current stack depth                 |
| `rt`  | rot       | rotate top three (`a b c -> b c a`)          |
| `cl`  | clear     | clear the whole stack. Defined in `std/core.zn` |
| `ct`  | concat    | concatenate two strings (`a b ct`). Both must already be strings |
| `rp`  | repeat    | `str count rp` repeats the string; a count `<= 0` yields `""` |
| `hw`  | cpus      | push the CPU count (`ZOWN_CPUS` overrides the host) |
| `lc`  | lanes     | `n lc` sets the real-time slot request and pushes the grant (§22) |
| `rn`  | run_ticks | `k rn` runs k scheduler ticks |
| `pm`  | pull_in   | pull one value from the input lane pipe |
| `pn`  | pull_net  | pull one value from the net lane pipe |
| `ws`  | steals    | push how many times a worker stole a job |
| `sg`  | trace     | push the lane trace |
| `wd`  | word      | `[body] $name$ wd` registers an auto-running word |
| `ts`  | test      | `[body] ts` — the body leaves one truthy value |
| `pp`  | prop      | `k [body] pp` repeats that check `k` times |

A prebuilt word is the shortest free spelling that still has one meaning: one
character, then two, then three. `cl` is two because `c` is the standard
clock capability. There is no second stack word for any row above. The manifest
carries the long alias; the source does not.

`mx`, `mn`, and `cl` are Zown, in [`zown/std/core.zn`](../zown/std/core.zn).
The VM loads that file at startup and `wd` registers each block as an
auto-running word. A user `:mx` still shadows it. A trap inside the block is
reported with the word as `op` (`TYPE_MISMATCH mx`, not `TYPE_MISMATCH >`).
`ab` stays a host word: `<` cannot tell `-0.0` from `0.0`, and `ab` must.

`[body] $name$ wd` registers a word. `$name$` is a letter followed by letters
or digits. `[body] ts` runs a test: the body must leave exactly one truthy
value, or the trap is `TEST_FAIL` (`ts`) or `BOUNDS` when the depth is wrong.
`k [body] pp` does that `k` times (`k` is an `int` ≥ 0). `0` repetitions do
not run the body.

---

## 8. Diagnostics (`.zerr` packets)

Errors are structured payloads, not prose. Every error carries a deterministic
recovery **code**, the source position, a **stack snapshot**, and a dense
**hint**, so an AI agent can self-heal deterministically. Run with `--zerr` to
emit JSON instead of the human form.

Recovery codes: `REPAIR_SYNTAX`, `STACK_UNDERFLOW`, `TYPE_MISMATCH`,
`NAME_UNRESOLVED`, `DIV_ZERO`, `BOUNDS`, `NOT_CALLABLE`, `UNSUPPORTED`
(v0.1), plus `OVERFLOW`, `NO_MATCH`, `BAD_PATTERN`, `TEST_FAIL`, and the security codes of
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
         | "." | "@" | "?" | ";" | "??"
         | "|" | "^" | "~" ident ;
ident    = letter { letter | digit } ;
```

---

# Part II — Planned language surface (v0.2 design)

Sections below are **implemented** in the oracle where they say so, and
**design-only** where they say so. The native compiler consumes the implemented
surface. The density audit (2026-10-24) froze the tokens: each one has a single
meaning, and every prebuilt word is at most three characters.

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

**Native `big` (decision D7, 2026-10-24).** The bits are a sign and a
little-endian vector of base-2^32 limbs: no leading zero limb, and zero is
non-negative. That is the representation in `zown-vm` (`BigInt`). It is not a
language-visible encoding; programs only see the decimal form and the `big`
type. A WASM payload for `big` is an index into a side table (tag 4 in
[`WASM.md`](./WASM.md)), because a limb vector does not fit in one `i64`.

**`b` — the big constructor — implemented (M8c-i, oracle).** `v b` converts an
int, a whole float, a big (no-op), or a **string of digits** — which is how a
literal beyond the int range is written: `$99999999999999999999$ b`. Whole floats
print as integers only while they fit `int`; larger whole floats keep float
notation (`1e+30`); `inf`, `-inf`, `nan` print as such and `n` parses them.

| Class | Types |
|-------|-------|
| signed int | `i8 i16 i32 i64 ix` — the suffix is the bit width; `ix` is 128 because `i128` is four characters |
| unsigned int | `u8 u16 u32 u64 ux` — `ux` is unsigned 128 |
| pointer-size | `iz` `uz` (*design-only*; the source words, not `isize` / `usize`) |
| float | one kind, `float` (binary64). A narrower float, if it is ever added, will not be spelled `f32` — that digit shape belongs to widths, and `vf` is already the f32×4 vector |
| exact | `dc` (decimal, design-only), `big` (arbitrary precision — **implemented**, the `b` word), `cx` (complex, design-only) |

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
an `int`; the others (`u64`, `ix`, `ux`) yield a `big` — so `5 u64 ck` is
`5 b`, and the type is static. `dec` and `cx` remain design-only.

### Vector / SIMD types — **implemented (M7c-ii, oracle)**

Lane-typed values that will lower to native SIMD (SSE/AVX/NEON, chosen by target).
A constructor word evaluates a block to produce its lanes:

```
vf   = f32 x4      vd = f64 x2
vi   = i32 x4      vu = u8 x16
[ 1 2 3 4 ] vi               # build an i32x4
[ 1 2 3 4 ] vi [ 10 20 30 40 ] vi va    # vi(11 22 33 44)
[ 1 2 3 4 ] vi vh .          # 10   (horizontal sum -> scalar)
[ 7 8 9 10 ] vi 2 at .       # 9    (lane 2)
```

Every vector word starts with `v` and is two characters. The number in `i8` or
`u64` is a bit width, so a vector is not spelled `i4` or `b16` (`b` is already
the big-integer word). Elementwise ops are `va` `vs` `vm`, the horizontal sum is
`vh`, and the lane index is `at`. They are words rather than `v+` because an
identifier cannot contain an operator character. Ops require matching vector
types (else `TYPE_MISMATCH`); integer lanes wrap to their width, the same rule
as scalar fixed-width ints.

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
| `` `t `` | net-send | scopable: `` `t rate:1000/s ``. Not `` `s ``: the word `s` is `to_str` |
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
[ ... ] :get        # manifest: get.caps = ["`t","`r"]
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
`t hv .                          # 0  -> no authority by default
`t [ `t rq $send-ok$ . ] gr      # grant net-send for this block only
`t hv .                          # 0  -> authority restored after the block
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
   constructor (`vf vd vi vu`), or `gr` — runs with exactly the enclosing
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
> Ed25519/X25519, which a pure stack interpreter cannot exhibit honestly
> (a stand-in would mislead on a security-critical surface). BLAKE3 itself is
> the word `h3` (M12). M14 seals a graph bundle with that hash (§26). `Key`,
> `Sig`, and Ed25519 are still not implemented.

First-class types the type system understands (not raw byte arrays):

| Type | Meaning |
|------|---------|
| `Key` | Ed25519/X25519 key (public or, gated by `` `k ``, secret) |
| `Sig` | a signature |
| `Hash` | a BLAKE3 content hash (256-bit) |
| `NodeID` | 128-bit identity derived from a public key (= IPv6 address) |

Defaults are fixed and hard to misuse: BLAKE3 (hash), ChaCha20-Poly1305
(symmetric), Ed25519 (sign), X25519 (exchange). One algorithm per purpose.

## 14. Network primitives (`~` family) & protocol-as-type

> `~m` and `~n` are the real-time lanes (§22). `~p` and `~f` publish and fetch
> a BLAKE3-addressed bundle (§24). The rest of this family is not implemented:
> streams, QUIC, and protocol-as-type checking stay design-only. A `~name`
> outside that set is one token and traps `UNSUPPORTED`.

The reserved `~` family (Part I §5) becomes the network/lane surface. A connection
targets an **interface**, not an IP; the runtime resolves which peer provides it
(`DESIGN.md` §3). On-device (`~l`) and remote share one interface.

| Token | Meaning |
|-------|---------|
| `~o` | open a stream to a peer — *not implemented* |
| `~s` | send on a stream (needs `` `t ``). The `~` makes this a different token from the word `s` — *not implemented* |
| `~r` | recv from a stream (needs `` `r ``) — *not implemented* |
| `~c` | connect to an interface (resolved via the mesh) — *not implemented* |
| `~l` | local same-machine fast path (shared memory) — *not implemented* |
| `~w` | world: tunnel/relay for legacy reach — *not implemented* |
| `~n` `~m` | network / input real-time lanes — **implemented** (§22). Not a stream send |
| `~p` | publish a string bundle; push its BLAKE3 hex — **implemented** (§24) |
| `~f` | fetch a bundle by that hex — **implemented** (§24) |

**Protocols are types.** Two programs connect iff their protocol types are
structurally compatible (checked at compile time, else at capability-grant time):

```
# a protocol type (conceptual; dense form TBD)
proto Store [ get:[Hash]->[val?]  put:[Hash val]->[ok|err] ]
`t Store ~c :db        # connect to any peer that speaks Store
```

Content addresses are written `$zown:<hash>$`; mutable refs are
`$zown:<pubkey>$` (latest signed version). `~f` fetch / `~p` publish operate on them.

## 15. Native UI & GPU types — *design-only, except the layout slice in §25*

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
42 [ [int]   [ , $an int$ . ]      # type pattern: int/big/float/str/block/cap/width/vec
     [str]   [ up . ]              # body sees the subject (here: uppercase it)
     [7]     [ , $seven$ . ]       # literal pattern (int/float/str), compared by value
     [any]   [ , $other$ . ] ] ??  # any is the default arm. There is no bool type
```

| Pattern | Matches |
|---------|---------|
| `[int]` `[big]` … `[vec]` | a value of that type (`type_name`) |
| `[42]` `[$hi$]` | a value equal to the literal (`5 b` matches `[5]`) |
| `[any]` | anything (default). `any` is not a stack word, and `_` is only negate |

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
[ body ] ty            # run body; ok(its single value) or err(the .zerr code it raised)
```

- `ty` runs the body on an **isolated stack** (like the SIMD constructors) so its
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
  `[ … ] ty [then] [else] ? @` branches without exposing the payload.
- **Exhaustiveness is a static rule.** For the literal shape `[ … ] ty [ arms ]
  ??` the checker requires the arms to cover both `ok` and `err` — via
  `[ok]`+`[err]`, `[res]`, or a default `[any]` — else a static `NO_MATCH` and the
  program is rejected before it runs (§8). At run time an unhandled err is
  `NO_MATCH` as well.

```
5 :x 0 :y
[ x y / ] ty [ [ok]  [ 2 * . ]
               [err] [ $div failed: $ \ ct . ] ] ??   # div failed: DIV_ZERO
[ `t rq 1 ] ty ! [ $no net$ . ] [ ] ? @              # probe a capability without faulting
```

Inside a `ty` body a fault is the *intended* err path, so the checker does not
report capability-flow denials there; well-formedness findings (unresolved names,
malformed arms, exhaustiveness) are still reported — no execution makes them right.

## 17. Reserved-symbol allocation (updated)

Part I §5 keeps `( )`, `{ }`, and `#` open. Part II claims:

| Symbol | Now reserved for |
|--------|------------------|
| `` ` `` | capability sigil (§12) |
| `'` | move sigil (§20) — **implemented** |
| `"` | reserved for a future *mutable reference* sigil once heap values exist (§20) |
| `~x` words | `~m` and `~n` are lanes (§22). Every other `~name` is still a network word (§14) and traps `UNSUPPORTED` |
| `» « →` | graph query (§26) — **implemented**. `»` opens a cursor, `→` follows one label, `«` joins the names |
| `??` | pattern-match dispatch (§16). `?` stays select; `??` is not a second meaning of `?` |

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
  "cap": "`t",
  "node": "zown:abc123...",
  "hint": "grant `t in the module manifest, or route via a capability holder"
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
| width policy result (`wr st ck`) | `int` for widths inside `int`, `big` for `u64 ix ux` | `cases_v2/int_checked` |
| division / modulo by zero — int, big, **and float** | trap `DIV_ZERO` | `errors/divzero`, `errors_v2/float_divzero` |
| zero to a negative power | trap `DIV_ZERO` | (unit test) |
| float overflow (`* + pw`) | `±inf` (value) | `cases_v2/float_ieee` |
| `inf - inf` and other IEEE invalid results *from non-finite inputs* | `nan` (value) | `cases_v2/float_ieee` |
| square root of a negative; negative base with fractional exponent | trap `UB_TRAP` (Zown traps where IEEE would create `nan` from *finite* inputs) | `errors_v2/sqrt_negative` |
| `nan` → integer (`fl ce rd`, `b`) | trap `UB_TRAP` | `errors_v2/float_nan_to_int` |
| `±inf` → integer | trap `OVERFLOW` | `errors_v2/float_inf_to_int` |
| whole float beyond `int` | stays a float (`1e+30`) | `cases_v2/float_ieee` |
| `%` with negative operands | result takes the divisor's sign (Python rule), ints and floats | `cases_v2/defined_behavior` |
| string repeat (`rp`) with count ≤ 0 | `""` | `cases_v2/defined_behavior` |
| vector lane index out of range | trap `BOUNDS` | `errors_v2/vec_oob` |
| wrong lane count / `ty` body arity | trap `BOUNDS` | `errors_v2/vec_arity`, `res_arity` |
| stack underflow | trap `STACK_UNDERFLOW` | `errors/underflow` |
| **unbounded recursion** (invocations nested past the limit) | trap `STACK_OVERFLOW` at the invoking site; loops (`;`) never consume depth | `errors_v2/recursion`, `cases_v2/defined_behavior` |
| **block nesting past the parser's limit** | parse trap `REPAIR_SYNTAX` | (unit test) |
| type confusion (`+` on a block, `@` on an int, …) | trap `TYPE_MISMATCH` / `NOT_CALLABLE` | `errors/typemismatch`, `notcallable` |
| unresolved name | **static** `NAME_UNRESOLVED` (rejected before running); runtime trap if reached unchecked | `check/name_unresolved`, `errors/unresolved` |
| capability used without a grant | **static** `CAP_DENIED` when provable; runtime trap `CAP_DENIED` otherwise | `check/flow_*`, `errors_v2/caps_denied*` |
| result not handled exhaustively | **static** `NO_MATCH`; runtime trap `NO_MATCH` | `check/res_exhaustive`, `errors_v2/res_unhandled` |
| null / uninitialized value | **impossible** — no null exists; a name is bound or `NAME_UNRESOLVED` | — |
| use-after-free, out-of-bounds memory, data race | **impossible in this model** — no raw memory (§21). Lanes share one stack and move values only through the pipe (§22), so a tick cannot race itself | §21, §22 |
| lane pipe holds 8 values, or a pull from an empty pipe | trap `BOUNDS` | `errors_v2/pipe_full`, `pipe_empty` |
| `\|` outside a running lane | trap `TYPE_MISMATCH` | `errors_v2/pipe_outside` |
| `rn` while a tick is already running | trap `TYPE_MISMATCH` | `errors_v2/tick_reenter` |
| `lc` or `rn` given a negative int | trap `BOUNDS` | `errors_v2/lane_negative` (`rn` is the same rule) |
| `~` name other than `~m` or `~n` | trap `UNSUPPORTED` | `errors_v2/lane_unknown` |
| a queued compute job skips a real-time lane | **impossible** — each tick runs every registered lane, then at most one job | `cases_v2/lanes` |
| `ts` / `pp` body leaves a false value | trap `TEST_FAIL` | `errors_v2/test_fail` |
| a graph name that is empty or contains whitespace | trap `TYPE_MISMATCH` | (unit test) |
| `so` on a bundle whose BLAKE3 trailer does not match the bytes | trap `INTEGRITY_FAIL` (kind `sec`) | `errors_v2/bundle_bad` |
| `so` on text that is not a `zown-sync` bundle | trap `TYPE_MISMATCH` | (unit test) |
| `se` when no span is open | trap `TYPE_MISMATCH` | `errors_v2/span_empty` |
| `ch` given a string that is not 64 hex digits | trap `TYPE_MISMATCH` | (unit test) |
| `hd` on an empty string | trap `BOUNDS` | `errors_v2/head_empty` |
| `bf` length above 1048576 | trap `BOUNDS` | (unit test) |
| `bf` given a negative int | trap `TYPE_MISMATCH` | (unit test) |
| `bg` / `bp` index outside the buffer | trap `BOUNDS` | `errors_v2/buf_oob` |
| `bp` on a borrow | trap `TYPE_MISMATCH` | `errors_v2/buf_borrow` |
| `bg` or `ln` after the exclusive buf was dropped | trap `BOUNDS` | `errors_v2/buf_freed` |
| `nt` given a string with a newline | trap `TYPE_MISMATCH` | (unit test) |
| `ts` / `pp` body leaves a number of values other than one | trap `BOUNDS` | (unit test) |
| resource exhaustion (a `big` too large, a non-terminating loop) | *implementation-defined limits*; not UB (the runtime may trap or keep running) | — |

Every trap above is a value when it happens inside `ty` (§16.1):
`[ f @ ] :f [ f @ ] ty` yields `err(STACK_OVERFLOW)`. The oracle's limits are
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
  A `:name` clears the mark; any node that can run code (`@ ; ??`, `gr`, `ty`,
  SIMD constructors) clears all marks, since it may have rebound anything.
  Blocks that run immediately at their position (`@`, the `;` condition, the
  `? @` branches, `??` bodies, `gr`, `ty`, constructors) inherit the marks; the
  `;` body (its condition ran first) and escaped blocks start clean. Inside `ty`
  the fault is the handled path and is not reported. Goldens: `conformance/check/
  move_*`.
- **Why it matters later.** Today every value is immutable, so a move is only a
  rebinding discipline. When the M9 memory model introduces heap values (`buf`,
  fat pointers), `'name` is the transfer of *exclusive* ownership the drop/no-GC
  story needs, and `"` is reserved for the mutable-reference sigil that story
  will add. The rule is fixed now so the checker's flow analysis is already in
  place when memory arrives. The heap those moves will own is specified in §21
  and is not implemented: the oracle does not allocate a `buf`.

## 21. Memory — fat pointers (`buf`) — **implemented (M18, oracle + Rust VM)**

A `buf` is the only reference to heap memory. It is a fat pointer: a base the
program cannot observe, a byte length, and a permission. There is no integer
address, no null, and no way to forge one. An index outside `0 .. length`, or
a use after the exclusive owner has been dropped, is `BOUNDS`. It is never a
raw load. The runtime grants at most 1048576 bytes per `bf`. A larger request
is `BOUNDS`. mmap and sharing across processes are not this value.

| Word | Effect |
|------|--------|
| `bf` | `n bf` allocates `n` zero bytes and pushes an **exclusive** `buf`. `n` is an `int` `≥ 0`, else `TYPE_MISMATCH`. A length the runtime will not grant is `BOUNDS` (an implementation-defined budget, not UB). |
| `ln` | `buf ln` pushes the byte length. `ln` keeps its one meaning, length, and also accepts a string or a block. |
| `bg` | `buf idx bg` pushes the byte at `idx` as an `int` in `0..255`. |
| `bp` | `buf idx byte bp` stores `byte` (`int`) at `idx`. |

`bg` and `bp` are three characters or fewer and do not reuse `at` (vector
index), `st` (saturate), or `b` (big).

Stack effects: `n bf` pushes the buf. `buf idx bg` leaves the buf and pushes the byte. `buf idx byte bp` leaves the buf. `buf ln` consumes the buf and pushes the length, the same shape as `ln` on a string. Consuming the exclusive value (`ln`, `.`, `pr`, `s`, or `,`) frees the bytes. `bg` and `bp` do not consume it, so the owner can keep writing.

A bare name of an exclusive binding pushes a borrow. The binding stays exclusive. `=` on a buf pushes a borrow and leaves the original value under it. `'name` moves the value as it is: an exclusive stays exclusive, and the bytes are not freed by the move.

It prints as `bf(n)` when exclusive and `bf&(n)` when borrowed. A freed buffer prints as `bf(-)` or `bf&(-)`. The number in parentheses is the length, never an address. The type name for `??` is `buf`. A buf is truthy when its length is greater than 0.

**Ownership.** Allocation is exclusive.

- `=` on a `buf` pushes a **borrow** and leaves the exclusive value under it.
  A borrow may `bg` and `ln`. It may not `bp` (`TYPE_MISMATCH`).
- `,` on an exclusive `buf` frees the bytes, then drops the value. `,` on a
  borrow drops only the borrow.
- `'name` moves whatever the binding held. Exclusive stays exclusive. The
  bytes are freed only when that exclusive value is dropped.
- A borrow whose exclusive owner has already been dropped is dangling. `bg`
  and `ln` on it are `BOUNDS`. `bp` on any borrow is `TYPE_MISMATCH`. The fat
  pointer does not become an address the program can reuse.

`"` stays reserved for a future mutable reference that is not exclusive
ownership (§17, §20). It is not a word.

## 22. Concurrency — fast lanes — **implemented (M10, oracle + Rust VM)**

`~`, `^`, and `|` schedule work. The oracle does not start an OS real-time
thread. It runs a cooperative tick scheduler, and the Rust VM, including a
program compiled with `zownc native`, follows the same steps. A later host that
pins OS threads must not change the trace this section defines.

Lanes and jobs share the one operand stack. A body in the conformance programs
is stack-neutral aside from the pipe. The scheduler does not preempt: a slice
runs until its block returns, and `rn` from inside a slice is `TYPE_MISMATCH`.

### Topology

`hw` pushes the CPU count. `ZOWN_CPUS`, when it is an integer ≥ 1, replaces the
host count so a test can pin the topology. An empty or non-numeric value is
ignored.

`room = max(cpus - 1, 1)`. One CPU stays a worker when the machine has more than
one; a single CPU still offers one real-time slot.

`n lc` sets the requested slot count and pushes the grant `min(n, room)`. `n`
is a plain `int`. A non-int is `TYPE_MISMATCH`. A negative int is `BOUNDS`.
Calling `lc` again rebalances before the next compute slice. Until the first
`lc`, the request is the default `min(2, room)`.

Workers = `max(cpus - granted, 1)`. Granted slots are not also workers.

`zown tune` prints `cpus`, `granted`, `workers`, and `pinned` when the grant is
at least one, otherwise `fibers`. `zown tune N` uses N as the request. It calls
the same function the VM uses.

### Registering work

| Token | Effect |
|-------|--------|
| `~m` | pop a block and register it as the input lane. A non-block is `NOT_CALLABLE` |
| `~n` | the same, for the net lane |
| `^` | pop a block onto worker 0's deque. A non-block is `NOT_CALLABLE` |
| `\|` | pop a value into the running lane's pipe. Outside a lane this is `TYPE_MISMATCH` |
| `pm` | pull one value from the input pipe (FIFO) |
| `pn` | pull one value from the net pipe |

`|` is send only. `pm` and `pn` are the pulls, so the pipe operator does not
also mean receive. The pipe holds 8 values. A send into a full pipe, or a pull
from an empty one, is `BOUNDS`.

`~` must be followed by a name or the lexer raises `REPAIR_SYNTAX`. `~m` and
`~n` are the only implemented names. Any other `~name` (the network words in
§14) is `UNSUPPORTED`. The checker treats a block consumed by `~m`, `~n`, or
`^` as escaped: those operators are not inline invocations like `gr` or `ty`.

### Ticks

`k rn` runs k ticks. `k` is a plain `int` ≥ 0, with the same type and bounds
rules as `lc`. The trace (`sg` pushes it) accumulates across `rn` calls. `ws`
pushes the steal count. Neither is reset by a later `rn`.

Each tick, in order:

1. The registered lanes, input then net. The first `granted` of those are the
   pinned class; the rest run as fibers. Both classes run once, pinned first.
   A lane with no body is skipped and does not take a slot.
2. At most one compute job. The worker is `rr` modulo the worker count. `rr`
   starts at 0 and increases by one each tick, including a tick with no job.
   That worker pops from the left of its deque. If the deque is empty, the
   scheduler scans the following workers and pops from the right of the first
   non-empty deque, and that pop counts as one steal. No job means no `c`.

The trace is one letter per slice that actually ran: `m`, `n`, then `c` when a
job ran. A job queued by a lane is visible to the compute slice of that same
tick. An `lc` in a lane updates the worker count before that slice; the lane
list for the tick was chosen at the start of the tick.

A queued job cannot skip a lane. Every tick spends its compute budget on one
job after every registered lane has run. That is the real-time property this
runtime can show. Wall-clock priority and `SCHED_FIFO` are not part of the
observable behavior.

### What a backend must match

WASM does not emit `|`, `^`, or `~name`. It refuses them at compile time. A
native backend that runs the program must reproduce the trace and the traps in
§19. Two hosts may report different `hw` values; a conformance program that
needs a stable trace calls `1 lc`, which grants one slot on every machine.

## 23. Batteries — *design-only, except the slice M11 shipped*

M11 ships three things: the std words in §7 (`mx`, `mn`, `cl`), `zown test`,
and `zown fmt`. The rest of the zero-install stdlib is named here so it is not
invented later under a second spelling, and it is **not implemented**. A stub
that hashed with a stand-in or pretended to read a file would be a lie.

| Area | Frozen choice | Word, when it exists |
|------|----------------|----------------------|
| Hash | BLAKE3 | `h3` — **implemented** (§24). The digest is 64 hex digits, not a separate value type yet |
| Symmetric box | ChaCha20-Poly1305 | `ae` seals, `ad` opens |
| Signature | Ed25519 | `es` signs, `ev` verifies |
| Key agreement | X25519 | `xk` |
| Files | capability-scoped paths | not a word yet; needs `` `f `` |
| Clock | wall and monotonic | already the capability `` `c ``; no word reads it |
| Env | the process environment | not a word; `ZOWN_CPUS` is the one override that exists (§22) |

Collections (`List`, `Map`, …) are not a second kind of block. A block is code.
A value collection waits on the fat pointer (§21). LSP, a doc generator, and a
mesh REPL are the same toolchain story and are not commands yet.

`zown fmt` rewrites whitespace only: one space between tokens, source line
breaks kept, blank lines removed, a trailing newline. It is idempotent.
`zown test [path]` runs `*.zn` files (default `tests/zn`). A sibling `.out` is
the stdout golden, a sibling `.code` is the recovery code, and a file with
neither passes when it runs without a trap. The runner is ungated, like the
conformance oracle.

## 24. ZownNet — the slice that runs — **implemented (M12, oracle + Rust VM)**

A bundle is a string. Its address is `h3`: BLAKE3 of the UTF-8 bytes, written
as 64 lowercase hex digits. `~p` stores the string under that address and
pushes the address. `~f` pops an address and pushes the string. A non-string,
or a hash that is not 64 hex digits, is `TYPE_MISMATCH`. An address with no
bundle is `BOUNDS`. The store is local to the VM. A golden can publish and
fetch without a socket (`cases_v2/bundle`).

`ni` pushes this VM's node id, 64 hex digits. `ZOWN_NODE`, when set and
non-empty, is hashed with BLAKE3 to make that id; otherwise the id is 32
random bytes. `ua` pushes the IPv6 unique-local form: the first byte is
`0xfd` and the next 15 bytes are the start of the node id, printed as eight
groups of four hex digits. This id is **not** an Ed25519 key. Signing,
X25519, and ChaCha20-Poly1305 are still the unimplemented words in §23.

`zown mesh [port]` runs two peers. Each is an IPv6 UDP socket. They rendezvous
at `[::1]:port` (default 59770): one binds it, the other announces, then fetches
the bundle `hello bundle` by its BLAKE3 address and checks the hash. The same
announce is offered to `ff02::5a77` on that port. A host that cannot deliver
the group still completes the exchange on `[::1]`, which does not leave the
machine. QUIC, the DHT, relays, and `~o`/`~s`/`~r` are not this command.

## 25. Layout — **implemented (M13, oracle + Rust VM)**

UI is data. There is no markup. A program builds a tree and `ly` pushes its
geometry. `zown draw` paints that tree as a PPM. This is a CPU picture of the
rectangles, not a GPU, a font, or an audio graph. Those stay design-only (§15).

| Word | Stack | Result |
|------|--------|--------|
| `co` | `r g b co` | a color. Each channel is a plain `int` in `0..255`, else `BOUNDS` |
| `tx` | `$text$ size color tx` | a text node. `size` is an `int` ≥ 1. Width is the number of characters times `size`; height is `size`. A newline in the text is `TYPE_MISMATCH` |
| `bt` | `$label$ size color bt` | a button. Width is the text width plus `size` on each side; height is `size * 2`. The accessible name is the label |
| `rw` | `color gap pad [nodes] rw` | a row. The block runs on a fresh stack and must leave nodes |
| `cm` | `color gap pad [nodes] cm` | a column, same rule |
| `ly` | `node ly` | the dump below, one string |

`pad` and `gap` are plain `int`s ≥ 0, else `BOUNDS`. A child that is not a node is `TYPE_MISMATCH`. Children are start-aligned: a column stacks them downward, a row lays them to the right, separated by `gap`, inset by `pad`. An empty box is `pad*2` by `pad*2`.

`ly` walks preorder. A box line is `rw|cm x y w h`. A leaf line is `tx|bt x y w h ok|low text`. Indentation is two spaces per level. Coordinates are the top-left of the root. `ok` means the leaf's color against its parent box (or black, if it is the root) passes the contrast test; `low` means it does not. Luma is linear Rec. 709, `(2126*r + 7152*g + 722*b) // 1000`. It passes when `(hi+128)*10 >= (lo+128)*45`.

`zown draw file.zn` runs the program and paints the one node it leaves. The root must have area. The file is a binary `P6` PPM: boxes fill their rectangle with their color, then children paint over them; a leaf fills its rectangle with its own color. Animation, input delivery, shaders, and audio are not in this picture.

## 26. The graph — **implemented (M14, oracle + Rust VM)**

The store that runs is an in-memory graph plus a text bundle. Vertices and edges are single words: no whitespace, not empty. A duplicate vertex or edge is a no-op and does not move the one that is already there. Insertion order is the order a snapshot writes. This is not a memory map, not a column store, and not a CRDT. Those, and a signed patch that hot-swaps code, stay design-only (`DESIGN.md` §6–§7).

| Word or op | Stack | Result |
|------------|--------|--------|
| `gv` | `name gv` | ensure a vertex. Pushes nothing |
| `ge` | `src label dst ge` | ensure the edge `src -label→ dst`, and both vertices |
| `gs` | `gs` | push the snapshot text |
| `gl` | `snapshot gl` | replace the graph with that snapshot |
| `gh` | `gh` | push BLAKE3 of the snapshot's UTF-8, 64 lowercase hex digits |
| `»` | `name »` | push a cursor. It holds `name` if that vertex exists, else it is empty. A missing name is not an error |
| `→` | `cursor label →` | follow `label` from each name in the cursor. Edge order is insertion order. A destination is kept once, the first time it is seen |
| `«` | `cursor «` | pop the cursor and push its names joined by one space. An empty cursor pushes `""` |
| `sb` | `sb` | push the sealed bundle |
| `so` | `bundle so` | check the seal, replace the graph, push the new `gh` |

A name that is not a string, or a string that is empty or contains whitespace, is `TYPE_MISMATCH`. `→` and `«` on a value that is not a cursor are `TYPE_MISMATCH`. The cursor's type name is `cur`. It is truthy when it holds at least one name. It prints as `cur(n)`, where `n` is how many names it holds.

The snapshot is one line per vertex, then one line per edge, in insertion order:

```
v a
v b
e a knows b
```

Each line ends with `\n`. An empty graph is the empty string. `gh` of that empty graph is BLAKE3 of zero bytes: `af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262`.

The bundle is the snapshot with a header and a trailer. The header is the line `zown-sync 1`. The trailer is the line `blake3` plus the BLAKE3 of every byte before it, including the newline that ends the body:

```
zown-sync 1
v a
blake3 <64 hex digits>
```

`so` finds the last line that starts with `blake3 `, hashes the bytes before that line, and compares. A trailer that is not 64 hex digits, a missing header, or a snapshot line the parser cannot read is `TYPE_MISMATCH`. A trailer that does not match the bytes is `INTEGRITY_FAIL` with kind `sec`. The graph is left unchanged. The check is a hash, not a signature. `SIG_INVALID` is still not produced, because nothing here is signed.

`zown keep export file.zn -o out.sync` checks the program, runs it, and writes the bundle. It fsyncs the file and the directory that holds it. A crash after that command returns still has those bytes. A crash in the middle of the write can leave a short file; `verify` then rejects it. `zown keep verify out.sync` opens the bundle in a fresh VM and prints `ok` and the head. It does not apply a patch to running code.

`cases_v2/graph` is the two-hop query and its head. `errors_v2/bundle_bad` is a trailer of 64 zeros.

## 27. The control plane — **implemented (M15, oracle + Rust VM)**

What runs is one VM's log. Events are numbered from 1 in the order the program causes them. There is no clock and no second machine, so the text is the same on every run. A fleet, a trace that crosses a network, a hot-swap, and on-device inference stay design-only (`DESIGN.md` §8).

Each line is:

```
seq content version trace kind rest
```

Until the program sets them, content, version, and trace are `-`. `ch` sets the content tag from 64 hex digits (lowercase when stored; hash a string with `h3` first). `vr` sets the version and `td` sets the trace id. Both are one word. Tags apply to later lines. They do not rewrite earlier ones.

| Word | Stack | Result |
|------|--------|--------|
| `ch` | `$hex$ ch` | set the content tag. Not 64 hex digits is `TYPE_MISMATCH` |
| `vr` | `$word$ vr` | set the version tag |
| `td` | `$word$ td` | set the trace id |
| `sp` | `$name$ sp` | open a span. The log kind is `open` |
| `se` | `se` | close the innermost span. The log kind is `close`. No open span is `TYPE_MISMATCH` |
| `nt` | `$text$ nt` | append a note. The text may contain spaces. A newline or an empty string is `TYPE_MISMATCH` |
| `wn` | `$name$ $value$ wn` | declare what `name` should be. Both are one word. Setting it again keeps its place and changes the value |
| `ob` | `$name$ $value$ ob` | record the actual word. The log kind is `obs`. If a want exists and matches, the next line is `meet`. If it differs, the next line is `drift` |
| `df` | `df` | push the drift report. One line `name want have` for each want that is unmet, in the order they were declared. A missing observation is `-`. Every want met, or no wants, pushes `ok` |
| `rm` | `$code$ $note$ rm` | remember a one-word note for a fault code. It is not run |
| `lg` | `lg` | push the log. Empty is `""` |

Every runtime fault is a log line `fault CODE op note`, written when the fault is raised. `op` is `-` when the fault has none. `note` is the `rm` note for that code, or `-`. A fault caught by `ty` is still in the log; `ty` itself still pushes `err(CODE)` and nothing else. The note is a record of a known response. It does not replace the running program.

`zown watch file.zn` checks the program, runs it, and then prints the log. The program's own output comes first. A fault still prints the log, including that fault's line, and the process exits 1.

`cases_v2/intent` declares a want, watches it drift and then meet, and records a `DIV_ZERO` whose note was set with `rm`. `errors_v2/span_empty` is a bare `se`.

## 28. The frontend — **implemented (M16 lexer, M19 parser, both on stage-0)**

Stage-0 is still the Rust toolchain and this Python VM. The lexer and the parser are also Zown programs, `zown/std/lex.zn` and `zown/std/parse.zn`. They are not loaded with the other std words.

`zown stage file.zn` places the file's text in `z` and runs `lx`. Two host words make that program possible. `hd` pushes the first character of a string, one code point. An empty string is `BOUNDS`. `tl` pushes the rest. An empty string stays empty. Anything that is not a string is `TYPE_MISMATCH` (`cases_v2/head`).

`lx` prints one line per token: the kind, a space, and the raw lexeme. Comments are not printed. A string's lexeme includes both `$` quotes and the escape characters as they appear in the source. The scanner still reads escapes, so `\$` does not end the string. The host lexer (`token_dump`) prints the same lines for every program it accepts. `zown stage zown/std/lex.zn` is the fixed point: the lexer tokenizes its own source, and the dump matches the host. `conformance/stage_parity.py` runs that same program through `zownc native`.

`zown stage parse file.zn` binds the host token dump to `d` and runs `px`. That dump is the text `lx` prints. `px` prints the canonical AST, and the host parser prints the same text. The fixed point is `zown stage parse zown/std/parse.zn`. `conformance/parse_parity.py` runs that program through `zownc native`.

The canonical AST is one node per line. A block is a `blk` line, the nodes inside indented two spaces, and an `end` line at the block's own indent. A leaf is the node tag, one space, and the raw lexeme:

```
int 007
float 1.50
str $a\n$
name x
bind :x
move 'x
cap `t
op +
blk
  int 1
end
```

The lexeme is the source text, not the value the host stores on the node. `007` stays `007`. `1.50` stays `1.50`. A string keeps both `$` quotes and the escape characters. A real newline inside a string stays inside the lexeme, and the dump's token separator is the newline after the closing `$`. Comments are absent, because the dump has already dropped them. `zown ast` still prints JSON for debugging. That JSON is not this text.

An integer literal past the `int` range, a file the host lexer rejects, and a file the host parser rejects (`REPAIR_SYNTAX`, including a block nested past 256) stay host errors. This slice does not reimplement them. The lowering and a compiler binary produced by Zown are not in this slice. A byte-identical stage-2 executable is still M23.

## 29. Bare metal — **implemented (M17, boot sector)**

`zownc bare file.zn -o boot.bin` writes a 512-byte PC boot sector. The program may only print string literals: `$text$ pr` prints the text, and `$text$ .` prints it and then a newline. Anything else is refused. A NUL in the text is refused. Text that does not fit the sector is refused.

The sector runs in 16-bit real mode. The BIOS loads it at `0x7C00`. It writes each character to the VGA text buffer at `0xB8000` with attribute `0x07`, 80 columns to a row. A newline starts the next row. The same bytes, including newlines, are written to port `0xE9` so a host without a framebuffer can see them. After the text, the sector runs `int 0x20`. The handler stores `0xB1` at linear address `0x8000` and returns with `iret`. Then the CPU halts.

This is the text on the screen and one software interrupt. It is not a long-mode kernel, not a hardware IRQ, and not a microkernel. Capability isolation and the rest of a Zown OS stay design-only. `examples/bare.zn` prints `Zown`. `conformance/bare_boot.py` boots that image in QEMU and reads the VGA cells back.
