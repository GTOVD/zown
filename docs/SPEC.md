# Zown Language Specification — v0.1 (Core)

Zown is an **AI-native, token-dense, stack-based** programming language. It is
designed to be written and read primarily by machines: control flow, stack
manipulation, and most operations are single ASCII characters, and the standard
library uses 1–2 character words. A program rarely needs identifiers at all.

This document defines the **v0.1 core** that the reference interpreter in this
repo actually implements. The longer-term vision (native WASM/LLVM compilation,
fast lanes, embedded graph DB, hot-swap, self-healing) is tracked in
[`ROADMAP.md`](./ROADMAP.md). Where the original design notes contradicted
themselves, this spec is the authority and the contradictions are called out.

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
