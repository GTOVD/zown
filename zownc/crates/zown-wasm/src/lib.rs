//! Zown WASM backend — lowers Zown IR to WebAssembly text (`.wat`).
//!
//! Milestone M6, slice **M6b**: the tagged-value runtime + strings.
//!
//! Every Zown value is represented on the WASM value stack as a pair of `i64`s:
//! a **tag** (deeper) and a **payload** (on top).
//!
//!   * tag `0` = int   — payload is the `i64` value
//!   * tag `1` = float — payload is the `f64` bit pattern   (reserved, M6d)
//!   * tag `2` = str   — payload is a pointer into linear memory
//!   * tag `3` = block — payload is a block table index      (reserved, M6c)
//!
//! Strings live in linear memory as `[i32 len][bytes...]`; the pointer addresses
//! the length field. String literals are emitted into a data segment; dynamic
//! strings (concat / repeat / case / trim / reverse) are bump-allocated from a
//! heap (`$hp`). A small fixed runtime prelude provides the string helpers and a
//! tagged `$print_value`.
//!
//! Supported this slice: ints, strings, `+ - * % _`, comparisons, logic, `.`,
//! the stack ops `= , \ &` and `rt`, and the string builtins `tr up lo rv ln`.
//! Floats (M6d) and `[ ... ]` blocks / control flow (M6c) still return a clear
//! `Err`. See `docs/PLAN.md` and `docs/WASM.md`.

use std::collections::HashMap;
use zown_ir::{Instr, IrProgram};

// Memory map (byte offsets):
//   [0, 8)     iovec for fd_write (ptr@0, len@4)
//   16         single newline byte (written on demand)
//   [44, 64)   itoa scratch (digits written backwards, ending at 64)
//   [1024, ..) string-literal data segment
//   heap       bump allocator ($hp), starts just past the literals
const LIT_BASE: usize = 1024;

/// Lower an IR program to a `.wat` module string, or report the first
/// unsupported construct for this backend slice.
pub fn emit_wat(prog: &IrProgram) -> Result<String, String> {
    if prog.blocks.len() != 1 {
        return Err(
            "wasm(M6c): programs with `[ ... ]` blocks are not supported yet \
             (needs call_indirect + a function table)"
                .to_string(),
        );
    }
    let main = &prog.blocks[prog.main];

    // Assign each distinct string literal an offset in the data segment.
    let mut lits: HashMap<String, usize> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut cursor = LIT_BASE;
    for instr in &main.0 {
        if let Instr::ConstStr(s) = instr {
            if !lits.contains_key(s) {
                lits.insert(s.clone(), cursor);
                order.push(s.clone());
                let size = 4 + s.as_bytes().len();
                cursor = (cursor + size + 3) & !3; // 4-byte align
            }
        }
    }
    let heap_base = (cursor + 7) & !7; // 8-byte align
    let pages = ((heap_base + 1_048_576) + 65_535) / 65_536; // +1 MiB of heap

    let mut data = String::new();
    for s in &order {
        let off = lits[s];
        data.push_str(&format!("  (data (i32.const {off}) \"{}\")\n", encode_str(s)));
    }

    let mut body = String::new();
    let mut depth: i64 = 0;
    for instr in &main.0 {
        emit_instr(instr, &lits, &mut body, &mut depth)?;
    }
    // Drop any values the program left on the stack (two i64 per value).
    for _ in 0..depth.max(0) {
        body.push_str("    drop\n    drop\n");
    }

    Ok(module(pages, heap_base, &data, &body))
}

/// Encode a string as `[i32 len LE][bytes]`, fully `\HH` escaped for `.wat`.
fn encode_str(s: &str) -> String {
    let bytes = s.as_bytes();
    let len = bytes.len() as u32;
    let mut out = String::new();
    for b in len.to_le_bytes() {
        out.push_str(&format!("\\{b:02x}"));
    }
    for &b in bytes {
        out.push_str(&format!("\\{b:02x}"));
    }
    out
}

fn push(body: &mut String, tag: &str, payload: &str) {
    body.push_str(&format!("    {tag}\n    {payload}\n"));
}

// Pop sequences: payload is on top, tag below it.
const POP_A: &str = "    local.set $ap\n    local.set $at\n";
const POP_B: &str = "    local.set $bp\n    local.set $bt\n";
const POP_C: &str = "    local.set $cp\n    local.set $ct\n";

fn emit_instr(
    instr: &Instr,
    lits: &HashMap<String, usize>,
    body: &mut String,
    depth: &mut i64,
) -> Result<(), String> {
    match instr {
        Instr::ConstInt(v) => {
            push(body, "i64.const 0", &format!("i64.const {v}"));
            *depth += 1;
        }
        Instr::ConstStr(s) => {
            let off = lits[s];
            push(body, "i64.const 2", &format!("i64.const {off}"));
            *depth += 1;
        }
        Instr::ConstFloat(_) => {
            return Err("wasm(M6d): float literals need the float path".to_string())
        }
        Instr::Op(s) => emit_op(s, body, depth)?,
        Instr::Load(name) => emit_load(name, body, depth)?,
        other => {
            return Err(format!(
                "wasm(M6c/M6d): unsupported instruction `{other:?}` \
                 (blocks/control and bindings come in M6c)"
            ))
        }
    }
    Ok(())
}

fn emit_op(op: &str, body: &mut String, depth: &mut i64) -> Result<(), String> {
    match op {
        "+" => {
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.or (i64.eq (local.get $at) (i64.const 2)) \
                                  (i64.eq (local.get $bt) (i64.const 2)))\n\
                 \x20     (then (local.set $rt (i64.const 2))\n\
                 \x20       (local.set $rp (i64.extend_i32_u \
                              (call $str_concat \
                                (call $sval (local.get $at) (local.get $ap)) \
                                (call $sval (local.get $bt) (local.get $bp))))))\n\
                 \x20     (else (local.set $rt (i64.const 0))\n\
                 \x20       (local.set $rp (i64.add (local.get $ap) (local.get $bp)))))\n",
            );
            push(body, "(local.get $rt)", "(local.get $rp)");
            *depth -= 1;
        }
        "*" => {
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 2)) \
                                   (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20     (then (local.set $rt (i64.const 2))\n\
                 \x20       (local.set $rp (i64.extend_i32_u \
                              (call $str_repeat (i32.wrap_i64 (local.get $ap)) (local.get $bp)))))\n\
                 \x20     (else (local.set $rt (i64.const 0))\n\
                 \x20       (local.set $rp (i64.mul (local.get $ap) (local.get $bp)))))\n",
            );
            push(body, "(local.get $rt)", "(local.get $rp)");
            *depth -= 1;
        }
        "-" => {
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(body, "i64.const 0", "(i64.sub (local.get $ap) (local.get $bp))");
            *depth -= 1;
        }
        "%" => {
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(body, "i64.const 0", "(call $zmod (local.get $ap) (local.get $bp))");
            *depth -= 1;
        }
        "_" => {
            need(*depth, 1, op)?;
            body.push_str(POP_A);
            push(body, "i64.const 0", "(i64.sub (i64.const 0) (local.get $ap))");
        }
        "<" | ">" | "<=" | ">=" => {
            need(*depth, 2, op)?;
            let cmp = match op {
                "<" => "i64.lt_s",
                ">" => "i64.gt_s",
                "<=" => "i64.le_s",
                ">=" => "i64.ge_s",
                _ => unreachable!(),
            };
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(
                body,
                "i64.const 0",
                &format!("(i64.extend_i32_u ({cmp} (local.get $ap) (local.get $bp)))"),
            );
            *depth -= 1;
        }
        "==" | "!=" => {
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            // $rp <- (a == b), comparing strings by content and ints by value;
            // mismatched tags are unequal.
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 2)) \
                                   (i64.eq (local.get $bt) (i64.const 2)))\n\
                 \x20     (then (local.set $rp (i64.extend_i32_u \
                              (call $str_eq (i32.wrap_i64 (local.get $ap)) \
                                            (i32.wrap_i64 (local.get $bp))))))\n\
                 \x20     (else (if (i64.eq (local.get $at) (local.get $bt))\n\
                 \x20             (then (local.set $rp (i64.extend_i32_u \
                                  (i64.eq (local.get $ap) (local.get $bp)))))\n\
                 \x20             (else (local.set $rp (i64.const 0))))))\n",
            );
            let payload = if op == "==" {
                "(local.get $rp)".to_string()
            } else {
                "(i64.extend_i32_u (i64.eqz (local.get $rp)))".to_string()
            };
            push(body, "i64.const 0", &payload);
            *depth -= 1;
        }
        "&&" | "||" => {
            need(*depth, 2, op)?;
            let bit = if op == "&&" { "i32.and" } else { "i32.or" };
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(
                body,
                "i64.const 0",
                &format!(
                    "(i64.extend_i32_u ({bit} \
                     (call $truthy (local.get $at) (local.get $ap)) \
                     (call $truthy (local.get $bt) (local.get $bp))))"
                ),
            );
            *depth -= 1;
        }
        "!" => {
            need(*depth, 1, op)?;
            body.push_str(POP_A);
            push(
                body,
                "i64.const 0",
                "(i64.extend_i32_u (i32.eqz (call $truthy (local.get $at) (local.get $ap))))",
            );
        }
        "." => {
            need(*depth, 1, op)?;
            body.push_str(POP_A);
            body.push_str("    (call $print_value (local.get $at) (local.get $ap))\n");
            *depth -= 1;
        }
        "=" => {
            // dup
            need(*depth, 1, op)?;
            body.push_str(POP_B);
            push(body, "(local.get $bt)", "(local.get $bp)");
            push(body, "(local.get $bt)", "(local.get $bp)");
            *depth += 1;
        }
        "," => {
            // drop
            need(*depth, 1, op)?;
            body.push_str("    drop\n    drop\n");
            *depth -= 1;
        }
        "\\" => {
            // swap
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(body, "(local.get $bt)", "(local.get $bp)");
            push(body, "(local.get $at)", "(local.get $ap)");
        }
        "&" => {
            // over: a b -> a b a
            need(*depth, 2, op)?;
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(body, "(local.get $at)", "(local.get $ap)");
            push(body, "(local.get $bt)", "(local.get $bp)");
            push(body, "(local.get $at)", "(local.get $ap)");
            *depth += 1;
        }
        other => {
            return Err(format!(
                "wasm(M6b): operator `{other}` not supported yet \
                 (ints, strings, arithmetic/compare/logic, `.`, and stack ops `= , \\ &`)"
            ))
        }
    }
    Ok(())
}

fn emit_load(name: &str, body: &mut String, depth: &mut i64) -> Result<(), String> {
    match name {
        "tr" | "up" | "lo" | "rv" => {
            need(*depth, 1, name)?;
            body.push_str(POP_A);
            let call = match name {
                "tr" => "(call $str_trim (i32.wrap_i64 (local.get $ap)))".to_string(),
                "rv" => "(call $str_reverse (i32.wrap_i64 (local.get $ap)))".to_string(),
                "up" => "(call $str_case (i32.wrap_i64 (local.get $ap)) (i32.const 0))".to_string(),
                "lo" => "(call $str_case (i32.wrap_i64 (local.get $ap)) (i32.const 1))".to_string(),
                _ => unreachable!(),
            };
            push(body, "i64.const 2", &format!("(i64.extend_i32_u {call})"));
        }
        "ln" => {
            need(*depth, 1, name)?;
            body.push_str(POP_A);
            push(
                body,
                "i64.const 0",
                "(i64.extend_i32_u (i32.load (i32.wrap_i64 (local.get $ap))))",
            );
        }
        "rt" => {
            // rot: a b c -> b c a
            need(*depth, 3, name)?;
            body.push_str(POP_C);
            body.push_str(POP_B);
            body.push_str(POP_A);
            push(body, "(local.get $bt)", "(local.get $bp)");
            push(body, "(local.get $ct)", "(local.get $cp)");
            push(body, "(local.get $at)", "(local.get $ap)");
        }
        other => {
            return Err(format!(
                "wasm(M6b): word `{other}` not supported yet \
                 (this slice handles `tr up lo rv ln` and `rt`)"
            ))
        }
    }
    Ok(())
}

fn need(depth: i64, n: i64, op: &str) -> Result<(), String> {
    if depth < n {
        Err(format!(
            "wasm(M6b): `{op}` needs {n} value(s) but the compile-time stack depth is {depth}"
        ))
    } else {
        Ok(())
    }
}

fn module(pages: usize, heap_base: usize, data: &str, body: &str) -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") {pages})
  (global $hp (mut i32) (i32.const {heap_base}))

{data}
  ;; Bump-allocate `n` bytes (4-byte aligned), return the pointer.
  (func $alloc (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $hp))
    (global.set $hp
      (i32.and (i32.add (i32.add (local.get $p) (local.get $n)) (i32.const 3))
               (i32.const -4)))
    (local.get $p))

  ;; Write [ptr,len) to stdout via a single iovec at offset 0.
  (func $write (param $ptr i32) (param $len i32)
    (i32.store (i32.const 0) (local.get $ptr))
    (i32.store (i32.const 4) (local.get $len))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8))))

  (func $nl
    (i32.store8 (i32.const 16) (i32.const 10))
    (call $write (i32.const 16) (i32.const 1)))

  ;; Python-style modulo: result takes the sign of the divisor.
  (func $zmod (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (local.set $r (i64.rem_s (local.get $a) (local.get $b)))
    (if (result i64)
      (i32.and
        (i64.ne (local.get $r) (i64.const 0))
        (i32.ne (i64.lt_s (local.get $r) (i64.const 0))
                (i64.lt_s (local.get $b) (i64.const 0))))
      (then (i64.add (local.get $r) (local.get $b)))
      (else (local.get $r))))

  ;; Render an i64 in decimal into [.. ,64) and write it (no newline).
  (func $print_i64 (param $v i64)
    (local $idx i32) (local $neg i32) (local $n i64)
    (local.set $idx (i32.const 64))
    (local.set $n (local.get $v))
    (if (i64.lt_s (local.get $n) (i64.const 0))
      (then (local.set $neg (i32.const 1))
            (local.set $n (i64.sub (i64.const 0) (local.get $n)))))
    (loop $loop
      (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
      (i32.store8 (local.get $idx)
        (i32.add (i32.const 48)
          (i32.wrap_i64 (i64.rem_u (local.get $n) (i64.const 10)))))
      (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
      (br_if $loop (i64.gt_u (local.get $n) (i64.const 0))))
    (if (local.get $neg)
      (then (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
            (i32.store8 (local.get $idx) (i32.const 45))))
    (call $write (local.get $idx) (i32.sub (i32.const 64) (local.get $idx))))

  ;; Print a tagged value with a trailing newline (the `.` operator).
  (func $print_value (param $tag i64) (param $pl i64)
    (if (i64.eq (local.get $tag) (i64.const 2))
      (then (call $write
              (i32.add (i32.wrap_i64 (local.get $pl)) (i32.const 4))
              (i32.load (i32.wrap_i64 (local.get $pl)))))
      (else (call $print_i64 (local.get $pl))))
    (call $nl))

  ;; Coerce a tagged value to a string pointer (int -> decimal string).
  (func $sval (param $tag i64) (param $pl i64) (result i32)
    (if (result i32) (i64.eq (local.get $tag) (i64.const 2))
      (then (i32.wrap_i64 (local.get $pl)))
      (else (call $str_from_int (local.get $pl)))))

  ;; Truthiness: ints -> nonzero; strings -> nonempty.
  (func $truthy (param $tag i64) (param $pl i64) (result i32)
    (if (result i32) (i64.eq (local.get $tag) (i64.const 2))
      (then (i32.ne (i32.load (i32.wrap_i64 (local.get $pl))) (i32.const 0)))
      (else (i64.ne (local.get $pl) (i64.const 0)))))

  (func $str_from_int (param $v i64) (result i32)
    (local $idx i32) (local $neg i32) (local $n i64) (local $len i32) (local $p i32)
    (local.set $idx (i32.const 64))
    (local.set $n (local.get $v))
    (if (i64.lt_s (local.get $n) (i64.const 0))
      (then (local.set $neg (i32.const 1))
            (local.set $n (i64.sub (i64.const 0) (local.get $n)))))
    (loop $loop
      (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
      (i32.store8 (local.get $idx)
        (i32.add (i32.const 48)
          (i32.wrap_i64 (i64.rem_u (local.get $n) (i64.const 10)))))
      (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
      (br_if $loop (i64.gt_u (local.get $n) (i64.const 0))))
    (if (local.get $neg)
      (then (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
            (i32.store8 (local.get $idx) (i32.const 45))))
    (local.set $len (i32.sub (i32.const 64) (local.get $idx)))
    (local.set $p (call $alloc (i32.add (local.get $len) (i32.const 4))))
    (i32.store (local.get $p) (local.get $len))
    (memory.copy (i32.add (local.get $p) (i32.const 4)) (local.get $idx) (local.get $len))
    (local.get $p))

  (func $str_concat (param $a i32) (param $b i32) (result i32)
    (local $la i32) (local $lb i32) (local $p i32)
    (local.set $la (i32.load (local.get $a)))
    (local.set $lb (i32.load (local.get $b)))
    (local.set $p (call $alloc (i32.add (i32.add (local.get $la) (local.get $lb)) (i32.const 4))))
    (i32.store (local.get $p) (i32.add (local.get $la) (local.get $lb)))
    (memory.copy (i32.add (local.get $p) (i32.const 4))
                 (i32.add (local.get $a) (i32.const 4)) (local.get $la))
    (memory.copy (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $la))
                 (i32.add (local.get $b) (i32.const 4)) (local.get $lb))
    (local.get $p))

  (func $str_repeat (param $a i32) (param $n i64) (result i32)
    (local $la i32) (local $cnt i32) (local $total i32) (local $p i32) (local $i i32)
    (local.set $la (i32.load (local.get $a)))
    (local.set $cnt (i32.wrap_i64 (local.get $n)))
    (if (i32.lt_s (local.get $cnt) (i32.const 0)) (then (local.set $cnt (i32.const 0))))
    (local.set $total (i32.mul (local.get $la) (local.get $cnt)))
    (local.set $p (call $alloc (i32.add (local.get $total) (i32.const 4))))
    (i32.store (local.get $p) (local.get $total))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $i) (local.get $cnt)))
      (memory.copy
        (i32.add (i32.add (local.get $p) (i32.const 4)) (i32.mul (local.get $i) (local.get $la)))
        (i32.add (local.get $a) (i32.const 4)) (local.get $la))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $p))

  (func $str_eq (param $a i32) (param $b i32) (result i32)
    (local $la i32) (local $i i32)
    (local.set $la (i32.load (local.get $a)))
    (if (i32.ne (local.get $la) (i32.load (local.get $b))) (then (return (i32.const 0))))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $i) (local.get $la)))
      (if (i32.ne
            (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4)) (local.get $i)))
            (i32.load8_u (i32.add (i32.add (local.get $b) (i32.const 4)) (local.get $i))))
        (then (return (i32.const 0))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (i32.const 1))

  ;; ASCII case fold: lo=0 -> upper, lo=1 -> lower.
  (func $str_case (param $a i32) (param $lo i32) (result i32)
    (local $n i32) (local $p i32) (local $i i32) (local $c i32)
    (local.set $n (i32.load (local.get $a)))
    (local.set $p (call $alloc (i32.add (local.get $n) (i32.const 4))))
    (i32.store (local.get $p) (local.get $n))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $i) (local.get $n)))
      (local.set $c (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4)) (local.get $i))))
      (if (local.get $lo)
        (then (if (i32.and (i32.ge_u (local.get $c) (i32.const 65))
                           (i32.le_u (local.get $c) (i32.const 90)))
                (then (local.set $c (i32.add (local.get $c) (i32.const 32))))))
        (else (if (i32.and (i32.ge_u (local.get $c) (i32.const 97))
                           (i32.le_u (local.get $c) (i32.const 122)))
                (then (local.set $c (i32.sub (local.get $c) (i32.const 32)))))))
      (i32.store8 (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i)) (local.get $c))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $p))

  (func $str_reverse (param $a i32) (result i32)
    (local $n i32) (local $p i32) (local $i i32)
    (local.set $n (i32.load (local.get $a)))
    (local.set $p (call $alloc (i32.add (local.get $n) (i32.const 4))))
    (i32.store (local.get $p) (local.get $n))
    (local.set $i (i32.const 0))
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $i) (local.get $n)))
      (i32.store8 (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))
        (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4))
          (i32.sub (i32.sub (local.get $n) (i32.const 1)) (local.get $i)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (local.get $p))

  (func $is_ws (param $c i32) (result i32)
    (i32.or (i32.eq (local.get $c) (i32.const 32))
    (i32.or (i32.eq (local.get $c) (i32.const 9))
    (i32.or (i32.eq (local.get $c) (i32.const 10))
    (i32.or (i32.eq (local.get $c) (i32.const 13))
    (i32.or (i32.eq (local.get $c) (i32.const 12))
            (i32.eq (local.get $c) (i32.const 11))))))))

  (func $str_trim (param $a i32) (result i32)
    (local $n i32) (local $s i32) (local $e i32) (local $len i32) (local $p i32)
    (local.set $n (i32.load (local.get $a)))
    (local.set $s (i32.const 0))
    (block $d1 (loop $l1
      (br_if $d1 (i32.ge_s (local.get $s) (local.get $n)))
      (br_if $d1 (i32.eqz (call $is_ws
        (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4)) (local.get $s))))))
      (local.set $s (i32.add (local.get $s) (i32.const 1))) (br $l1)))
    (local.set $e (local.get $n))
    (block $d2 (loop $l2
      (br_if $d2 (i32.le_s (local.get $e) (local.get $s)))
      (br_if $d2 (i32.eqz (call $is_ws
        (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4))
          (i32.sub (local.get $e) (i32.const 1)))))))
      (local.set $e (i32.sub (local.get $e) (i32.const 1))) (br $l2)))
    (local.set $len (i32.sub (local.get $e) (local.get $s)))
    (local.set $p (call $alloc (i32.add (local.get $len) (i32.const 4))))
    (i32.store (local.get $p) (local.get $len))
    (memory.copy (i32.add (local.get $p) (i32.const 4))
                 (i32.add (i32.add (local.get $a) (i32.const 4)) (local.get $s))
                 (local.get $len))
    (local.get $p))

  (func $main
    (local $at i64) (local $ap i64)
    (local $bt i64) (local $bp i64)
    (local $ct i64) (local $cp i64)
    (local $rt i64) (local $rp i64)
{body}  )

  (func (export "_start") (call $main))
)
"#,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use zown_ir::IrBlock;

    fn prog(instrs: Vec<Instr>) -> IrProgram {
        IrProgram { blocks: vec![IrBlock(instrs)], main: 0 }
    }

    #[test]
    fn emits_for_int_add() {
        let wat = emit_wat(&prog(vec![
            Instr::ConstInt(2),
            Instr::ConstInt(3),
            Instr::Op("+".into()),
            Instr::Op(".".into()),
        ]))
        .unwrap();
        assert!(wat.contains("i64.const 2"));
        assert!(wat.contains("call $print_value"));
        assert!(wat.contains("$str_concat"));
    }

    #[test]
    fn emits_for_strings() {
        let wat = emit_wat(&prog(vec![
            Instr::ConstStr("foo".into()),
            Instr::ConstStr("bar".into()),
            Instr::Op("+".into()),
            Instr::Op(".".into()),
        ]))
        .unwrap();
        // length-prefixed literal: "foo" -> \03\00\00\00foo
        assert!(wat.contains("\\03\\00\\00\\00\\66\\6f\\6f"));
        assert!(wat.contains("(data (i32.const 1024)"));
    }

    #[test]
    fn emits_string_builtins() {
        let wat = emit_wat(&prog(vec![
            Instr::ConstStr("abc".into()),
            Instr::Load("up".into()),
            Instr::Op(".".into()),
        ]))
        .unwrap();
        assert!(wat.contains("$str_case"));
    }

    #[test]
    fn rejects_blocks() {
        let p = IrProgram {
            blocks: vec![IrBlock(vec![Instr::ConstInt(1)]), IrBlock(vec![Instr::ConstBlock(0)])],
            main: 1,
        };
        assert!(emit_wat(&p).is_err());
    }

    #[test]
    fn rejects_floats() {
        let err = emit_wat(&prog(vec![Instr::ConstFloat(1.5)])).unwrap_err();
        assert!(err.contains("float"));
    }
}
