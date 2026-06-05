//! Zown WASM backend — lowers Zown IR to WebAssembly text (`.wat`).
//!
//! Milestone M6, slice **M6c**: blocks, control flow, and bindings.
//!
//! Values are tagged `(tag, payload)` pairs (as in M6b), but the operand stack
//! now lives in **linear memory** (global `$sp`) instead of the WASM value stack.
//! That is what lets `[ … ]` blocks — compiled to separate WASM functions and
//! invoked via `call_indirect` — all share one stack:
//!
//!   * tag `0` = int   — payload is the `i64` value
//!   * tag `1` = float — payload is the `f64` bit pattern   (reserved, M6d)
//!   * tag `2` = str   — payload is a pointer into linear memory
//!   * tag `3` = block — payload is the block's function-table index
//!
//! Each IR block becomes a `() -> ()` WASM function `$blkN`; the program's block
//! table is mirrored in a `funcref` table so `@` / `?` / `;` dispatch by index.
//! Bindings (`:name`) are a pair of mutable globals per distinct name, so the
//! environment is shared across block invocations like the interpreter's `env`.
//!
//! Supported: ints, strings, all value operators, `.`, stack ops, the string
//! words `tr/up/lo/rv/ln` and `rt`, blocks `[ … ]`, `@ ? ;`, and `:bind` / name
//! load. Floats and the math words (`/ sq pw fl ce …`) remain M6d. See
//! `docs/PLAN.md` and `docs/WASM.md`.

use std::collections::HashMap;
use zown_ir::{Instr, IrProgram};

// Memory map (byte offsets):
//   [0, 8)     iovec for fd_write (ptr@0, len@4)
//   16         scratch newline byte
//   [44, 64)   itoa scratch (digits written backwards, ending at 64)
//   [1024, ..) string-literal data segment
//   operand stack ($sp), 16 bytes per value, just past the literals
//   heap ($hp), bump allocator, just past the operand stack
const LIT_BASE: usize = 1024;
const STACK_SIZE: usize = 65_536; // 4096 operand slots

/// Lower an IR program to a `.wat` module string, or report the first
/// unsupported construct for this backend slice.
pub fn emit_wat(prog: &IrProgram) -> Result<String, String> {
    // String literals (across every block) -> data-segment offsets.
    let mut lits: HashMap<String, usize> = HashMap::new();
    let mut lit_order: Vec<String> = Vec::new();
    let mut cursor = LIT_BASE;
    for block in &prog.blocks {
        for instr in &block.0 {
            if let Instr::ConstStr(s) = instr {
                if !lits.contains_key(s) {
                    lits.insert(s.clone(), cursor);
                    lit_order.push(s.clone());
                    cursor = (cursor + 4 + s.as_bytes().len() + 3) & !3;
                }
            }
        }
    }

    // Distinct binding names (across every block) -> global slot indices.
    let mut vars: HashMap<String, usize> = HashMap::new();
    for block in &prog.blocks {
        for instr in &block.0 {
            if let Instr::Bind(name) = instr {
                let n = vars.len();
                vars.entry(name.clone()).or_insert(n);
            }
        }
    }

    let stack_base = (cursor + 15) & !15;
    let heap_base = stack_base + STACK_SIZE;
    let pages = ((heap_base + 1_048_576) + 65_535) / 65_536;

    let mut data = String::new();
    for s in &lit_order {
        data.push_str(&format!("  (data (i32.const {}) \"{}\")\n", lits[s], encode_str(s)));
    }

    let mut var_globals = String::new();
    for idx in 0..vars.len() {
        var_globals.push_str(&format!(
            "  (global $g{idx}t (mut i64) (i64.const 0))\n  (global $g{idx}p (mut i64) (i64.const 0))\n"
        ));
    }

    let n = prog.blocks.len();
    let mut elem = String::new();
    for i in 0..n {
        elem.push_str(&format!(" $blk{i}"));
    }

    let mut funcs = String::new();
    for (id, block) in prog.blocks.iter().enumerate() {
        funcs.push_str(&emit_block(id, block, &lits, &vars)?);
    }

    Ok(module(
        pages,
        stack_base,
        heap_base,
        n,
        &elem,
        &var_globals,
        &data,
        &funcs,
        prog.main,
    ))
}

/// Encode a string as `[i32 len LE][bytes]`, fully `\HH` escaped for `.wat`.
fn encode_str(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::new();
    for b in (bytes.len() as u32).to_le_bytes() {
        out.push_str(&format!("\\{b:02x}"));
    }
    for &b in bytes {
        out.push_str(&format!("\\{b:02x}"));
    }
    out
}

// Pop the top operand into locals `$Xt` (tag) / `$Xp` (payload).
const POP_A: &str = "    call $pop\n    local.set $ap\n    local.set $at\n";
const POP_B: &str = "    call $pop\n    local.set $bp\n    local.set $bt\n";
const POP_C: &str = "    call $pop\n    local.set $cp\n    local.set $ct\n";

fn push_val(body: &mut String, tag: &str, payload: &str) {
    body.push_str(&format!("    (call $push {tag} {payload})\n"));
}

fn emit_block(
    id: usize,
    block: &zown_ir::IrBlock,
    lits: &HashMap<String, usize>,
    vars: &HashMap<String, usize>,
) -> Result<String, String> {
    let mut body = String::new();
    let mut wc = 0usize;
    for instr in &block.0 {
        emit_instr(instr, lits, vars, &mut body, &mut wc)?;
    }
    Ok(format!(
        "  (func $blk{id}\n\
         \x20   (local $at i64) (local $ap i64) (local $bt i64) (local $bp i64)\n\
         \x20   (local $ct i64) (local $cp i64) (local $rt i64) (local $rp i64)\n\
         {body}  )\n"
    ))
}

fn emit_instr(
    instr: &Instr,
    lits: &HashMap<String, usize>,
    vars: &HashMap<String, usize>,
    body: &mut String,
    wc: &mut usize,
) -> Result<(), String> {
    match instr {
        Instr::ConstInt(v) => push_val(body, "(i64.const 0)", &format!("(i64.const {v})")),
        Instr::ConstStr(s) => {
            push_val(body, "(i64.const 2)", &format!("(i64.const {})", lits[s]))
        }
        Instr::ConstBlock(bid) => push_val(body, "(i64.const 3)", &format!("(i64.const {bid})")),
        Instr::ConstFloat(_) => {
            return Err("wasm(M6d): float literals need the float path".to_string())
        }
        Instr::Bind(name) => {
            let idx = vars[name];
            body.push_str(&format!(
                "    call $pop\n    global.set $g{idx}p\n    global.set $g{idx}t\n"
            ));
        }
        Instr::Load(name) => emit_load(name, vars, body)?,
        Instr::Op(s) => emit_op(s, body)?,
        Instr::Invoke => {
            body.push_str(POP_A);
            body.push_str(
                "    (call_indirect (type $bt) (i32.wrap_i64 (local.get $ap)))\n",
            );
        }
        Instr::Select => {
            body.push_str(POP_A); // else block
            body.push_str(POP_B); // then block
            body.push_str(POP_C); // condition
            push_val(
                body,
                "(i64.const 3)",
                "(select (local.get $bp) (local.get $ap) \
                 (call $truthy (local.get $ct) (local.get $cp)))",
            );
        }
        Instr::While => {
            body.push_str(POP_A); // body block id -> $ap
            body.push_str(POP_B); // cond block id -> $bp
            let k = *wc;
            *wc += 1;
            body.push_str(&format!(
                "    (block $done{k} (loop $loop{k}\n\
                 \x20     (call_indirect (type $bt) (i32.wrap_i64 (local.get $bp)))\n\
                 \x20     call $pop\n      local.set $rp\n      local.set $rt\n\
                 \x20     (br_if $done{k} (i32.eqz (call $truthy (local.get $rt) (local.get $rp))))\n\
                 \x20     (call_indirect (type $bt) (i32.wrap_i64 (local.get $ap)))\n\
                 \x20     (br $loop{k})))\n"
            ));
        }
    }
    Ok(())
}

fn emit_load(
    name: &str,
    vars: &HashMap<String, usize>,
    body: &mut String,
) -> Result<(), String> {
    // A user binding shadows builtins (the VM checks `env` first).
    if let Some(&idx) = vars.get(name) {
        push_val(
            body,
            &format!("(global.get $g{idx}t)"),
            &format!("(global.get $g{idx}p)"),
        );
        return Ok(());
    }
    match name {
        "tr" | "up" | "lo" | "rv" => {
            body.push_str(POP_A);
            let call = match name {
                "tr" => "(call $str_trim (i32.wrap_i64 (local.get $ap)))".to_string(),
                "rv" => "(call $str_reverse (i32.wrap_i64 (local.get $ap)))".to_string(),
                "up" => "(call $str_case (i32.wrap_i64 (local.get $ap)) (i32.const 0))".to_string(),
                "lo" => "(call $str_case (i32.wrap_i64 (local.get $ap)) (i32.const 1))".to_string(),
                _ => unreachable!(),
            };
            push_val(body, "(i64.const 2)", &format!("(i64.extend_i32_u {call})"));
        }
        "ln" => {
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 0)",
                "(i64.extend_i32_u (i32.load (i32.wrap_i64 (local.get $ap))))",
            );
        }
        "rt" => {
            body.push_str(POP_C); // top -> c
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $ct)", "(local.get $cp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        other => {
            return Err(format!(
                "wasm(M6c/M6d): word `{other}` not supported yet \
                 (this slice handles bindings, `tr up lo rv ln`, and `rt`; \
                 math words come in M6d)"
            ))
        }
    }
    Ok(())
}

fn emit_op(op: &str, body: &mut String) -> Result<(), String> {
    match op {
        "+" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.or (i64.eq (local.get $at) (i64.const 2)) \
                                  (i64.eq (local.get $bt) (i64.const 2)))\n\
                 \x20     (then (call $push (i64.const 2) (i64.extend_i32_u \
                              (call $str_concat \
                                (call $sval (local.get $at) (local.get $ap)) \
                                (call $sval (local.get $bt) (local.get $bp))))))\n\
                 \x20     (else (call $push (i64.const 0) \
                              (i64.add (local.get $ap) (local.get $bp)))))\n",
            );
        }
        "*" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 2)) \
                                   (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20     (then (call $push (i64.const 2) (i64.extend_i32_u \
                              (call $str_repeat (i32.wrap_i64 (local.get $ap)) (local.get $bp)))))\n\
                 \x20     (else (call $push (i64.const 0) \
                              (i64.mul (local.get $ap) (local.get $bp)))))\n",
            );
        }
        "-" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(i64.const 0)", "(i64.sub (local.get $ap) (local.get $bp))");
        }
        "%" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(i64.const 0)", "(call $zmod (local.get $ap) (local.get $bp))");
        }
        "_" => {
            body.push_str(POP_A);
            push_val(body, "(i64.const 0)", "(i64.sub (i64.const 0) (local.get $ap))");
        }
        "<" | ">" | "<=" | ">=" => {
            let cmp = match op {
                "<" => "i64.lt_s",
                ">" => "i64.gt_s",
                "<=" => "i64.le_s",
                ">=" => "i64.ge_s",
                _ => unreachable!(),
            };
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 0)",
                &format!("(i64.extend_i32_u ({cmp} (local.get $ap) (local.get $bp)))"),
            );
        }
        "==" | "!=" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
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
            push_val(body, "(i64.const 0)", &payload);
        }
        "&&" | "||" => {
            let bit = if op == "&&" { "i32.and" } else { "i32.or" };
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 0)",
                &format!(
                    "(i64.extend_i32_u ({bit} \
                     (call $truthy (local.get $at) (local.get $ap)) \
                     (call $truthy (local.get $bt) (local.get $bp))))"
                ),
            );
        }
        "!" => {
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 0)",
                "(i64.extend_i32_u (i32.eqz (call $truthy (local.get $at) (local.get $ap))))",
            );
        }
        "." => {
            body.push_str(POP_A);
            body.push_str("    (call $print_value (local.get $at) (local.get $ap))\n");
        }
        "=" => {
            // dup top
            body.push_str(POP_A);
            push_val(body, "(local.get $at)", "(local.get $ap)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        "," => body.push_str("    call $pop\n    drop\n    drop\n"),
        "\\" => {
            // swap
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        "&" => {
            // over: a b -> a b a
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $at)", "(local.get $ap)");
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        other => {
            return Err(format!(
                "wasm(M6d): operator `{other}` not supported yet (floats land in M6d)"
            ))
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn module(
    pages: usize,
    stack_base: usize,
    heap_base: usize,
    table_n: usize,
    elem: &str,
    var_globals: &str,
    data: &str,
    funcs: &str,
    main: usize,
) -> String {
    format!(
        r#"(module
  (type $bt (func))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") {pages})
  (table {table_n} funcref)
  (elem (i32.const 0){elem})
  (global $sp (mut i32) (i32.const {stack_base}))
  (global $hp (mut i32) (i32.const {heap_base}))
{var_globals}
{data}
  ;; Operand stack in linear memory: 16 bytes/value ([i64 tag][i64 payload]).
  (func $push (param $t i64) (param $p i64)
    (i64.store (global.get $sp) (local.get $t))
    (i64.store offset=8 (global.get $sp) (local.get $p))
    (global.set $sp (i32.add (global.get $sp) (i32.const 16))))
  (func $pop (result i64 i64)
    (global.set $sp (i32.sub (global.get $sp) (i32.const 16)))
    (i64.load (global.get $sp))
    (i64.load offset=8 (global.get $sp)))

  ;; Bump-allocate `n` bytes (4-byte aligned), return the pointer.
  (func $alloc (param $n i32) (result i32)
    (local $p i32)
    (local.set $p (global.get $hp))
    (global.set $hp
      (i32.and (i32.add (i32.add (local.get $p) (local.get $n)) (i32.const 3))
               (i32.const -4)))
    (local.get $p))

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

  ;; Truthiness: ints -> nonzero; strings -> nonempty; blocks -> true.
  (func $truthy (param $tag i64) (param $pl i64) (result i32)
    (if (result i32) (i64.eq (local.get $tag) (i64.const 2))
      (then (i32.ne (i32.load (i32.wrap_i64 (local.get $pl))) (i32.const 0)))
      (else (if (result i32) (i64.eq (local.get $tag) (i64.const 3))
              (then (i32.const 1))
              (else (i64.ne (local.get $pl) (i64.const 0)))))))

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

{funcs}
  (func (export "_start") (call $blk{main}))
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
        assert!(wat.contains("call $push"));
        assert!(wat.contains("call $print_value"));
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
        assert!(wat.contains("\\03\\00\\00\\00\\66\\6f\\6f"));
        assert!(wat.contains("(data (i32.const 1024)"));
    }

    #[test]
    fn emits_blocks_and_invoke() {
        // [ 1 ] @  -> block table + call_indirect
        let p = IrProgram {
            blocks: vec![
                IrBlock(vec![Instr::ConstInt(1)]),
                IrBlock(vec![Instr::ConstBlock(0), Instr::Invoke]),
            ],
            main: 1,
        };
        let wat = emit_wat(&p).unwrap();
        assert!(wat.contains("call_indirect"));
        assert!(wat.contains("$blk0"));
        assert!(wat.contains("(elem (i32.const 0) $blk0 $blk1)"));
    }

    #[test]
    fn emits_bindings() {
        // 5 :x x .
        let wat = emit_wat(&prog(vec![
            Instr::ConstInt(5),
            Instr::Bind("x".into()),
            Instr::Load("x".into()),
            Instr::Op(".".into()),
        ]))
        .unwrap();
        assert!(wat.contains("global $g0t"));
        assert!(wat.contains("global.set $g0p"));
    }

    #[test]
    fn rejects_floats() {
        let err = emit_wat(&prog(vec![Instr::ConstFloat(1.5)])).unwrap_err();
        assert!(err.contains("float"));
    }
}
