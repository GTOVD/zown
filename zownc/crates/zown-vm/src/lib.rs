//! Zown stack VM (Rust port of `zown/vm.py` + `zown/builtins.py`).
//!
//! A tree-walking interpreter over the AST. This is milestone M4 in
//! `docs/PLAN.md`: when `zownc run` matches the Python oracle byte-for-byte on
//! the conformance suite, today's language is fully reimplemented in Rust and
//! becomes the springboard for the IR + WASM/native backends.

mod error;
mod value;

use std::collections::HashMap;
use std::rc::Rc;

use zown_ast::Node;
use zown_parser::{parse, ParseError};

pub use error::*;
pub use value::Value;

/// A top-level diagnostic: a parse/lex error or a runtime error.
#[derive(Debug)]
pub enum Diag {
    Parse(ParseError),
    Run(RunError),
}

impl Diag {
    pub fn code(&self) -> &'static str {
        match self {
            Diag::Parse(e) => e.code,
            Diag::Run(e) => e.code,
        }
    }

    pub fn to_json(&self, file: Option<&str>) -> String {
        match self {
            Diag::Run(e) => e.to_json(),
            Diag::Parse(e) => {
                let f = match file {
                    Some(f) => value::json_str(f),
                    None => "null".into(),
                };
                format!(
                    "{{\n  \"zerr\": \"0.1\",\n  \"kind\": \"{kind}\",\n  \"code\": \"{code}\",\n  \"msg\": {msg},\n  \"op\": null,\n  \"pos\": {{\"line\": {l}, \"col\": {c}, \"offset\": {o}}},\n  \"stack\": [],\n  \"hint\": {hint},\n  \"file\": {file}\n}}",
                    kind = e.kind,
                    code = e.code,
                    msg = value::json_str(&e.msg),
                    l = e.pos.line,
                    c = e.pos.col,
                    o = e.pos.offset,
                    hint = value::json_str(e.hint),
                    file = f,
                )
            }
        }
    }

    pub fn render_human(&self, file: Option<&str>) -> String {
        match self {
            Diag::Run(e) => e.render_human(),
            Diag::Parse(e) => {
                let loc = file.unwrap_or("");
                let head = if loc.is_empty() {
                    format!("zerr[{}]", e.code)
                } else {
                    format!("zerr[{}] {}:{}:{}", e.code, loc, e.pos.line, e.pos.col)
                };
                let mut s = format!("{head} ({}): {}", e.kind, e.msg);
                if !e.hint.is_empty() {
                    s.push_str(&format!("\n  hint: {}", e.hint));
                }
                s
            }
        }
    }
}

pub struct Vm {
    pub stack: Vec<Value>,
    pub env: HashMap<String, Value>,
    pub out: String,
    pub file: Option<String>,
}

impl Vm {
    pub fn new(file: Option<String>) -> Self {
        Vm { stack: Vec::new(), env: HashMap::new(), out: String::new(), file }
    }

    /// Parse + run a source string.
    pub fn run_str(&mut self, src: &str) -> Result<(), Diag> {
        let program = parse(src).map_err(Diag::Parse)?;
        self.exec(&program).map_err(Diag::Run)
    }

    // --- stack helpers ---------------------------------------------------------
    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn err(&self, code: &'static str, msg: String, op: Option<String>, hint: &'static str) -> RunError {
        RunError {
            code,
            msg,
            op,
            hint,
            stack: self.stack.iter().map(|v| v.snapshot_json()).collect(),
            file: self.file.clone(),
        }
    }

    fn pop(&mut self, op: &str) -> Result<Value, RunError> {
        match self.stack.pop() {
            Some(v) => Ok(v),
            None => Err(self.err(
                STACK_UNDERFLOW,
                format!("`{op}` needs a value but the stack is empty"),
                Some(op.to_string()),
                "push a value before this operator",
            )),
        }
    }

    fn pop_num(&mut self, op: &str) -> Result<Value, RunError> {
        let v = self.pop(op)?;
        if matches!(v, Value::Int(_) | Value::Float(_)) {
            Ok(v)
        } else {
            let tn = v.type_name();
            Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a number, got {tn}"),
                Some(op.to_string()),
                "this operator only works on numbers",
            ))
        }
    }

    fn pop_block(&mut self, op: &str) -> Result<Rc<Vec<Node>>, RunError> {
        let v = self.pop(op)?;
        match v {
            Value::Block(b) => Ok(b),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    NOT_CALLABLE,
                    format!("`{op}` expected a block, got {tn}"),
                    Some(op.to_string()),
                    "wrap the code in [ ... ] to make a block",
                ))
            }
        }
    }

    // --- execution -------------------------------------------------------------
    fn exec(&mut self, nodes: &[Node]) -> Result<(), RunError> {
        for node in nodes {
            self.exec_node(node)?;
        }
        Ok(())
    }

    fn invoke(&mut self, blk: Rc<Vec<Node>>) -> Result<(), RunError> {
        self.exec(&blk)
    }

    fn exec_node(&mut self, node: &Node) -> Result<(), RunError> {
        match node {
            Node::Int(v) => self.push(Value::Int(*v)),
            Node::Float(v) => self.push(Value::Float(*v)),
            Node::Str(s) => self.push(Value::Str(s.clone())),
            Node::Blk(inner) => self.push(Value::Block(Rc::new(inner.clone()))),
            Node::Name(s) => self.exec_name(s)?,
            Node::Bind(s) => {
                let v = self.pop(&format!(":{s}"))?;
                self.env.insert(s.clone(), v);
            }
            Node::Op(s) => self.exec_op(s)?,
        }
        Ok(())
    }

    fn exec_name(&mut self, name: &str) -> Result<(), RunError> {
        if let Some(v) = self.env.get(name) {
            let v = v.clone();
            self.push(v);
            return Ok(());
        }
        if let Some(res) = self.try_builtin(name) {
            return res;
        }
        Err(self.err(
            NAME_UNRESOLVED,
            format!("`{name}` is not bound and is not a builtin"),
            Some(name.to_string()),
            "bind it with `:<name>` or check the stdlib word list",
        ))
    }

    fn exec_op(&mut self, op: &str) -> Result<(), RunError> {
        match op {
            "+" => self.op_add(),
            "-" => self.op_sub(),
            "*" => self.op_mul(),
            "/" => self.op_div(),
            "%" => self.op_mod(),
            "_" => self.op_neg(),
            "==" => self.op_eq(false),
            "!=" => self.op_eq(true),
            "<" => self.op_cmp("<"),
            ">" => self.op_cmp(">"),
            "<=" => self.op_cmp("<="),
            ">=" => self.op_cmp(">="),
            "&&" => self.op_and(),
            "||" => self.op_or(),
            "!" => self.op_not(),
            "=" => self.op_dup(),
            "," => self.op_drop(),
            "\\" => self.op_swap(),
            "&" => self.op_over(),
            "." => self.op_print(),
            "@" => self.op_invoke(),
            "?" => self.op_select(),
            ";" => self.op_while(),
            other => Err(self.err(
                UNSUPPORTED,
                format!("operator `{other}` is not implemented"),
                Some(other.to_string()),
                "",
            )),
        }
    }

    // --- arithmetic ------------------------------------------------------------
    fn op_add(&mut self) -> Result<(), RunError> {
        let b = self.pop("+")?;
        let a = self.pop("+")?;
        if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) {
            self.push(Value::Str(format!("{}{}", a.display(), b.display())));
            return Ok(());
        }
        let (af, bf, both_int, ai, bi) = self.nums2(&a, &b, "+")?;
        self.push(if both_int { Value::Int(ai + bi) } else { Value::Float(af + bf) });
        Ok(())
    }

    fn op_sub(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("-")?;
        let a = self.pop_num("-")?;
        self.push(arith(&a, &b, |x, y| x - y, |x, y| x - y));
        Ok(())
    }

    fn op_mul(&mut self) -> Result<(), RunError> {
        let b = self.pop("*")?;
        let a = self.pop("*")?;
        if let (Value::Str(s), Value::Int(n)) = (&a, &b) {
            self.push(Value::Str(if *n > 0 { s.repeat(*n as usize) } else { String::new() }));
            return Ok(());
        }
        self.require_num(&a, "*")?;
        self.require_num(&b, "*")?;
        self.push(arith(&a, &b, |x, y| x * y, |x, y| x * y));
        Ok(())
    }

    fn op_div(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("/")?;
        let a = self.pop_num("/")?;
        if b.as_f64() == Some(0.0) {
            return Err(self.err(DIV_ZERO, "division by zero".into(), Some("/".into()),
                "guard the denominator with `=0 ==` before dividing"));
        }
        self.push(collapse(a.as_f64().unwrap() / b.as_f64().unwrap()));
        Ok(())
    }

    fn op_mod(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("%")?;
        let a = self.pop_num("%")?;
        if b.as_f64() == Some(0.0) {
            return Err(self.err(DIV_ZERO, "modulo by zero".into(), Some("%".into()), ""));
        }
        self.push(match (&a, &b) {
            (Value::Int(x), Value::Int(y)) => Value::Int(py_mod_i64(*x, *y)),
            _ => Value::Float(py_mod_f64(a.as_f64().unwrap(), b.as_f64().unwrap())),
        });
        Ok(())
    }

    fn op_neg(&mut self) -> Result<(), RunError> {
        let a = self.pop_num("_")?;
        self.push(match a {
            Value::Int(v) => Value::Int(-v),
            Value::Float(v) => Value::Float(-v),
            _ => unreachable!(),
        });
        Ok(())
    }

    // --- comparison / logic ----------------------------------------------------
    fn op_eq(&mut self, negate: bool) -> Result<(), RunError> {
        let op = if negate { "!=" } else { "==" };
        let b = self.pop(op)?;
        let a = self.pop(op)?;
        let eq = a.zown_eq(&b);
        self.push(Value::Int(if eq != negate { 1 } else { 0 }));
        Ok(())
    }

    fn op_cmp(&mut self, op: &str) -> Result<(), RunError> {
        let b = self.pop(op)?;
        let a = self.pop(op)?;
        let ord = match (&a, &b) {
            (Value::Str(x), Value::Str(y)) => x.partial_cmp(y),
            _ => match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => x.partial_cmp(&y),
                _ => {
                    return Err(self.err(
                        TYPE_MISMATCH,
                        format!("`{op}` cannot compare {} and {}", a.type_name(), b.type_name()),
                        Some(op.to_string()),
                        "",
                    ))
                }
            },
        };
        let res = match ord {
            Some(o) => match op {
                "<" => o.is_lt(),
                ">" => o.is_gt(),
                "<=" => o.is_le(),
                ">=" => o.is_ge(),
                _ => unreachable!(),
            },
            None => false,
        };
        self.push(Value::Int(if res { 1 } else { 0 }));
        Ok(())
    }

    fn op_and(&mut self) -> Result<(), RunError> {
        let b = self.pop("&&")?;
        let a = self.pop("&&")?;
        self.push(Value::Int(if a.truthy() && b.truthy() { 1 } else { 0 }));
        Ok(())
    }

    fn op_or(&mut self) -> Result<(), RunError> {
        let b = self.pop("||")?;
        let a = self.pop("||")?;
        self.push(Value::Int(if a.truthy() || b.truthy() { 1 } else { 0 }));
        Ok(())
    }

    fn op_not(&mut self) -> Result<(), RunError> {
        let a = self.pop("!")?;
        self.push(Value::Int(if a.truthy() { 0 } else { 1 }));
        Ok(())
    }

    // --- stack -----------------------------------------------------------------
    fn op_dup(&mut self) -> Result<(), RunError> {
        match self.stack.last() {
            Some(v) => {
                let v = v.clone();
                self.push(v);
                Ok(())
            }
            None => Err(self.err(STACK_UNDERFLOW, "`=` (dup) needs a value".into(), Some("=".into()), "")),
        }
    }

    fn op_drop(&mut self) -> Result<(), RunError> {
        self.pop(",")?;
        Ok(())
    }

    fn op_swap(&mut self) -> Result<(), RunError> {
        let b = self.pop("\\")?;
        let a = self.pop("\\")?;
        self.push(b);
        self.push(a);
        Ok(())
    }

    fn op_over(&mut self) -> Result<(), RunError> {
        if self.stack.len() < 2 {
            return Err(self.err(STACK_UNDERFLOW, "`&` (over) needs two values".into(), Some("&".into()), ""));
        }
        let v = self.stack[self.stack.len() - 2].clone();
        self.push(v);
        Ok(())
    }

    // --- io / control ----------------------------------------------------------
    fn op_print(&mut self) -> Result<(), RunError> {
        let v = self.pop(".")?;
        self.out.push_str(&v.display());
        self.out.push('\n');
        Ok(())
    }

    fn op_invoke(&mut self) -> Result<(), RunError> {
        let b = self.pop_block("@")?;
        self.invoke(b)
    }

    fn op_select(&mut self) -> Result<(), RunError> {
        let els = self.pop_block("?")?;
        let then = self.pop_block("?")?;
        let cond = self.pop("?")?;
        self.push(Value::Block(if cond.truthy() { then } else { els }));
        Ok(())
    }

    fn op_while(&mut self) -> Result<(), RunError> {
        let body = self.pop_block(";")?;
        let cond = self.pop_block(";")?;
        loop {
            self.invoke(cond.clone())?;
            if !self.pop(";")?.truthy() {
                break;
            }
            self.invoke(body.clone())?;
        }
        Ok(())
    }

    // --- helpers ---------------------------------------------------------------
    fn require_num(&self, v: &Value, op: &str) -> Result<(), RunError> {
        if matches!(v, Value::Int(_) | Value::Float(_)) {
            Ok(())
        } else {
            Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected numbers, got {}", v.type_name()),
                Some(op.to_string()),
                "",
            ))
        }
    }

    fn nums2(&self, a: &Value, b: &Value, op: &str) -> Result<(f64, f64, bool, i64, i64), RunError> {
        self.require_num(a, op)?;
        self.require_num(b, op)?;
        let both_int = matches!((a, b), (Value::Int(_), Value::Int(_)));
        let (ai, bi) = (
            if let Value::Int(x) = a { *x } else { 0 },
            if let Value::Int(x) = b { *x } else { 0 },
        );
        Ok((a.as_f64().unwrap(), b.as_f64().unwrap(), both_int, ai, bi))
    }

    // --- builtins (stdlib WORDS) ----------------------------------------------
    fn try_builtin(&mut self, name: &str) -> Option<Result<(), RunError>> {
        match name {
            "ln" => Some(self.b_len()),
            "tr" => Some(self.b_str_map("tr", |s| s.trim().to_string())),
            "up" => Some(self.b_str_map("up", |s| s.to_uppercase())),
            "lo" => Some(self.b_str_map("lo", |s| s.to_lowercase())),
            "rv" => Some(self.b_str_map("rv", |s| s.chars().rev().collect())),
            "pr" => Some(self.b_print_raw()),
            "ab" => Some(self.b_abs()),
            "mx" => Some(self.b_minmax(true)),
            "mn" => Some(self.b_minmax(false)),
            "sq" => Some(self.b_sqrt()),
            "pw" => Some(self.b_pow()),
            "fl" => Some(self.b_round_kind("fl")),
            "ce" => Some(self.b_round_kind("ce")),
            "rd" => Some(self.b_round_kind("rd")),
            "s" => Some(self.b_to_str()),
            "n" => Some(self.b_to_num()),
            "dp" => Some(self.b_depth()),
            "rt" => Some(self.b_rot()),
            "clr" => Some(self.b_clear()),
            _ => None,
        }
    }

    fn b_len(&mut self) -> Result<(), RunError> {
        let v = self.pop("ln")?;
        match v {
            Value::Str(s) => self.push(Value::Int(s.chars().count() as i64)),
            Value::Block(b) => self.push(Value::Int(b.len() as i64)),
            other => {
                let tn = other.type_name();
                return Err(self.err(TYPE_MISMATCH,
                    format!("`ln` expected a string or block, got {tn}"), Some("ln".into()), ""));
            }
        }
        Ok(())
    }

    fn b_str_map(&mut self, op: &str, f: impl Fn(&str) -> String) -> Result<(), RunError> {
        let v = self.pop(op)?;
        match v {
            Value::Str(s) => {
                self.push(Value::Str(f(&s)));
                Ok(())
            }
            other => {
                let tn = other.type_name();
                Err(self.err(TYPE_MISMATCH, format!("`{op}` expected a string, got {tn}"),
                    Some(op.to_string()), ""))
            }
        }
    }

    fn b_print_raw(&mut self) -> Result<(), RunError> {
        let v = self.pop("pr")?;
        self.out.push_str(&v.display());
        Ok(())
    }

    fn b_abs(&mut self) -> Result<(), RunError> {
        let v = self.pop_num("ab")?;
        self.push(match v {
            Value::Int(x) => Value::Int(x.abs()),
            Value::Float(x) => Value::Float(x.abs()),
            _ => unreachable!(),
        });
        Ok(())
    }

    fn b_minmax(&mut self, want_max: bool) -> Result<(), RunError> {
        let op = if want_max { "mx" } else { "mn" };
        let b = self.pop_num(op)?;
        let a = self.pop_num(op)?;
        let take_a = if want_max {
            a.as_f64().unwrap() >= b.as_f64().unwrap()
        } else {
            a.as_f64().unwrap() <= b.as_f64().unwrap()
        };
        self.push(if take_a { a } else { b });
        Ok(())
    }

    fn b_sqrt(&mut self) -> Result<(), RunError> {
        let v = self.pop_num("sq")?;
        self.push(collapse(v.as_f64().unwrap().sqrt()));
        Ok(())
    }

    fn b_pow(&mut self) -> Result<(), RunError> {
        let e = self.pop_num("pw")?;
        let a = self.pop_num("pw")?;
        if let (Value::Int(base), Value::Int(exp)) = (&a, &e) {
            if *exp >= 0 {
                self.push(Value::Int(base.pow(*exp as u32)));
                return Ok(());
            }
        }
        self.push(collapse(a.as_f64().unwrap().powf(e.as_f64().unwrap())));
        Ok(())
    }

    fn b_round_kind(&mut self, op: &str) -> Result<(), RunError> {
        let v = self.pop_num(op)?.as_f64().unwrap();
        let r = match op {
            "fl" => v.floor() as i64,
            "ce" => v.ceil() as i64,
            "rd" => round_half_even(v),
            _ => unreachable!(),
        };
        self.push(Value::Int(r));
        Ok(())
    }

    fn b_to_str(&mut self) -> Result<(), RunError> {
        let v = self.pop("s")?;
        self.push(Value::Str(v.display()));
        Ok(())
    }

    fn b_to_num(&mut self) -> Result<(), RunError> {
        let v = self.pop("n")?;
        match v {
            Value::Int(_) | Value::Float(_) => self.push(v),
            Value::Str(s) => {
                let t = s.trim();
                let is_float = t.contains('.') || t.to_lowercase().contains('e');
                let parsed = if is_float {
                    t.parse::<f64>().ok().map(Value::Float)
                } else {
                    t.parse::<i64>().ok().map(Value::Int)
                };
                match parsed {
                    Some(val) => self.push(val),
                    None => {
                        return Err(self.err(TYPE_MISMATCH,
                            format!("`n` cannot parse {s:?} as a number"), Some("n".into()), ""))
                    }
                }
            }
            other => {
                let tn = other.type_name();
                return Err(self.err(TYPE_MISMATCH,
                    format!("`n` expected a string or number, got {tn}"), Some("n".into()), ""));
            }
        }
        Ok(())
    }

    fn b_depth(&mut self) -> Result<(), RunError> {
        self.push(Value::Int(self.stack.len() as i64));
        Ok(())
    }

    fn b_rot(&mut self) -> Result<(), RunError> {
        if self.stack.len() < 3 {
            return Err(self.err(STACK_UNDERFLOW, "`rt` (rot) needs three values".into(),
                Some("rt".into()), ""));
        }
        let idx = self.stack.len() - 3;
        let v = self.stack.remove(idx);
        self.push(v);
        Ok(())
    }

    fn b_clear(&mut self) -> Result<(), RunError> {
        self.stack.clear();
        Ok(())
    }
}

// --- free numeric helpers ------------------------------------------------------
fn arith(a: &Value, b: &Value, fi: impl Fn(i64, i64) -> i64, ff: impl Fn(f64, f64) -> f64) -> Value {
    match (a, b) {
        (Value::Int(x), Value::Int(y)) => Value::Int(fi(*x, *y)),
        _ => Value::Float(ff(a.as_f64().unwrap(), b.as_f64().unwrap())),
    }
}

/// Collapse a whole-valued float back to an int (matches `_num_result`).
fn collapse(v: f64) -> Value {
    if v.fract() == 0.0 && v.is_finite() {
        Value::Int(v as i64)
    } else {
        Value::Float(v)
    }
}

/// Python-style modulo (result takes the sign of the divisor).
fn py_mod_i64(a: i64, b: i64) -> i64 {
    let r = a % b;
    if r != 0 && (r < 0) != (b < 0) {
        r + b
    } else {
        r
    }
}

fn py_mod_f64(a: f64, b: f64) -> f64 {
    let r = a % b;
    if r != 0.0 && (r < 0.0) != (b < 0.0) {
        r + b
    } else {
        r
    }
}

/// Round half to even (banker's rounding), matching Python's `round`.
fn round_half_even(x: f64) -> i64 {
    let f = x.floor();
    let diff = x - f;
    if diff < 0.5 {
        f as i64
    } else if diff > 0.5 {
        f as i64 + 1
    } else {
        let fi = f as i64;
        if fi % 2 == 0 {
            fi
        } else {
            fi + 1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vm {
        let mut vm = Vm::new(Some("<test>".into()));
        vm.run_str(src).unwrap();
        vm
    }

    #[test]
    fn hello() {
        assert_eq!(run("[$Hello, World!$.]:h h@").out, "Hello, World!\n");
    }

    #[test]
    fn arithmetic() {
        assert_eq!(run("10 20 * .").out, "200\n");
        assert_eq!(run("10 4 / .").out, "2.5\n");
        assert_eq!(run("10 2 / .").out, "5\n");
        assert_eq!(run("10 3 % .").out, "1\n");
    }

    #[test]
    fn fizz_first_lines() {
        let out = run("1:i [ i 4 < ] [ i 3 % 0 == [$Fizz$] [i] ? @ . i 1 + :i ] ;").out;
        assert_eq!(out, "1\n2\nFizz\n");
    }

    #[test]
    fn errors() {
        let mut vm = Vm::new(None);
        assert_eq!(vm.run_str("1 +").unwrap_err().code(), STACK_UNDERFLOW);
        let mut vm = Vm::new(None);
        assert_eq!(vm.run_str("1 0 /").unwrap_err().code(), DIV_ZERO);
        let mut vm = Vm::new(None);
        assert_eq!(vm.run_str("[ 1 2").unwrap_err().code(), "REPAIR_SYNTAX");
    }
}
