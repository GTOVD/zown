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

mod v2;

use std::collections::HashMap;
use zown_ir::{Instr, IrProgram};

// Scratch below 1280 holds the runtime (fault text, big digits, cap table).
// Literals start at 1280; the operand stack, a save area, and the heap follow.
//   [0, 8)     iovec for fd_write (ptr@0, len@4)
//   16         newline byte
//   [44, 64)   itoa scratch (digits written backwards, ending at 64)
//   [80, 120)  float-format output buffer
//   [200, 240) u64->decimal reversal scratch
// Literals start at 1024; the operand stack and heap follow.
const LIT_BASE: usize = 1280;
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
    v2::intern_names(prog, &mut lits, &mut lit_order, &mut cursor);

    let mut vars: HashMap<String, usize> = HashMap::new();
    for block in &prog.blocks {
        for instr in &block.0 {
            if let Instr::Bind(name) = instr {
                let n = vars.len();
                vars.entry(name.clone()).or_insert(n);
            }
        }
    }

    let meta = v2::meta_image(prog, &lits);
    let meta_base = (cursor + 3) & !3;
    let stack_base = (meta_base + meta.len() + 15) & !15;
    let save_base = stack_base + STACK_SIZE;
    let heap_base = save_base + STACK_SIZE;
    let pages = ((heap_base + 1_048_576) + 65_535) / 65_536;

    let mut data = String::new();
    for s in &lit_order {
        data.push_str(&format!(
            "  (data (i32.const {}) \"{}\")\n",
            lits[s],
            encode_str(s)
        ));
    }
    if !meta.is_empty() {
        data.push_str(&format!(
            "  (data (i32.const {meta_base}) \"{}\")\n",
            encode_bytes(&meta)
        ));
    }

    let mut var_globals = String::new();
    for idx in 0..vars.len() {
        var_globals.push_str(&format!(
            "  (global $g{idx}t (mut i64) (i64.const 0))\n  (global $g{idx}p (mut i64) (i64.const 0))\n  (global $g{idx}b (mut i32) (i32.const 0))\n"
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
        meta_base,
        save_base,
        n,
        &elem,
        &var_globals,
        &data,
        &v2::runtime(),
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
    let mut raw = Vec::with_capacity(4 + bytes.len());
    raw.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    raw.extend_from_slice(bytes);
    encode_bytes(&raw)
}

fn encode_bytes(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        out.push_str(&format!("\\{b:02x}"));
    }
    out
}

// Pop the top operand into locals `$Xt` (tag) / `$Xp` (payload).
const POP_A: &str = "    call $pop\n    local.set $ap\n    local.set $at\n    (if (global.get $err) (then return))\n";
const POP_B: &str = "    call $pop\n    local.set $bp\n    local.set $bt\n    (if (global.get $err) (then return))\n";
const POP_C: &str = "    call $pop\n    local.set $cp\n    local.set $ct\n    (if (global.get $err) (then return))\n";

fn guard(body: &mut String) {
    body.push_str("    (if (global.get $err) (then return))\n");
}

/// Op ids shared with the blob at address 940 in `v2.wat`.
fn op_id(op: &str) -> i32 {
    match op {
        "+" => 0,
        "-" => 1,
        "*" => 2,
        "/" => 3,
        "%" => 4,
        "_" => 5,
        "pw" => 6,
        "ck" => 7,
        "sq" => 8,
        "rq" => 9,
        "ty" => 10,
        "??" => 11,
        "hd" => 12,
        "@" => 13,
        "ts" => 14,
        "pp" => 15,
        "at" => 16,
        "wr" => 17,
        "st" => 18,
        "b" => 19,
        "n" => 20,
        "fl" => 21,
        "ce" => 22,
        "rd" => 23,
        "gr" => 24,
        ";" => 25,
        "vi" => 26,
        "vf" => 27,
        "vd" => 28,
        "vu" => 29,
        "tl" => 30,
        "up" => 31,
        "." => 32,
        "," => 33,
        "=" => 34,
        "\\" => 35,
        "&" => 36,
        "?" => 37,
        "!" => 38,
        "ct" => 39,
        "rp" => 40,
        "s" => 41,
        "pr" => 42,
        "ab" => 43,
        "mx" => 44,
        "mn" => 45,
        "dp" => 46,
        "cl" => 47,
        "rt" => 48,
        "ln" => 49,
        "tr" => 50,
        "lo" => 51,
        "rv" => 52,
        "hv" => 53,
        "va" => 54,
        "vs" => 55,
        "vm" => 56,
        "vh" => 57,
        _ => 0,
    }
}

fn set_op(body: &mut String, op: &str) {
    body.push_str(&format!("    (global.set $opcur (i32.const {}))\n", op_id(op)));
}

fn fault_lit(body: &mut String, code: i32, lit: usize) {
    body.push_str(&format!(
        "    (call $fault_lit (i32.const {code}) (i32.const {lit}))\n"
    ));
}

/// Host words this backend does not lower. Compiling them is an explicit skip,
/// not a wrong answer.
fn refused(name: &str) -> bool {
    matches!(
        name,
        "bf" | "bg" | "bp"
            | "wd"
            | "h3" | "ni" | "ua"
            | "co" | "tx" | "bt" | "bh" | "pk" | "ky" | "an" | "gp" | "rw" | "cm" | "ly"
            | "gv" | "ge" | "gs" | "gl" | "gh" | "sb" | "so"
            | "sl" | "la" | "lp" | "lr"
            | "ch" | "vr" | "td" | "sp" | "se" | "nt" | "wn" | "ob" | "df" | "rm" | "lg"
            | "gd" | "fr" | "fw" | "mm" | "mo" | "ml" | "ac" | "ag" | "en" | "tm"
            | "mg" | "tc" | "tu" | "tq"
            | "ae" | "ad" | "es" | "ev" | "xk"
            | "op" | "oe" | "av" | "ve"
            | "iz" | "uz" | "dc" | "cx"
            | "ls" | "mp" | "sc" | "qu" | "rg" | "te" | "ti" | "bm" | "pq" | "bv"
            | "ps" | "po" | "gt" | "pt" | "hs" | "sk" | "px"
            | "cs" | "sw"
            | "hw" | "lc" | "rn" | "pm" | "pn" | "ws" | "sg"
    )
}

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
    guard(body);
    match instr {
        Instr::ConstInt(v) => push_val(body, "(i64.const 0)", &format!("(i64.const {v})")),
        Instr::ConstFloat(v) => push_val(
            body,
            "(i64.const 1)",
            &format!("(i64.const {})", v.to_bits() as i64),
        ),
        Instr::ConstStr(s) => push_val(body, "(i64.const 2)", &format!("(i64.const {})", lits[s])),
        Instr::ConstBlock(bid) => push_val(body, "(i64.const 3)", &format!("(i64.const {bid})")),
        Instr::Bind(name) => {
            let idx = vars[name];
            set_op(body, ":");
            body.push_str(&format!(
                "    call $pop\n    (if (global.get $err) (then return))\n    global.set $g{idx}p\n    global.set $g{idx}t\n    (global.set $g{idx}b (i32.const 1))\n"
            ));
        }
        Instr::Load(name) => emit_load(name, lits, vars, body)?,
        Instr::Op(s) => emit_op(s, body)?,
        Instr::Invoke => {
            set_op(body, "@");
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const 13)) (return)))\n\
                 \x20   (call $invoke (i32.wrap_i64 (local.get $ap)) (i32.const 13))\n",
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
            set_op(body, ";");
            body.push_str(POP_A); // body block id -> $ap
            body.push_str(POP_B); // cond block id -> $bp
            let k = *wc;
            *wc += 1;
            body.push_str(&format!(
                "    (block $done{k} (loop $loop{k}\n\
                 \x20     (if (global.get $err) (then return))\n\
                 \x20     (call_indirect (type $bt) (i32.wrap_i64 (local.get $bp)))\n\
                 \x20     (if (global.get $err) (then return))\n\
                 \x20     call $pop\n      local.set $rp\n      local.set $rt\n\
                 \x20     (if (global.get $err) (then return))\n\
                 \x20     (br_if $done{k} (i32.eqz (call $truthy (local.get $rt) (local.get $rp))))\n\
                 \x20     (call_indirect (type $bt) (i32.wrap_i64 (local.get $ap)))\n\
                 \x20     (if (global.get $err) (then return))\n\
                 \x20     (br $loop{k})))\n"
            ));
        }
        Instr::ConstCap(name) => {
            push_val(body, "(i64.const 5)", &format!("(i64.const {})", lits[name]));
        }
        Instr::Move(name) => {
            let lit = lits[name];
            if let Some(&idx) = vars.get(name) {
                body.push_str(&format!(
                    "    (if (global.get $g{idx}b)\n\
                     \x20     (then (call $push (global.get $g{idx}t) (global.get $g{idx}p))\n\
                     \x20           (global.set $g{idx}b (i32.const 0)))\n\
                     \x20     (else (call $fault_move (i32.const {lit}))))\n"
                ));
            } else {
                body.push_str(&format!("    (call $fault_move (i32.const {lit}))\n"));
            }
        }
        Instr::Match => {
            set_op(body, "??");
            body.push_str("    call $do_match\n");
        }
        Instr::Attempt => {
            set_op(body, "ty");
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const 10)) (return)))\n\
                 \x20   (call $do_ty (i32.wrap_i64 (local.get $ap)))\n",
            );
        }
    }
    Ok(())
}

fn emit_load(
    name: &str,
    lits: &HashMap<String, usize>,
    vars: &HashMap<String, usize>,
    body: &mut String,
) -> Result<(), String> {
    if let Some(&idx) = vars.get(name) {
        body.push_str(&format!(
            "    (if (global.get $g{idx}b)\n\
             \x20     (then (call $push (global.get $g{idx}t) (global.get $g{idx}p)))\n\
             \x20     (else\n"
        ));
        emit_word(name, lits, body)?;
        body.push_str("    ))\n");
        return Ok(());
    }
    emit_word(name, lits, body)
}

fn emit_word(name: &str, lits: &HashMap<String, usize>, body: &mut String) -> Result<(), String> {
    set_op(body, name);
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
        "sq" => body.push_str("    call $do_sq\n"),
        "pw" => body.push_str("    call $do_pow\n"),
        "b" => body.push_str("    call $do_big\n"),
        "hd" => body.push_str("    call $do_hd\n"),
        "tl" => body.push_str("    call $do_tl\n"),
        "hv" => body.push_str("    call $do_hv\n"),
        "rq" => body.push_str("    call $do_rq\n"),
        "at" => body.push_str("    call $vec_at\n"),
        "vh" => body.push_str("    call $vec_sum\n"),
        "va" => body.push_str("    (call $vec_binop (i32.const 0))\n"),
        "vs" => body.push_str("    (call $vec_binop (i32.const 1))\n"),
        "vm" => body.push_str("    (call $vec_binop (i32.const 2))\n"),
        "wr" => body.push_str("    (call $do_policy (i32.const 0))\n"),
        "st" => body.push_str("    (call $do_policy (i32.const 1))\n"),
        "ck" => body.push_str("    (call $do_policy (i32.const 2))\n"),
        "ts" => {
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const 14)) (return)))\n\
                 \x20   (call $do_ts (i32.wrap_i64 (local.get $ap)))\n",
            );
        }
        "pp" => {
            body.push_str(POP_A);
            body.push_str(POP_B);
            body.push_str(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const 15)) (return)))\n\
                 \x20   (if (i64.ne (local.get $bt) (i64.const 0))\n\
                 \x20     (then (call $fault (i32.const 8) (i32.const 15)) (return)))\n\
                 \x20   (if (i64.lt_s (local.get $bp) (i64.const 0))\n\
                 \x20     (then (call $fault (i32.const 9) (i32.const 15)) (return)))\n\
                 \x20   (call $do_pp (i32.wrap_i64 (local.get $ap)) (local.get $bp))\n",
            );
        }
        "gr" => {
            body.push_str(POP_A);
            body.push_str(POP_B);
            body.push_str(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const 24)) (return)))\n\
                 \x20   (if (i64.ne (local.get $bt) (i64.const 5))\n\
                 \x20     (then (call $fault (i32.const 8) (i32.const 24)) (return)))\n\
                 \x20   (call $do_gr (i32.wrap_i64 (local.get $ap)) (i32.wrap_i64 (local.get $bp)))\n",
            );
        }
        "ab" => {
            body.push_str(POP_A);
            body.push_str(
                "    (if (i64.eq (local.get $at) (i64.const 1))\n\
                 \x20     (then (call $push (i64.const 1) (i64.reinterpret_f64 \
                              (f64.abs (f64.reinterpret_i64 (local.get $ap))))))\n\
                 \x20     (else (call $push (i64.const 0) (select \
                              (call $ineg (local.get $ap)) (local.get $ap) \
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
            let mode = match name {
                "fl" => 0,
                "ce" => 1,
                "rd" => 2,
                _ => unreachable!(),
            };
            body.push_str(&format!("    (call $do_round (i32.const {mode}))\n"));
        }
        "i8" | "i16" | "i32" | "i64" | "ix" | "u8" | "u16" | "u32" | "u64" | "ux" => {
            let (signed, bits) = match name {
                "i8" => (1, 8),
                "i16" => (1, 16),
                "i32" => (1, 32),
                "i64" => (1, 64),
                "ix" => (1, 128),
                "u8" => (0, 8),
                "u16" => (0, 16),
                "u32" => (0, 32),
                "u64" => (0, 64),
                "ux" => (0, 128),
                _ => unreachable!(),
            };
            body.push_str(&format!(
                "    (call $push (i64.const 6) (call $width_pack (i32.const {signed}) (i32.const {bits})))\n"
            ));
        }
        "vi" | "vf" | "vd" | "vu" => {
            let (kind, count) = match name {
                "vi" => (0, 4),
                "vf" => (1, 4),
                "vd" => (2, 2),
                "vu" => (3, 16),
                _ => unreachable!(),
            };
            body.push_str(POP_A);
            body.push_str(&format!(
                "    (if (i64.ne (local.get $at) (i64.const 3))\n\
                 \x20     (then (call $fault (i32.const 13) (i32.const {})) (return)))\n\
                 \x20   (call $vec_new (i32.const {kind}) (i32.const {count}) (i32.wrap_i64 (local.get $ap)) (i32.const {}))\n",
                op_id(name),
                op_id(name)
            ));
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
        "cl" => body.push_str("    (global.set $sp (global.get $sbase))\n"),
        "ct" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 2)) \
                                   (i64.eq (local.get $bt) (i64.const 2)))\n\
                 \x20     (then (call $push (i64.const 2) (i64.extend_i32_u \
                              (call $str_concat (i32.wrap_i64 (local.get $ap)) \
                                                (i32.wrap_i64 (local.get $bp))))))\n\
                 \x20     (else unreachable))\n",
            );
        }
        "rp" => {
            body.push_str(POP_B);
            body.push_str(POP_A);
            body.push_str(
                "    (if (i32.and (i64.eq (local.get $at) (i64.const 2)) \
                                   (i64.eq (local.get $bt) (i64.const 0)))\n\
                 \x20     (then (call $push (i64.const 2) (i64.extend_i32_u \
                              (call $str_repeat (i32.wrap_i64 (local.get $ap)) (local.get $bp)))))\n\
                 \x20     (else unreachable))\n",
            );
        }
        "pr" => {
            body.push_str(POP_A);
            body.push_str("    (call $print_raw (local.get $at) (local.get $ap))\n");
        }
        other => {
            if refused(other) {
                return Err(format!(
                    "UNSUPPORTED: word `{other}` is not in the WASM backend (PLAN M22)"
                ));
            }
            let lit = lits.get(other).copied().ok_or_else(|| {
                format!("UNSUPPORTED: word `{other}` is not in the WASM backend (PLAN M22)")
            })?;
            fault_lit(body, 6, lit);
        }
    }
    Ok(())
}

fn emit_op(op: &str, body: &mut String) -> Result<(), String> {
    set_op(body, op);
    match op {
        "+" => body.push_str("    call $do_add\n"),
        "*" => body.push_str("    call $do_mul\n"),
        "-" => body.push_str("    call $do_sub\n"),
        "/" => body.push_str("    call $do_div\n"),
        "%" => body.push_str("    call $do_mod\n"),
        "_" => body.push_str("    call $do_neg\n"),
        "<" | ">" | "<=" | ">=" => {
            let (icmp, fcmp) = match op {
                "<" => ("i64.lt_s", "f64.lt"),
                ">" => ("i64.gt_s", "f64.gt"),
                "<=" => ("i64.le_s", "f64.le"),
                ">=" => ("i64.ge_s", "f64.ge"),
                _ => unreachable!(),
            };
            let which = match op {
                "<" => 0,
                ">" => 1,
                "<=" => 2,
                ">=" => 3,
                _ => unreachable!(),
            };
            let _ = (icmp, fcmp);
            body.push_str(&format!("    (call $do_rel (i32.const {which}))\n"));
        }
        "==" => body.push_str("    call $do_eq\n"),
        "!=" => body.push_str("    call $do_ne\n"),
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
        "|" | "^" => {
            return Err(format!(
                "UNSUPPORTED: operator `{op}` is not in the WASM backend (PLAN M22)"
            ));
        }
        other if other.starts_with('~') || matches!(other, "»" | "«" | "→") => {
            return Err(format!(
                "UNSUPPORTED: operator `{other}` is not in the WASM backend (PLAN M22)"
            ));
        }
        other => {
            return Err(format!(
                "UNSUPPORTED: operator `{other}` is not in the WASM backend (PLAN M22)"
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
    meta_base: usize,
    save_base: usize,
    table_n: usize,
    elem: &str,
    var_globals: &str,
    data: &str,
    runtime: &str,
    funcs: &str,
    main: usize,
) -> String {
    format!(
        r#"(module
  (type $bt (func))
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (import "wasi_snapshot_preview1" "proc_exit"
    (func $proc_exit (param i32)))
  (memory (export "memory") {pages})
  (table {table_n} funcref)
  (elem (i32.const 0){elem})
  (global $sp (mut i32) (i32.const {stack_base}))
  (global $sbase i32 (i32.const {stack_base}))
  (global $hp (mut i32) (i32.const {heap_base}))
  (global $err (mut i32) (i32.const 0))
  (global $catch (mut i32) (i32.const 0))
  (global $depth (mut i32) (i32.const 0))
  (global $ncaps (mut i32) (i32.const 0))
  (global $opcur (mut i32) (i32.const 0))
  (global $meta i32 (i32.const {meta_base}))
  (global $save i32 (i32.const {save_base}))
  (global $ssp (mut i32) (i32.const {save_base}))
{var_globals}
{data}
  ;; Operand stack in linear memory: 16 bytes/value ([i64 tag][i64 payload]).
  (func $push (param $t i64) (param $p i64)
    (i64.store (global.get $sp) (local.get $t))
    (i64.store offset=8 (global.get $sp) (local.get $p))
    (global.set $sp (i32.add (global.get $sp) (i32.const 16))))
  (func $pop (result i64 i64)
    (if (i32.le_u (global.get $sp) (global.get $sbase))
      (then (call $fault (i32.const 12) (global.get $opcur))
            (return (i64.const 0) (i64.const 0))))
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
      (else (if (result f64) (i64.eq (local.get $t) (i64.const 4))
        (then (call $big_f64 (i32.wrap_i64 (local.get $p))))
        (else (f64.convert_i64_s (local.get $p)))))))

  ;; Push a float result. Whole finite values collapse to int only inside i64.
  (func $push_num (param $v f64)
    (if (i32.and
          (i32.and (f64.eq (f64.sub (local.get $v) (local.get $v)) (f64.const 0))
                   (f64.eq (f64.floor (local.get $v)) (local.get $v)))
          (call $fits_f (local.get $v)))
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

  ;; Checked int arithmetic (SPEC §11). Overflow traps the module (`unreachable`)
  ;; rather than wrapping. v0.1 programs stay in range, so this does not change
  ;; their output; a `.zerr` packet for the trap is M9.
  (func $iadd (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (local.set $r (i64.add (local.get $a) (local.get $b)))
    (if (i32.and
          (i64.eq (i64.shr_s (local.get $a) (i64.const 63))
                  (i64.shr_s (local.get $b) (i64.const 63)))
          (i64.ne (i64.shr_s (local.get $r) (i64.const 63))
                  (i64.shr_s (local.get $a) (i64.const 63))))
      (then (call $fault (i32.const 1) (global.get $opcur)) (return (i64.const 0))))
    (local.get $r))
  (func $isub (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (local.set $r (i64.sub (local.get $a) (local.get $b)))
    (if (i32.and
          (i64.ne (i64.shr_s (local.get $a) (i64.const 63))
                  (i64.shr_s (local.get $b) (i64.const 63)))
          (i64.ne (i64.shr_s (local.get $r) (i64.const 63))
                  (i64.shr_s (local.get $a) (i64.const 63))))
      (then (call $fault (i32.const 1) (global.get $opcur)) (return (i64.const 0))))
    (local.get $r))
  (func $ineg (param $a i64) (result i64)
    (if (i64.eq (local.get $a) (i64.const 0x8000000000000000))
      (then (call $fault (i32.const 1) (global.get $opcur)) (return (i64.const 0))))
    (i64.sub (i64.const 0) (local.get $a)))
  (func $imul (param $a i64) (param $b i64) (result i64)
    (local $r i64)
    (if (i32.or (i64.eq (local.get $a) (i64.const 0))
                (i64.eq (local.get $b) (i64.const 0)))
      (then (return (i64.const 0))))
    (if (i32.or
          (i32.and (i64.eq (local.get $a) (i64.const 0x8000000000000000))
                   (i64.eq (local.get $b) (i64.const -1)))
          (i32.and (i64.eq (local.get $b) (i64.const 0x8000000000000000))
                   (i64.eq (local.get $a) (i64.const -1))))
      (then (call $fault (i32.const 1) (global.get $opcur)) (return (i64.const 0))))
    (local.set $r (i64.mul (local.get $a) (local.get $b)))
    (if (i64.ne (local.get $a) (i64.div_s (local.get $r) (local.get $b)))
      (then (call $fault (i32.const 1) (global.get $opcur)) (return (i64.const 0))))
    (local.get $r))

  (func $ipow (param $b i64) (param $e i64) (result i64)
    (local $r i64) (local $i i64)
    (local.set $r (i64.const 1))
    (block $d (loop $l
      (br_if $d (i64.ge_s (local.get $i) (local.get $e)))
      (local.set $r (call $imul (local.get $r) (local.get $b)))
      (if (global.get $err) (then (return (i64.const 0))))
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
    (if (i64.eq (local.get $v) (i64.const 0x8000000000000000))
      (then (call $write_byte (i32.const 45))
            (call $write (i32.const 720) (i32.const 19))
            (return)))
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

  (func $pow10 (param $e i32) (result f64)
    (local $p f64) (local $i i32) (local $n i32) (local $neg i32)
    (local.set $p (f64.const 1))
    (if (i32.lt_s (local.get $e) (i32.const 0))
      (then (local.set $neg (i32.const 1))
            (local.set $n (i32.sub (i32.const 0) (local.get $e))))
      (else (local.set $n (local.get $e))))
    (block $d (loop $l
      (br_if $d (i32.ge_s (local.get $i) (local.get $n)))
      (local.set $p (f64.mul (local.get $p) (f64.const 10)))
      (local.set $i (i32.add (local.get $i) (i32.const 1)))
      (br $l)))
    (if (result f64) (local.get $neg)
      (then (f64.div (f64.const 1) (local.get $p)))
      (else (local.get $p))))

  (func $fexp (param $v f64) (result i32)
    (local $e i32) (local $a f64)
    (local.set $a (f64.abs (local.get $v)))
    (if (f64.eq (local.get $a) (f64.const 0)) (then (return (i32.const 0))))
    (if (f64.ge (local.get $a) (f64.const 1))
      (then (block $d (loop $l
        (br_if $d (f64.lt (local.get $a) (f64.const 10)))
        (local.set $a (f64.div (local.get $a) (f64.const 10)))
        (local.set $e (i32.add (local.get $e) (i32.const 1)))
        (br $l))))
      (else (block $d2 (loop $l2
        (br_if $d2 (f64.ge (local.get $a) (f64.const 1)))
        (local.set $a (f64.mul (local.get $a) (f64.const 10)))
        (local.set $e (i32.sub (local.get $e) (i32.const 1)))
        (br $l2)))))
    (local.get $e))

  (func $fmt_sci (param $v f64) (param $dst i32) (param $len i32) (result i32)
    (local $e i32) (local $mant f64) (local $n i64) (local $nd i32) (local $i i32) (local $ed i32)
    (local.set $e (call $fexp (f64.abs (local.get $v))))
    (local.set $mant (f64.div (f64.abs (local.get $v)) (call $pow10 (local.get $e))))
    (local.set $n (i64.trunc_f64_s (f64.add (f64.mul (local.get $mant) (call $pow10 (i32.const 14))) (f64.const 0.5))))
    (if (i64.ge_u (local.get $n) (i64.const 1000000000000000))
      (then (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
            (local.set $e (i32.add (local.get $e) (i32.const 1)))))
    (local.set $nd (call $u64_to_buf (local.get $n) (i32.const 240)))
    (block $z (loop $zl
      (br_if $z (i32.le_s (local.get $nd) (i32.const 1)))
      (br_if $z (i32.ne (i32.load8_u (i32.add (i32.const 240) (i32.sub (local.get $nd) (i32.const 1)))) (i32.const 48)))
      (local.set $nd (i32.sub (local.get $nd) (i32.const 1)))
      (br $zl)))
    (i32.store8 (i32.add (local.get $dst) (local.get $len))
      (i32.load8_u (i32.const 240)))
    (local.set $len (i32.add (local.get $len) (i32.const 1)))
    (if (i32.gt_s (local.get $nd) (i32.const 1))
      (then
        (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 46))
        (local.set $len (i32.add (local.get $len) (i32.const 1)))
        (local.set $i (i32.const 1))
        (block $d (loop $l
          (br_if $d (i32.ge_s (local.get $i) (local.get $nd)))
          (i32.store8 (i32.add (local.get $dst) (local.get $len))
            (i32.load8_u (i32.add (i32.const 240) (local.get $i))))
          (local.set $len (i32.add (local.get $len) (i32.const 1)))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $l)))))
    (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 101))
    (local.set $len (i32.add (local.get $len) (i32.const 1)))
    (if (i32.lt_s (local.get $e) (i32.const 0))
      (then (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 45))
            (local.set $e (i32.sub (i32.const 0) (local.get $e))))
      (else (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 43))))
    (local.set $len (i32.add (local.get $len) (i32.const 1)))
    (local.set $ed (call $u64_to_buf (i64.extend_i32_u (local.get $e)) (i32.const 240)))
    (if (i32.eq (local.get $ed) (i32.const 1))
      (then (i32.store8 (i32.add (local.get $dst) (local.get $len)) (i32.const 48))
            (local.set $len (i32.add (local.get $len) (i32.const 1)))))
    (memory.copy (i32.add (local.get $dst) (local.get $len)) (i32.const 240) (local.get $ed))
    (i32.add (local.get $len) (local.get $ed)))

  ;; Format f64 at `dst`. Whole values that fit int print as ints; non-finite
  ;; values print inf/-inf/nan; large magnitudes use Python's scientific form.
  (func $fmt_f64 (param $v f64) (param $dst i32) (result i32)
    (local $len i32) (local $ip i64) (local $frac f64) (local $d i32) (local $cnt i32) (local $e i32)
    (if (f64.ne (local.get $v) (local.get $v))
      (then (i32.store8 (local.get $dst) (i32.const 110))
            (i32.store8 (i32.add (local.get $dst) (i32.const 1)) (i32.const 97))
            (i32.store8 (i32.add (local.get $dst) (i32.const 2)) (i32.const 110))
            (return (i32.const 3))))
    (if (f64.eq (local.get $v) (f64.const inf))
      (then (i32.store8 (local.get $dst) (i32.const 105))
            (i32.store8 (i32.add (local.get $dst) (i32.const 1)) (i32.const 110))
            (i32.store8 (i32.add (local.get $dst) (i32.const 2)) (i32.const 102))
            (return (i32.const 3))))
    (if (f64.eq (local.get $v) (f64.const -inf))
      (then (i32.store8 (local.get $dst) (i32.const 45))
            (i32.store8 (i32.add (local.get $dst) (i32.const 1)) (i32.const 105))
            (i32.store8 (i32.add (local.get $dst) (i32.const 2)) (i32.const 110))
            (i32.store8 (i32.add (local.get $dst) (i32.const 3)) (i32.const 102))
            (return (i32.const 4))))
    (if (f64.lt (local.get $v) (f64.const 0))
      (then (i32.store8 (local.get $dst) (i32.const 45))
            (local.set $len (i32.const 1))
            (local.set $v (f64.neg (local.get $v)))))
    (if (i32.and
          (i32.and (f64.eq (f64.floor (local.get $v)) (local.get $v))
                   (call $fits_f (local.get $v)))
          (i32.const 1))
      (then
        (if (f64.lt (local.get $v) (f64.const 0))
          (then (local.set $ip (i64.sub (i64.const 0) (i64.trunc_f64_s (local.get $v)))))
          (else (local.set $ip (i64.trunc_f64_s (local.get $v)))))
        (return (i32.add (local.get $len)
          (call $u64_to_buf (local.get $ip) (i32.add (local.get $dst) (local.get $len)))))))
    (local.set $e (call $fexp (local.get $v)))
    (if (i32.or (i32.ge_s (local.get $e) (i32.const 16)) (i32.le_s (local.get $e) (i32.const -5)))
      (then (return (call $fmt_sci (local.get $v) (local.get $dst) (local.get $len)))))
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
    (if (i64.ge_u (local.get $tag) (i64.const 4))
      (then (call $print_extra (local.get $tag) (local.get $pl)))
      (else (if (i64.eq (local.get $tag) (i64.const 2))
        (then (call $write
                (i32.add (i32.wrap_i64 (local.get $pl)) (i32.const 4))
                (i32.load (i32.wrap_i64 (local.get $pl)))))
        (else (if (i64.eq (local.get $tag) (i64.const 1))
                (then (call $print_f64 (f64.reinterpret_i64 (local.get $pl))))
                (else (call $print_i64 (local.get $pl)))))))))

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
          (then (call $block_true (i32.wrap_i64 (local.get $pl))))
          (else (if (result i32) (i64.eq (local.get $tag) (i64.const 4))
            (then (i32.eqz (call $is_zero_val (local.get $tag) (local.get $pl))))
            (else (if (result i32) (i32.or (i64.eq (local.get $tag) (i64.const 5))
                                           (i64.eq (local.get $tag) (i64.const 6)))
              (then (i32.const 1))
              (else (if (result i32) (i64.eq (local.get $tag) (i64.const 7))
                (then (i32.gt_u (i32.load offset=4 (i32.wrap_i64 (local.get $pl))) (i32.const 0)))
                (else (if (result i32) (i64.eq (local.get $tag) (i64.const 8))
                  (then (call $res_ok (i32.wrap_i64 (local.get $pl))))
                  (else (i64.ne (local.get $pl) (i64.const 0)))))))))))))))))

  ;; Parse a string (at ptr) to a number and push it (int unless it has a '.').
  (data (i32.const 220) "inf")
  (data (i32.const 224) "-inf")
  (data (i32.const 229) "nan")
  (data (i32.const 233) "+inf")
  (data (i32.const 238) "+nan")
  (data (i32.const 243) "-nan")

  (func $parse_num (param $p i32)
    (local $len i32) (local $i i32) (local $c i32) (local $isf i32)
    (local $sign f64) (local $val f64) (local $fdiv f64)
    (local $u i64) (local $neg i32) (local $digits i32)
    (local.set $len (i32.load (local.get $p)))
    (if (call $name_is (local.get $p) (i32.const 220) (i32.const 3))
      (then (call $push (i64.const 1) (i64.reinterpret_f64 (f64.const inf))) (return)))
    (if (i32.or (call $name_is (local.get $p) (i32.const 224) (i32.const 4))
                (call $name_is (local.get $p) (i32.const 233) (i32.const 4)))
      (then
        (if (call $name_is (local.get $p) (i32.const 224) (i32.const 4))
          (then (call $push (i64.const 1) (i64.reinterpret_f64 (f64.const -inf))))
          (else (call $push (i64.const 1) (i64.reinterpret_f64 (f64.const inf)))))
        (return)))
    (if (i32.or (call $name_is (local.get $p) (i32.const 229) (i32.const 3))
                (i32.or (call $name_is (local.get $p) (i32.const 238) (i32.const 4))
                        (call $name_is (local.get $p) (i32.const 243) (i32.const 4))))
      (then (call $push (i64.const 1) (i64.reinterpret_f64 (f64.const nan))) (return)))
    (block $ds (loop $ls
      (br_if $ds (i32.ge_s (local.get $i) (local.get $len)))
      (local.set $c (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))))
      (if (i32.eq (local.get $c) (i32.const 46)) (then (local.set $isf (i32.const 1))))
      (local.set $i (i32.add (local.get $i) (i32.const 1))) (br $ls)))
    (if (i32.eqz (local.get $isf))
      (then
        (local.set $i (i32.const 0))
        (if (i32.gt_s (local.get $len) (i32.const 0))
          (then
            (local.set $c (i32.load8_u (i32.add (local.get $p) (i32.const 4))))
            (if (i32.eq (local.get $c) (i32.const 45))
              (then (local.set $neg (i32.const 1)) (local.set $i (i32.const 1))))
            (if (i32.eq (local.get $c) (i32.const 43)) (then (local.set $i (i32.const 1))))))
        (block $di2 (loop $li2
          (br_if $di2 (i32.ge_s (local.get $i) (local.get $len)))
          (local.set $c (i32.load8_u (i32.add (i32.add (local.get $p) (i32.const 4)) (local.get $i))))
          (if (i32.or (i32.lt_u (local.get $c) (i32.const 48)) (i32.gt_u (local.get $c) (i32.const 57)))
            (then (call $fault (i32.const 8) (global.get $opcur)) (return)))
          (local.set $digits (i32.add (local.get $digits) (i32.const 1)))
          (if (i32.gt_s (local.get $digits) (i32.const 19))
            (then (call $fault (i32.const 1) (global.get $opcur)) (return)))
          (local.set $u (i64.add (i64.mul (local.get $u) (i64.const 10))
                                 (i64.extend_i32_u (i32.sub (local.get $c) (i32.const 48)))))
          (local.set $i (i32.add (local.get $i) (i32.const 1)))
          (br $li2)))
        (if (i32.eqz (local.get $digits))
          (then (call $fault (i32.const 8) (global.get $opcur)) (return)))
        (if (local.get $neg)
          (then
            (if (i64.gt_u (local.get $u) (i64.const 0x8000000000000000))
              (then (call $fault (i32.const 1) (global.get $opcur)) (return)))
            (call $push (i64.const 0) (i64.sub (i64.const 0) (local.get $u))))
          (else
            (if (i64.gt_u (local.get $u) (i64.const 9223372036854775807))
              (then (call $fault (i32.const 1) (global.get $opcur)) (return)))
            (call $push (i64.const 0) (local.get $u))))
        (return)))
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

{runtime}
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
        IrProgram {
            blocks: vec![IrBlock(instrs)],
            main: 0,
        }
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
        let wat = emit_wat(&prog(vec![
            Instr::ConstFloat(2.7),
            Instr::Load("fl".into()),
        ]))
        .unwrap();
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
