//! Zown WASM backend — lowers Zown IR to WebAssembly text (`.wat`).
//!
//! Milestone M6, slice **M6a**: the integer core. It compiles straight-line
//! integer programs — integer literals, `+ - * % _`, the comparisons and logic
//! ops (which yield `1`/`0`), and `.` printing — to a self-contained WASM module
//! that prints via WASI `fd_write`. The module runs directly in `wasmtime`.
//!
//! Because this subset has no blocks, bindings, strings, or floats, values map
//! 1:1 onto the WASM value stack as `i64`, so no in-memory tagged-value runtime
//! is needed yet. Unsupported instructions return a clear `Err` describing the
//! gap; later slices (M6b strings, M6c blocks/control, M6d floats) add the
//! tagged runtime. See `docs/PLAN.md` and `docs/WASM.md`.

use zown_ir::{Instr, IrProgram};

/// Lower an IR program to a `.wat` module string, or report the first
/// unsupported construct for this backend slice.
pub fn emit_wat(prog: &IrProgram) -> Result<String, String> {
    if prog.blocks.len() != 1 {
        return Err(
            "wasm(M6a): programs with `[ ... ]` blocks are not supported yet \
             (needs the tagged-value runtime + call_indirect, slice M6c)"
                .to_string(),
        );
    }
    let main = &prog.blocks[prog.main];

    let mut body = String::new();
    let mut depth: i64 = 0;

    for instr in &main.0 {
        emit_instr(instr, &mut body, &mut depth)?;
    }
    // Drop any values the program left on the stack so $main validates with no
    // result (mirrors the interpreter, which simply leaves them unused).
    for _ in 0..depth.max(0) {
        body.push_str("    drop\n");
    }

    Ok(module(&body))
}

fn emit_instr(instr: &Instr, body: &mut String, depth: &mut i64) -> Result<(), String> {
    match instr {
        Instr::ConstInt(v) => {
            body.push_str(&format!("    i64.const {v}\n"));
            *depth += 1;
        }
        Instr::Op(s) => emit_op(s, body, depth)?,
        other => {
            return Err(format!(
                "wasm(M6a): unsupported instruction `{other:?}` \
                 (this slice handles integers, arithmetic/compare/logic, and `.`)"
            ))
        }
    }
    Ok(())
}

fn emit_op(op: &str, body: &mut String, depth: &mut i64) -> Result<(), String> {
    // helpers that pop into locals $a (lower) and $b (top)
    let pop2 = "    local.set $b\n    local.set $a\n";
    match op {
        "+" | "-" | "*" => {
            need(*depth, 2, op)?;
            let instr = match op {
                "+" => "i64.add",
                "-" => "i64.sub",
                "*" => "i64.mul",
                _ => unreachable!(),
            };
            body.push_str(pop2);
            body.push_str(&format!("    ({instr} (local.get $a) (local.get $b))\n"));
            *depth -= 1;
        }
        "%" => {
            need(*depth, 2, op)?;
            body.push_str(pop2);
            body.push_str("    (call $zmod (local.get $a) (local.get $b))\n");
            *depth -= 1;
        }
        "_" => {
            need(*depth, 1, op)?;
            body.push_str("    local.set $a\n");
            body.push_str("    (i64.sub (i64.const 0) (local.get $a))\n");
        }
        "==" | "!=" | "<" | ">" | "<=" | ">=" => {
            need(*depth, 2, op)?;
            let cmp = match op {
                "==" => "i64.eq",
                "!=" => "i64.ne",
                "<" => "i64.lt_s",
                ">" => "i64.gt_s",
                "<=" => "i64.le_s",
                ">=" => "i64.ge_s",
                _ => unreachable!(),
            };
            body.push_str(pop2);
            body.push_str(&format!(
                "    (i64.extend_i32_u ({cmp} (local.get $a) (local.get $b)))\n"
            ));
            *depth -= 1;
        }
        "&&" | "||" => {
            need(*depth, 2, op)?;
            let bit = if op == "&&" { "i32.and" } else { "i32.or" };
            body.push_str(pop2);
            body.push_str(&format!(
                "    (i64.extend_i32_u ({bit} \
                 (i64.ne (local.get $a) (i64.const 0)) \
                 (i64.ne (local.get $b) (i64.const 0))))\n"
            ));
            *depth -= 1;
        }
        "!" => {
            need(*depth, 1, op)?;
            body.push_str("    local.set $a\n");
            body.push_str("    (i64.extend_i32_u (i64.eqz (local.get $a)))\n");
        }
        "." => {
            need(*depth, 1, op)?;
            body.push_str("    call $print_i64\n");
            *depth -= 1;
        }
        other => {
            return Err(format!(
                "wasm(M6a): operator `{other}` not supported yet \
                 (this slice handles integers, arithmetic/compare/logic, and `.`)"
            ))
        }
    }
    Ok(())
}

fn need(depth: i64, n: i64, op: &str) -> Result<(), String> {
    if depth < n {
        Err(format!(
            "wasm(M6a): `{op}` needs {n} value(s) but the compile-time stack depth is {depth}"
        ))
    } else {
        Ok(())
    }
}

fn module(body: &str) -> String {
    format!(
        r#"(module
  (import "wasi_snapshot_preview1" "fd_write"
    (func $fd_write (param i32 i32 i32 i32) (result i32)))
  (memory (export "memory") 1)

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

  ;; Print an i64 in decimal followed by a newline to stdout (fd 1).
  ;; Digits are written backwards into [.. ,64); the newline lives at byte 64.
  (func $print_i64 (param $v i64)
    (local $idx i32)
    (local $neg i32)
    (local $n i64)
    (i32.store8 (i32.const 64) (i32.const 10)) ;; '\n'
    (local.set $idx (i32.const 64))
    (local.set $n (local.get $v))
    (if (i64.lt_s (local.get $n) (i64.const 0))
      (then
        (local.set $neg (i32.const 1))
        (local.set $n (i64.sub (i64.const 0) (local.get $n)))))
    (loop $loop
      (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
      (i32.store8 (local.get $idx)
        (i32.add (i32.const 48)
          (i32.wrap_i64 (i64.rem_u (local.get $n) (i64.const 10)))))
      (local.set $n (i64.div_u (local.get $n) (i64.const 10)))
      (br_if $loop (i64.gt_u (local.get $n) (i64.const 0))))
    (if (local.get $neg)
      (then
        (local.set $idx (i32.sub (local.get $idx) (i32.const 1)))
        (i32.store8 (local.get $idx) (i32.const 45)))) ;; '-'
    ;; iovec at 0: ptr=idx, len=(65-idx)
    (i32.store (i32.const 0) (local.get $idx))
    (i32.store (i32.const 4) (i32.sub (i32.const 65) (local.get $idx)))
    (drop (call $fd_write (i32.const 1) (i32.const 0) (i32.const 1) (i32.const 8))))

  (func $main
    (local $a i64)
    (local $b i64)
{body}  )

  (func (export "_start") (call $main))
)
"#,
        body = body
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
        assert!(wat.contains("(i64.add (local.get $a) (local.get $b))"));
        assert!(wat.contains("call $print_i64"));
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
    fn rejects_strings() {
        let err = emit_wat(&prog(vec![Instr::ConstStr("x".into())])).unwrap_err();
        assert!(err.contains("unsupported"));
    }
}
