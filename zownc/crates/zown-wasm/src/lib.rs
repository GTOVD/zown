//! Zown WASM backend — lowers Zown IR to WebAssembly text (`.wat`) and binary
//! (`.wasm`).
//!
//! Milestone M6, slice **M6d**: floats, the math words, and binary emission —
//! completing the WASM backend. Every conformance case now compiles and runs
//! under `wasmtime`.
//!
//! Values are tagged `(tag, payload)` pairs on an in-memory operand stack
//! (global `$sp`, 16 bytes/value), so `[ … ]` blocks — compiled to separate
//! WASM functions invoked via `call_indirect` — share one stack:
//!
//!   * tag `0` = int   — payload is the `i64` value
//!   * tag `1` = float — payload is the `f64` **bit pattern**
//!   * tag `2` = str   — payload is a pointer into linear memory
//!   * tag `3` = block — payload is the block's function-table index
//!
//! Numeric operators promote int → float exactly like the interpreter; whole
//! floats render as integers (`7.0` → `7`), matching the oracle's `display`.
//!
//! Known limitation: float-to-decimal uses a terminating long-division
//! expansion, which is exact for dyadic values (the suite only prints `2.5`) but
//! is *not* the shortest round-tripping representation for values like `0.1`. A
//! Ryu/Grisu formatter is future work; see `docs/WASM.md`.

use std::collections::HashMap;
use zown_ir::{Instr, IrProgram};

// Scratch layout inside [0, 1024):
//   [0, 8)     iovec for fd_write (ptr@0, len@4)
//   16         newline byte
//   [44, 64)   itoa scratch (digits written backwards, ending at 64)
//   [80, 120)  float-format output buffer
//   [200, 240) u64->decimal reversal scratch
// Literals start at 1024; the operand stack and heap follow.
const LIT_BASE: usize = 1024;
const STACK_SIZE: usize = 65_536; // 4096 operand slots

/// Lower an IR program to a `.wat` module string, or report the first
/// unsupported construct.
pub fn emit_wat(prog: &IrProgram) -> Result<String, String> {
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

/// Lower an IR program straight to a binary `.wasm` module (assembles the `.wat`
/// then encodes it with the `wat` crate).
pub fn emit_wasm(prog: &IrProgram) -> Result<Vec<u8>, String> {
    let wat = emit_wat(prog)?;
    wat::parse_str(&wat).map_err(|e| format!("wasm encode: {e}"))
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
        Instr::ConstFloat(v) => {
            push_val(body, "(i64.const 1)", &format!("(i64.const {})", v.to_bits() as i64))
        }
        Instr::ConstStr(s) => {
            push_val(body, "(i64.const 2)", &format!("(i64.const {})", lits[s]))
        }
        Instr::ConstBlock(bid) => push_val(body, "(i64.const 3)", &format!("(i64.const {bid})")),
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
            body.push_str("    (call_indirect (type $bt) (i32.wrap_i64 (local.get $ap)))\n");
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

fn emit_load(name: &str, vars: &HashMap<String, usize>, body: &mut String) -> Result<(), String> {
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
            body.push_str(POP_C);
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $ct)", "(local.get $cp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        "sq" => {
            body.push_str(POP_A);
            body.push_str(
                "    (call $push_num (f64.sqrt (call $to_f (local.get $at) (local.get $ap))))\n",
            );
        }
        "pw" => {
            body.push_str(POP_B); // exponent
            body.push_str(POP_A); // base
            body.push_str(
                "    (if (i32.and (i32.and (i64.eq (local.get $at) (i64.const 0)) \
                                          (i64.eq (local.get $bt) (i64.const 0))) \
                                 (i64.ge_s (local.get $bp) (i64.const 0)))\n\
                 \x20     (then (call $push (i64.const 0) \
                              (call $ipow (local.get $ap) (local.get $bp))))\n\
                 \x20     (else (call $push_num (call $fpow \
                              (call $to_f (local.get $at) (local.get $ap)) \
                              (call $to_f (local.get $bt) (local.get $bp))))))\n",
            );
        }
        "ab" => {
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.eq (local.get $at) (i64.const 1))\n\
                 \x20     (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                              (f64.abs (f64.reinterpret_i64 (local.get $ap))))))\n\
                 \x20     (else (call $push (i64.const 0) (select \
                              (i64.sub (i64.const 0) (local.get $ap)) (local.get $ap) \
                              (i64.lt_s (local.get $ap) (i64.const 0))))))\n",
            );
        }
        "mx" | "mn" => {
            let cmp = if name == "mx" { "f64.ge" } else { "f64.le" };
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(&format!(
                "    (if ({cmp} (call $to_f (local.get $at) (local.get $ap)) \
                              (call $to_f (local.get $bt) (local.get $bp)))\n\
                 \x20     (then (call $push (local.get $at) (local.get $ap)))\n\
                 \x20     (else (call $push (local.get $bt) (local.get $bp))))\n"
            ));
        }
        "fl" | "ce" | "rd" => {
            let round = match name {
                "fl" => "f64.floor",
                "ce" => "f64.ceil",
                "rd" => "f64.nearest", // round half to even, matches Python round
                _ => unreachable!(),
            };
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 0)",
                &format!(
                    "(i64.trunc_f64_s ({round} (call $to_f (local.get $at) (local.get $ap))))"
                ),
            );
        }
        "s" => {
            body.push_str(POP_A);
            push_val(
                body,
                "(i64.const 2)",
                "(i64.extend_i32_u (call $sval (local.get $at) (local.get $ap)))",
            );
        }
        "n" => {
            body.push_str(POP_A);
            body.push_str(
                "    (if (call $is_num (local.get $at))\n\
                 \x20     (then (call $push (local.get $at) (local.get $ap)))\n\
                 \x20     (else (call $parse_num (i32.wrap_i64 (local.get $ap)))))\n",
            );
        }
        "dp" => push_val(
            body,
            "(i64.const 0)",
            "(i64.extend_i32_s (i32.div_s (i32.sub (global.get $sp) (global.get $sbase)) (i32.const 16)))",
        ),
        "clr" => body.push_str("    (global.set $sp (global.get $sbase))\n"),
        "pr" => {
            body.push_str(POP_A);
            body.push_str("    (call $print_raw (local.get $at) (local.get $ap))\n");
        }
        other => {
            return Err(format!("wasm: word `{other}` is not a known builtin"))
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
                 \x20     (else (if (i32.or (i64.eq (local.get $at) (i64.const 1)) \
                                            (i64.eq (local.get $bt) (i64.const 1)))\n\
                 \x20             (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                                      (f64.add (call $to_f (local.get $at) (local.get $ap)) \
                                               (call $to_f (local.get $bt) (local.get $bp))))))\n\
                 \x20             (else (call $push (i64.const 0) \
                                      (i64.add (local.get $ap) (local.get $bp)))))))\n",
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
                 \x20     (else (if (i32.or (i64.eq (local.get $at) (i64.const 1)) \
                                            (i64.eq (local.get $bt) (i64.const 1)))\n\
                 \x20             (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                                      (f64.mul (call $to_f (local.get $at) (local.get $ap)) \
                                               (call $to_f (local.get $bt) (local.get $bp))))))\n\
                 \x20             (else (call $push (i64.const 0) \
                                      (i64.mul (local.get $ap) (local.get $bp)))))))\n",
            );
        }
        "-" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.or (i64.eq (local.get $at) (i64.const 1)) \
                                  (i64.eq (local.get $bt) (i64.const 1)))\n\
                 \x20     (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                              (f64.sub (call $to_f (local.get $at) (local.get $ap)) \
                                       (call $to_f (local.get $bt) (local.get $bp))))))\n\
                 \x20     (else (call $push (i64.const 0) \
                              (i64.sub (local.get $ap) (local.get $bp)))))\n",
            );
        }
        "/" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (call $push_num (f64.div \
                     (call $to_f (local.get $at) (local.get $ap)) \
                     (call $to_f (local.get $bt) (local.get $bp))))\n",
            );
        }
        "%" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 0)) \
                                   (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20     (then (call $push (i64.const 0) \
                              (call $zmod (local.get $ap) (local.get $bp))))\n\
                 \x20     (else (call $push (i64.const 1) (i64.reinterpret_f64 \
                              (call $fmod (call $to_f (local.get $at) (local.get $ap)) \
                                          (call $to_f (local.get $bt) (local.get $bp)))))))\n",
            );
        }
        "_" => {
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.eq (local.get $at) (i64.const 1))\n\
                 \x20     (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                              (f64.neg (f64.reinterpret_i64 (local.get $ap))))))\n\
                 \x20     (else (call $push (i64.const 0) \
                              (i64.sub (i64.const 0) (local.get $ap)))))\n",
            );
        }
        "<" | ">" | "<=" | ">=" => {
            let (icmp, fcmp) = match op {
                "<" => ("i64.lt_s", "f64.lt"),
                ">" => ("i64.gt_s", "f64.gt"),
                "<=" => ("i64.le_s", "f64.le"),
                ">=" => ("i64.ge_s", "f64.ge"),
                _ => unreachable!(),
            };
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(&format!(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 0)) \
                                   (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20     (then (call $push (i64.const 0) (i64.extend_i32_u \
                              ({icmp} (local.get $ap) (local.get $bp)))))\n\
                 \x20     (else (call $push (i64.const 0) (i64.extend_i32_u \
                              ({fcmp} (call $to_f (local.get $at) (local.get $ap)) \
                                      (call $to_f (local.get $bt) (local.get $bp)))))))\n"
            ));
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
                 \x20     (else (if (i32.and (i64.eq (local.get $at) (i64.const 0)) \
                                             (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20             (then (local.set $rp (i64.extend_i32_u \
                                  (i64.eq (local.get $ap) (local.get $bp)))))\n\
                 \x20             (else (if (i32.and (call $is_num (local.get $at)) \
                                                     (call $is_num (local.get $bt)))\n\
 \x20                     (then (local.set $rp (i64.extend_i32_u \
                          (f64.eq (call $to_f (local.get $at) (local.get $ap)) \
                                  (call $to_f (local.get $bt) (local.get $bp))))))\n\
 \x20                     (else (local.set $rp (i64.const 0))))))))\n",
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
            body.push_str(POP_A);
            push_val(body, "(local.get $at)", "(local.get $ap)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        "," => body.push_str("    call $pop\n    drop\n    drop\n"),
        "\\" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        "&" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            push_val(body, "(local.get $at)", "(local.get $ap)");
            push_val(body, "(local.get $bt)", "(local.get $bp)");
            push_val(body, "(local.get $at)", "(local.get $ap)");
        }
        other => return Err(format!("wasm: operator `{other}` is not implemented")),
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
  (global $sbase i32 (i32.const {stack_base}))
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

  (func $is_num (param $t i64) (result i32) (i64.lt_u (local.get $t) (i64.const 2)))

  (func $to_f (param $t i64) (param $p i64) (result f64)
    (if (result f64) (i64.eq (local.get $t) (i64.const 1))
      (then (f64.reinterpret_i64 (local.get $p)))
      (else (f64.convert_i64_s (local.get $p)))))

  ;; Push a float result, collapsing whole-valued finite floats to int.
  (func $push_num (param $v f64)
    (if (i32.and (f64.eq (f64.floor (local.get $v)) (local.get $v))
                 (f64.eq (f64.sub (local.get $v) (local.get $v)) (f64.const 0)))
      (then (call $push (i64.const 0) (i64.trunc_f64_s (local.get $v))))
      (else (call $push (i64.const 1) (i64.reinterpret_f64 (local.get $v))))))

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

  (func $fmod (param $a f64) (param $b f64) (result f64)
    (f64.sub (local.get $a)
      (f64.mul (f64.floor (f64.div (local.get $a) (local.get $b))) (local.get $b))))

  (func $ipow (param $b i64) (param $e i64) (result i64)
    (local $r i64) (local $i i64)
    (local.set $r (i64.const 1))
    (block $d (loop $l
      (br_if $d (i64.ge_s (local.get $i) (local.get $e)))
      (local.set $r (i64.mul (local.get $r) (local.get $b)))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $l)))
    (local.get $r))

  (func $fpow (param $b f64) (param $e f64) (result f64)
    (local $r f64) (local $n i64) (local $i i64) (local $neg i32)
    (local.set $n (i64.trunc_f64_s (local.get $e)))
    (if (i64.lt_s (local.get $n) (i64.const 0))
      (then (local.set $neg (i32.const 1))
            (local.set $n (i64.sub (i64.const 0) (local.get $n)))))
    (local.set $r (f64.const 1))
    (block $d (loop $l
      (br_if $d (i64.ge_s (local.get $i) (local.get $n)))
      (local.set $r (f64.mul (local.get $r) (local.get $b)))
      (local.set $i (i64.add (local.get $i) (i64.const 1)))
      (br $l)))
    (if (result f64) (local.get $neg)
      (then (f64.div (f64.const 1) (local.get $r)))
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

  ;; Write `v`'s decimal digits forward at `dst`; return the count (v >= 0).
  (func $u64_to_buf (param $v i64) (param $dst i32) (result i32)
    (local $n i64) (local $cnt i32) (local $i i32)
    (if (i64.eqz (local.get $v))
      (then (i32.store8 (local.get $dst) (i32.const 48)) (return (i32.const 1))))
    (local.set $n (local.get $v))
    (block $d (loop $l
      (br_if $d (i64.eqz (local.get $n)))
      (i32.store8 (i32.add (i32.const 200) (local.get $cnt))
        (i32.add (i32.const 48) (i32.wrap_i64 (i64.rem_u (local.get $n) (i64.const 10)))))
      (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
      (local.set $cnt (i32.add (local.get $cnt) (i32.const 1)))
      (br $l)))
    (block $d2 (loop $l2
      (br_if $d2 (i32.ge_s (local.get $i) (local.get $cnt)))
      (i32.store8 (i32.add (local.get $dst) (local.get $i))
        (i32.load8_u (i32.add (i32.const 200)
          (i32.sub (i32.sub (local.get $cnt) (i32.const 1)) (local.get $i)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l2)))
    (local.get $cnt))

  ;; Format f64 at `dst` (display rules: whole -> integer); return byte count.
  ;; Terminating long division -> exact for dyadic values (e.g. 2.5).
  (func $fmt_f64 (param $v f64) (param $dst i32) (result i32)
    (local $len i32) (local $ip i64) (local $frac f64) (local $d i32) (local $cnt i32)
    (if (f64.lt (local.get $v) (f64.const 0))
      (then (i32.store8 (local.get $dst) (i32.const 45))
            (local.set $len (i32.const 1))
            (local.set $v (f64.neg (local.get $v)))))
    (local.set $ip (i64.trunc_f64_s (f64.floor (local.get $v))))
    (local.set $len
      (i32.add (local.get $len)
        (call $u64_to_buf (local.get $ip) (i32.add (local.get $dst) (local.get $len)))))
    (if (f64.eq (f64.floor (local.get $v)) (local.get $v))
      (then (return (local.get $len))))
    (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 46))
    (local.set $len (i32.add (local.get $len) (i32.const 1)))
    (local.set $frac (f64.sub (local.get $v) (f64.convert_i64_s (local.get $ip))))
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $cnt) (i32.const 17)))
      (br_if $done (f64.eq (local.get $frac) (f64.const 0)))
      (local.set $frac (f64.mul (local.get $frac) (f64.const 10)))
      (local.set $d (i32.trunc_f64_s (f64.floor (local.get $frac))))
      (i32.store8 (i32.add (local.get $dst) (local.get $len))
        (i32.add (i32.const 48) (local.get $d)))
      (local.set $len (i32.add (local.get $len) (i32.const 1)))
      (local.set $frac (f64.sub (local.get $frac) (f64.convert_i32_s (local.get $d))))
      (local.set $cnt (i32.add (local.get $cnt) (i32.const 1)))
      (br $l)))
    (local.get $len))

  (func $print_f64 (param $v f64)
    (call $write (i32.const 80) (call $fmt_f64 (local.get $v) (i32.const 80))))

  ;; Print a tagged value without a trailing newline.
  (func $print_raw (param $tag i64) (param $pl i64)
    (if (i64.eq (local.get $tag) (i64.const 2))
      (then (call $write
              (i32.add (i32.wrap_i64 (local.get $pl)) (i32.const 4))
              (i32.load (i32.wrap_i64 (local.get $pl)))))
      (else (if (i64.eq (local.get $tag) (i64.const 1))
              (then (call $print_f64 (f64.reinterpret_i64 (local.get $pl))))
              (else (call $print_i64 (local.get $pl)))))))

  ;; Print a tagged value with a trailing newline (the `.` operator).
  (func $print_value (param $tag i64) (param $pl i64)
    (call $print_raw (local.get $tag) (local.get $pl))
    (call $nl))

  ;; Coerce a tagged value to a string pointer (display rules).
  (func $sval (param $tag i64) (param $pl i64) (result i32)
    (if (result i32) (i64.eq (local.get $tag) (i64.const 2))
      (then (i32.wrap_i64 (local.get $pl)))
      (else (if (result i32) (i64.eq (local.get $tag) (i64.const 1))
              (then (call $str_from_float (f64.reinterpret_i64 (local.get $pl))))
              (else (call $str_from_int (local.get $pl)))))))

  ;; Truthiness: ints/floats -> nonzero; strings -> nonempty; blocks -> true.
  (func $truthy (param $tag i64) (param $pl i64) (result i32)
    (if (result i32) (i64.eq (local.get $tag) (i64.const 2))
      (then (i32.ne (i32.load (i32.wrap_i64 (local.get $pl))) (i32.const 0)))
      (else (if (result i32) (i64.eq (local.get $tag) (i64.const 1))
              (then (f64.ne (f64.reinterpret_i64 (local.get $pl)) (f64.const 0)))
              (else (if (result i32) (i64.eq (local.get $tag) (i64.const 3))
                      (then (i32.const 1))
                      (else (i64.ne (local.get $pl) (i64.const 0)))))))))

  ;; Parse a string (at ptr) to a number and push it (int unless it has a '.').
  (func $parse_num (param $p i32)
    (local $len i32) (local $i i32) (local $c i32) (local $isf i32)
    (local $sign f64) (local $val f64) (local $fdiv f64)
    (local.set $len (i32.load (local.get $p)))
    (block $ds (loop $ls
      (br_if $ds (i32.ge_s (local.get $i) (local.get $len)))
      (local.set $c (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))))
      (if (i32.eq (local.get $c) (i32.const 46)) (then (local.set $isf (i32.const 1))))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $ls)))
    (local.set $i (i32.const 0))
    (local.set $sign (f64.const 1))
    (if (i32.gt_s (local.get $len) (i32.const 0))
      (then
        (local.set $c (i32.load8_u (i32.add (local.get $p) (i32.const 4))))
        (if (i32.eq (local.get $c) (i32.const 45))
          (then (local.set $sign (f64.const -1)) (local.set $i (i32.const 1))))
        (if (i32.eq (local.get $c) (i32.const 43)) (then (local.set $i (i32.const 1))))))
    (local.set $val (f64.const 0))
    (block $di (loop $li
      (br_if $di (i32.ge_s (local.get $i) (local.get $len)))
      (local.set $c (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))))
      (br_if $di (i32.or (i32.lt_s (local.get $c) (i32.const 48)) (i32.gt_s (local.get $c) (i32.const 57))))
      (local.set $val (f64.add (f64.mul (local.get $val) (f64.const 10))
                               (f64.convert_i32_s (i32.sub (local.get $c) (i32.const 48)))))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $li)))
    (if (i32.and (i32.lt_s (local.get $i) (local.get $len))
                 (i32.eq (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))) (i32.const 46)))
      (then
        (local.set $i (i32.add (local.get $i) (i32.const 1)))
        (local.set $fdiv (f64.const 1))
        (block $df (loop $lf
          (br_if $df (i32.ge_s (local.get $i) (local.get $len)))
          (local.set $c (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))))
          (br_if $df (i32.or (i32.lt_s (local.get $c) (i32.const 48)) (i32.gt_s (local.get $c) (i32.const 57))))
          (local.set $val (f64.add (f64.mul (local.get $val) (f64.const 10))
                                   (f64.convert_i32_s (i32.sub (local.get $c) (i32.const 48)))))
          (local.set $fdiv (f64.mul (local.get $fdiv) (f64.const 10)))
          (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $lf)))
        (local.set $val (f64.div (local.get $val) (local.get $fdiv)))))
    (local.set $val (f64.mul (local.get $val) (local.get $sign)))
    (if (local.get $isf)
      (then (call $push (i64.const 1) (i64.reinterpret_f64 (local.get $val))))
      (else (call $push (i64.const 0) (i64.trunc_f64_s (local.get $val))))))

  (func $str_from_int (param $v i64) (result i32)
    (local $len i32) (local $p i32) (local $neg i32) (local $n i64) (local $off i32)
    (if (i64.lt_s (local.get $v) (i64.const 0))
      (then (local.set $neg (i32.const 1))
            (local.set $n (i64.sub (i64.const 0) (local.get $v))))
      (else (local.set $n (local.get $v))))
    (local.set $len (call $u64_to_buf (local.get $n) (i32.const 80)))
    (local.set $p (call $alloc (i32.add (i32.add (local.get $len) (local.get $neg)) (i32.const 4))))
    (i32.store (local.get $p) (i32.add (local.get $len) (local.get $neg)))
    (if (local.get $neg)
      (then (i32.store8 (i32.add (local.get $p) (i32.const 4)) (i32.const 45))
            (local.set $off (i32.const 1))))
    (memory.copy (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $off))
                 (i32.const 80) (local.get $len))
    (local.get $p))

  (func $str_from_float (param $v f64) (result i32)
    (local $len i32) (local $p i32)
    (local.set $len (call $fmt_f64 (local.get $v) (i32.const 80)))
    (local.set $p (call $alloc (i32.add (local.get $len) (i32.const 4))))
    (i32.store (local.get $p) (local.get $len))
    (memory.copy (i32.add (local.get $p) (i32.const 4)) (i32.const 80) (local.get $len))
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
    (block $done (loop $l
      (br_if $done (i32.ge_s (local.get $i) (local.get $la)))
      (if (i32.ne
            (i32.load8_u (i32.add (i32.add (local.get $a) (i32.const 4)) (local.get $i)))
            (i32.load8_u (i32.add (i32.add (local.get $b) (i32.const 4)) (local.get $i))))
        (then (return (i32.const 0))))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (i32.const 1))

  (func $str_case (param $a i32) (param $lo i32) (result i32)
    (local $n i32) (local $p i32) (local $i i32) (local $c i32)
    (local.set $n (i32.load (local.get $a)))
    (local.set $p (call $alloc (i32.add (local.get $n) (i32.const 4))))
    (i32.store (local.get $p) (local.get $n))
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
    fn emits_float_div() {
        let wat = emit_wat(&prog(vec![
            Instr::ConstInt(10),
            Instr::ConstInt(4),
            Instr::Op("/".into()),
            Instr::Op(".".into()),
        ]))
        .unwrap();
        assert!(wat.contains("$push_num"));
        assert!(wat.contains("f64.div"));
    }

    #[test]
    fn emits_float_literal() {
        // 2.7 -> tag 1 with the f64 bit pattern
        let bits = 2.7f64.to_bits() as i64;
        let wat = emit_wat(&prog(vec![Instr::ConstFloat(2.7), Instr::Load("fl".into())])).unwrap();
        assert!(wat.contains(&format!("(i64.const {bits})")));
        assert!(wat.contains("f64.floor"));
    }

    #[test]
    fn emits_math_words() {
        let wat = emit_wat(&prog(vec![Instr::ConstInt(9), Instr::Load("sq".into())])).unwrap();
        assert!(wat.contains("f64.sqrt"));
    }

    #[test]
    fn emits_binary_wasm() {
        let bytes = emit_wasm(&prog(vec![Instr::ConstInt(1), Instr::Op(".".into())])).unwrap();
        assert_eq!(&bytes[0..4], b"\0asm");
    }

    #[test]
    fn emits_blocks_and_invoke() {
        let p = IrProgram {
            blocks: vec![
                IrBlock(vec![Instr::ConstInt(1)]),
                IrBlock(vec![Instr::ConstBlock(0), Instr::Invoke]),
            ],
            main: 1,
        };
        let wat = emit_wat(&p).unwrap();
        assert!(wat.contains("call_indirect"));
        assert!(wat.contains("(elem (i32.const 0) $blk0 $blk1)"));
    }
}
