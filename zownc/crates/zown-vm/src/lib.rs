//! Zown stack VM (Rust port of `zown/vm.py` + `zown/builtins.py`).
//!
//! Tree-walking interpreter. v0.1 arithmetic is checked (SPEC §11, M8d-ii):
//! an `int` result outside `i64` is `OVERFLOW`, and `b` opts into exact `big`.
//! v0.2 words (`gr`/`rq`/`hv`, widths, vectors, `??`, `at`, `'x`) landed in
//! M8d-iii. Observable behavior matches the Python oracle.

mod bigint;
mod error;
mod native;
mod value;

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use zown_ast::{Node, NodeKind, Pos};
use zown_parser::{parse, ParseError};

use bigint::BigInt;
use value::{f64_fits_int, num_partial_cmp, Lane, VecVal, WidthTag};

pub use error::*;
pub use native::NatOp;
pub use value::Value;

/// Nested block invocations before a structured `STACK_OVERFLOW` (SPEC §19).
/// Loops (`;`) do not accumulate depth: each condition and body returns.
const MAX_INVOKE_DEPTH: u32 = 128;

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
                    "{{\n  \"zerr\": \"0.2\",\n  \"kind\": \"{kind}\",\n  \"code\": \"{code}\",\n  \"msg\": {msg},\n  \"op\": null,\n  \"pos\": {{\"line\": {l}, \"col\": {c}, \"offset\": {o}}},\n  \"stack\": [],\n  \"hint\": {hint},\n  \"file\": {file}\n}}",
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
    /// Capabilities granted right now. Empty until a `gr` block adds one.
    caps: HashSet<String>,
    depth: u32,
    pub out: String,
    pub file: Option<String>,
    pos: Option<Pos>,
}

enum Bin {
    Add,
    Sub,
    Mul,
}

impl Vm {
    pub fn new(file: Option<String>) -> Self {
        Vm {
            stack: Vec::new(),
            env: HashMap::new(),
            caps: HashSet::new(),
            depth: 0,
            out: String::new(),
            file,
            pos: None,
        }
    }

    /// Parse + run a source string. Ungated: the checker (M8d-v) sits above this.
    pub fn run_str(&mut self, src: &str) -> Result<(), Diag> {
        let program = parse(src).map_err(Diag::Parse)?;
        self.exec(&program).map_err(Diag::Run)
    }

    fn push(&mut self, v: Value) {
        self.stack.push(v);
    }

    fn err(
        &self,
        code: &'static str,
        msg: String,
        op: Option<String>,
        hint: &'static str,
    ) -> RunError {
        RunError {
            code,
            msg,
            op,
            hint,
            stack: self.stack.iter().map(|v| v.snapshot_json()).collect(),
            file: self.file.clone(),
            pos: self.pos,
            kind: "run",
            cap: None,
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
        if v.is_num() {
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

    fn pop_cap(&mut self, op: &str) -> Result<String, RunError> {
        match self.pop(op)? {
            Value::Cap(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a capability `name, got {tn}"),
                    Some(op.to_string()),
                    "write a capability token like `s (net-send)",
                ))
            }
        }
    }

    fn exec(&mut self, nodes: &[Node]) -> Result<(), RunError> {
        for node in nodes {
            self.exec_node(node)?;
        }
        Ok(())
    }

    fn invoke(&mut self, blk: Rc<Vec<Node>>, op: &str) -> Result<(), RunError> {
        if self.depth >= MAX_INVOKE_DEPTH {
            return Err(self.err(
                STACK_OVERFLOW,
                format!("block invocations nested deeper than the limit ({MAX_INVOKE_DEPTH})"),
                Some(op.to_string()),
                "unbounded recursion? give it a base case, or iterate with `;`",
            ));
        }
        self.depth += 1;
        let res = self.exec(&blk);
        self.depth -= 1;
        res
    }

    fn exec_node(&mut self, node: &Node) -> Result<(), RunError> {
        self.pos = Some(node.pos);
        match &node.kind {
            NodeKind::Int(v) => self.push(Value::Int(*v)),
            NodeKind::Float(v) => self.push(Value::Float(*v)),
            NodeKind::Str(s) => self.push(Value::Str(s.clone())),
            NodeKind::Blk(inner) => self.push(Value::Block(Rc::new(inner.clone()))),
            NodeKind::Name(s) => self.exec_name(s)?,
            NodeKind::Bind(s) => {
                let v = self.pop(&format!(":{s}"))?;
                self.env.insert(s.clone(), v);
            }
            NodeKind::Move(s) => self.exec_move(s)?,
            NodeKind::Cap(s) => self.push(Value::Cap(s.clone())),
            NodeKind::Op(s) => self.exec_op(s)?,
        }
        Ok(())
    }

    fn exec_move(&mut self, name: &str) -> Result<(), RunError> {
        match self.env.remove(name) {
            Some(v) => {
                self.push(v);
                Ok(())
            }
            None => Err(self.err(
                NAME_UNRESOLVED,
                format!("`'{name}`: {name} is not bound; a move needs a live binding"),
                Some(format!("'{name}")),
                "was it already moved out? rebind it first",
            )),
        }
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
            "==" | "!=" | "<" | ">" | "<=" | ">=" => self.op_rel(op),
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
            "??" => self.op_match(),
            other => Err(self.err(
                UNSUPPORTED,
                format!("operator `{other}` is not implemented"),
                Some(other.to_string()),
                "",
            )),
        }
    }

    // --- arithmetic (SPEC §11: overflow is never silent) -----------------------
    fn op_add(&mut self) -> Result<(), RunError> {
        let b = self.pop("+")?;
        let a = self.pop("+")?;
        if matches!(a, Value::Str(_)) || matches!(b, Value::Str(_)) {
            self.push(Value::Str(format!("{}{}", a.display(), b.display())));
            return Ok(());
        }
        self.push(self.num_binop("+", &a, &b, Bin::Add)?);
        Ok(())
    }

    fn op_sub(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("-")?;
        let a = self.pop_num("-")?;
        self.push(self.num_binop("-", &a, &b, Bin::Sub)?);
        Ok(())
    }

    fn op_mul(&mut self) -> Result<(), RunError> {
        let b = self.pop("*")?;
        let a = self.pop("*")?;
        if let (Value::Str(s), Value::Int(n)) = (&a, &b) {
            self.push(Value::Str(if *n > 0 {
                s.repeat(*n as usize)
            } else {
                String::new()
            }));
            return Ok(());
        }
        self.push(self.num_binop("*", &a, &b, Bin::Mul)?);
        Ok(())
    }

    fn op_div(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("/")?;
        let a = self.pop_num("/")?;
        if num_is_zero(&b) {
            return Err(self.err(
                DIV_ZERO,
                "division by zero".into(),
                Some("/".into()),
                "guard the denominator with `=0 ==` before dividing",
            ));
        }
        if a.is_float() || b.is_float() {
            let r = self.to_float("/", &a)? / self.to_float("/", &b)?;
            self.push(num_result(r));
            return Ok(());
        }
        let av = a.to_bigint().unwrap();
        let bv = b.to_bigint().unwrap();
        let (q, r) = av.divmod_floor(&bv).unwrap();
        if r.is_zero() {
            self.push(if a.is_big() || b.is_big() {
                Value::Big(q)
            } else {
                self.fit_int("/", q)?
            });
        } else {
            match av.true_div_f64(&bv) {
                Some(f) => self.push(Value::Float(f)),
                None => {
                    return Err(self.err(
                        OVERFLOW,
                        "`/`: quotient is too large for a float".into(),
                        Some("/".into()),
                        "",
                    ))
                }
            }
        }
        Ok(())
    }

    fn op_mod(&mut self) -> Result<(), RunError> {
        let b = self.pop_num("%")?;
        let a = self.pop_num("%")?;
        if num_is_zero(&b) {
            return Err(self.err(DIV_ZERO, "modulo by zero".into(), Some("%".into()), ""));
        }
        if a.is_float() || b.is_float() {
            let r = py_mod_f64(self.to_float("%", &a)?, self.to_float("%", &b)?);
            self.push(Value::Float(r));
            return Ok(());
        }
        let av = a.to_bigint().unwrap();
        let bv = b.to_bigint().unwrap();
        let r = av.divmod_floor(&bv).unwrap().1;
        self.push(if a.is_big() || b.is_big() {
            Value::Big(r)
        } else {
            self.fit_int("%", r)?
        });
        Ok(())
    }

    fn op_neg(&mut self) -> Result<(), RunError> {
        match self.pop_num("_")? {
            Value::Big(b) => self.push(Value::Big(b.neg())),
            Value::Float(f) => self.push(Value::Float(-f)),
            Value::Int(i) => self.push(self.fit_int("_", BigInt::from_i64(i).neg())?),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn num_binop(&self, op: &str, a: &Value, b: &Value, which: Bin) -> Result<Value, RunError> {
        self.require_num(a, op)?;
        self.require_num(b, op)?;
        if a.is_float() || b.is_float() {
            let (fa, fb) = (self.to_float(op, a)?, self.to_float(op, b)?);
            let r = match which {
                Bin::Add => fa + fb,
                Bin::Sub => fa - fb,
                Bin::Mul => fa * fb,
            };
            return Ok(Value::Float(r));
        }
        let (av, bv) = (a.to_bigint().unwrap(), b.to_bigint().unwrap());
        let r = match which {
            Bin::Add => av.add(&bv),
            Bin::Sub => av.sub(&bv),
            Bin::Mul => av.mul(&bv),
        };
        if a.is_big() || b.is_big() {
            Ok(Value::Big(r))
        } else {
            self.fit_int(op, r)
        }
    }

    fn to_float(&self, op: &str, v: &Value) -> Result<f64, RunError> {
        match v {
            Value::Int(i) => Ok(*i as f64),
            Value::Float(f) => Ok(*f),
            Value::Big(b) => b.to_f64().ok_or_else(|| {
                self.err(
                    OVERFLOW,
                    format!("`{op}`: big value is too large for a float"),
                    Some(op.to_string()),
                    "keep the computation in big (no float operand)",
                )
            }),
            _ => unreachable!(),
        }
    }

    fn fit_int(&self, op: &str, n: BigInt) -> Result<Value, RunError> {
        match n.to_i64() {
            Some(i) => Ok(Value::Int(i)),
            None => Err(self.err(
                OVERFLOW,
                format!(
                    "`{op}` result {n} does not fit int [{}..{}]",
                    i64::MIN,
                    i64::MAX
                ),
                Some(op.to_string()),
                "convert an operand with `b` for arbitrary-precision big math",
            )),
        }
    }

    fn require_num(&self, v: &Value, op: &str) -> Result<(), RunError> {
        if v.is_num() {
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

    // --- comparison / logic ----------------------------------------------------
    fn op_rel(&mut self, op: &str) -> Result<(), RunError> {
        let b = self.pop(op)?;
        let a = self.pop(op)?;
        let bit = self.rel_bool(op, &a, &b)?;
        self.push(Value::Int(if bit { 1 } else { 0 }));
        Ok(())
    }

    fn rel_bool(&self, op: &str, a: &Value, b: &Value) -> Result<bool, RunError> {
        if let (Value::Str(x), Value::Str(y)) = (a, b) {
            let ord = x.cmp(y);
            return Ok(match op {
                "==" => ord.is_eq(),
                "!=" => ord.is_ne(),
                "<" => ord.is_lt(),
                ">" => ord.is_gt(),
                "<=" => ord.is_le(),
                ">=" => ord.is_ge(),
                _ => false,
            });
        }
        if op == "==" || op == "!=" {
            let eq = a.zown_eq(b);
            return Ok(if op == "==" { eq } else { !eq });
        }
        match num_partial_cmp(a, b) {
            Some(ord) => Ok(match op {
                "<" => ord.is_lt(),
                ">" => ord.is_gt(),
                "<=" => ord.is_le(),
                ">=" => ord.is_ge(),
                _ => false,
            }),
            None if a.is_num() && b.is_num() => Ok(false),
            None => Err(self.err(
                TYPE_MISMATCH,
                format!(
                    "`{op}` cannot compare {} and {}",
                    a.type_name(),
                    b.type_name()
                ),
                Some(op.to_string()),
                "",
            )),
        }
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

    fn op_dup(&mut self) -> Result<(), RunError> {
        match self.stack.last() {
            Some(v) => {
                let v = v.clone();
                self.push(v);
                Ok(())
            }
            None => Err(self.err(
                STACK_UNDERFLOW,
                "`=` (dup) needs a value".into(),
                Some("=".into()),
                "",
            )),
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
            return Err(self.err(
                STACK_UNDERFLOW,
                "`&` (over) needs two values".into(),
                Some("&".into()),
                "",
            ));
        }
        let v = self.stack[self.stack.len() - 2].clone();
        self.push(v);
        Ok(())
    }

    fn op_print(&mut self) -> Result<(), RunError> {
        let v = self.pop(".")?;
        self.out.push_str(&v.display());
        self.out.push('\n');
        Ok(())
    }

    fn op_invoke(&mut self) -> Result<(), RunError> {
        let b = self.pop_block("@")?;
        self.invoke(b, "@")
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
            self.invoke(cond.clone(), ";")?;
            if !self.pop(";")?.truthy() {
                break;
            }
            self.invoke(body.clone(), ";")?;
        }
        Ok(())
    }

    fn op_match(&mut self) -> Result<(), RunError> {
        let arms = self.pop_block("??")?;
        let subject = self.pop("??")?;
        if arms.len() % 2 != 0 {
            return Err(self.bad_arm_count(arms.len()));
        }
        for pair in arms.chunks(2) {
            match (&pair[0].kind, &pair[1].kind) {
                (NodeKind::Blk(pat), NodeKind::Blk(body)) => {
                    if let Some(value) = self.pattern_matches(&subject, pat)? {
                        self.push(value);
                        return self.invoke(Rc::new(body.clone()), "??");
                    }
                }
                _ => return Err(self.bad_arm_shape()),
            }
        }
        Err(self.unmatched(&subject))
    }

    /// `Some(value)` when the pattern matches. `ok`/`err` unwrap the payload.
    fn pattern_matches(&self, subject: &Value, pat: &[Node]) -> Result<Option<Value>, RunError> {
        if pat.len() != 1 {
            return Err(self.bad_pat(format!(
                "a pattern holds exactly one token, got {}",
                pat.len()
            )));
        }
        let n = &pat[0];
        let matched = match &n.kind {
            NodeKind::Op(s) if s == "_" => return Ok(Some(subject.clone())),
            NodeKind::Int(v) => subject.zown_eq(&Value::Int(*v)),
            NodeKind::Float(v) => subject.zown_eq(&Value::Float(*v)),
            NodeKind::Str(s) => subject.zown_eq(&Value::Str(s.clone())),
            NodeKind::Name(s) if s == "ok" || s == "err" => {
                if let Value::Res(r) = subject {
                    if r.ok == (s == "ok") {
                        return Ok(Some((*r.val).clone()));
                    }
                }
                false
            }
            NodeKind::Name(s) if is_type_pattern(s) => subject.type_name() == s,
            other => {
                return Err(self.bad_pat(format!("unrecognized pattern `{}`", pat_desc(other))))
            }
        };
        Ok(if matched { Some(subject.clone()) } else { None })
    }

    fn bad_pat(&self, msg: String) -> RunError {
        self.err(
            BAD_PATTERN,
            msg,
            Some("??".into()),
            "a pattern is a type name, a literal, ok/err, or _ (default)",
        )
    }

    // --- builtins --------------------------------------------------------------
    fn try_builtin(&mut self, name: &str) -> Option<Result<(), RunError>> {
        Some(match name {
            "ln" => self.b_len(),
            "tr" => self.b_str_map("tr", |s| s.trim().to_string()),
            "up" => self.b_str_map("up", |s| s.to_uppercase()),
            "lo" => self.b_str_map("lo", |s| s.to_lowercase()),
            "rv" => self.b_str_map("rv", |s| s.chars().rev().collect()),
            "pr" => self.b_print_raw(),
            "ab" => self.b_abs(),
            "mx" => self.b_minmax(true),
            "mn" => self.b_minmax(false),
            "sq" => self.b_sqrt(),
            "pw" => self.b_pow(),
            "fl" => self.b_roundish("fl"),
            "ce" => self.b_roundish("ce"),
            "rd" => self.b_roundish("rd"),
            "s" => self.b_to_str(),
            "n" => self.b_to_num(),
            "b" => self.b_big(),
            "dp" => self.b_depth(),
            "rt" => self.b_rot(),
            "clr" => self.b_clear(),
            "at" => self.b_attempt(),
            "gr" => self.b_grant(),
            "rq" => self.b_require(),
            "hv" => self.b_have(),
            "wr" => self.b_policy("wr"),
            "st" => self.b_policy("st"),
            "ck" => self.b_policy("ck"),
            "i8" => self.b_width(true, 8),
            "i16" => self.b_width(true, 16),
            "i32" => self.b_width(true, 32),
            "i64" => self.b_width(true, 64),
            "i128" => self.b_width(true, 128),
            "u8" => self.b_width(false, 8),
            "u16" => self.b_width(false, 16),
            "u32" => self.b_width(false, 32),
            "u64" => self.b_width(false, 64),
            "u128" => self.b_width(false, 128),
            "f4" => self.b_vec("f4", 4, true, true, 32),
            "d2" => self.b_vec("d2", 2, true, true, 64),
            "i4" => self.b_vec("i4", 4, false, true, 32),
            "b16" => self.b_vec("b16", 16, false, false, 8),
            "vadd" => self.b_vbinop("vadd", Bin::Add),
            "vsub" => self.b_vbinop("vsub", Bin::Sub),
            "vmul" => self.b_vbinop("vmul", Bin::Mul),
            "vsum" => self.b_vsum(),
            "vat" => self.b_vat(),
            _ => return None,
        })
    }

    fn b_len(&mut self) -> Result<(), RunError> {
        let v = self.pop("ln")?;
        match v {
            Value::Str(s) => self.push(Value::Int(s.chars().count() as i64)),
            Value::Block(b) => self.push(Value::Int(b.len() as i64)),
            Value::Func { nodes, .. } => self.push(Value::Int(nodes as i64)),
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`ln` expected a string or block, got {tn}"),
                    Some("ln".into()),
                    "",
                ));
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
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a string, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn b_print_raw(&mut self) -> Result<(), RunError> {
        let v = self.pop("pr")?;
        self.out.push_str(&v.display());
        Ok(())
    }

    fn b_abs(&mut self) -> Result<(), RunError> {
        match self.pop_num("ab")? {
            Value::Big(b) => self.push(Value::Big(b.abs())),
            Value::Float(f) => self.push(Value::Float(f.abs())),
            Value::Int(i) => self.push(self.fit_int("ab", BigInt::from_i64(i).abs())?),
            _ => unreachable!(),
        }
        Ok(())
    }

    fn b_minmax(&mut self, want_max: bool) -> Result<(), RunError> {
        let op = if want_max { "mx" } else { "mn" };
        let b = self.pop_num(op)?;
        let a = self.pop_num(op)?;
        let b_wins = match num_partial_cmp(&b, &a) {
            Some(Ordering::Greater) => want_max,
            Some(Ordering::Less) => !want_max,
            _ => false,
        };
        self.push(if b_wins { b } else { a });
        Ok(())
    }

    fn b_sqrt(&mut self) -> Result<(), RunError> {
        match self.pop_num("sq")? {
            Value::Big(b) => {
                if b.is_neg() {
                    return Err(self.no_real("sq"));
                }
                let r = b.isqrt();
                if r.mul(&r) == b {
                    self.push(Value::Big(r));
                } else {
                    let f = self.to_float("sq", &Value::Big(b))?;
                    self.push(num_result(f.sqrt()));
                }
            }
            Value::Float(f) => {
                if f < 0.0 {
                    return Err(self.no_real("sq"));
                }
                self.push(num_result(f.sqrt()));
            }
            Value::Int(i) => {
                if i < 0 {
                    return Err(self.no_real("sq"));
                }
                self.push(num_result((i as f64).sqrt()));
            }
            _ => unreachable!(),
        }
        Ok(())
    }

    fn no_real(&self, op: &str) -> RunError {
        self.err(
            UB_TRAP,
            format!("`{op}`: the square root of a negative number has no real result"),
            Some(op.to_string()),
            "guard the operand's sign first; Zown traps instead of producing nan",
        )
    }

    fn b_pow(&mut self) -> Result<(), RunError> {
        let e = self.pop_num("pw")?;
        let a = self.pop_num("pw")?;
        if a.is_float() || e.is_float() {
            return self.pow_float(&a, &e);
        }
        let ev = e.to_bigint().unwrap();
        if ev.is_neg() {
            return self.pow_neg_exp(&a, &ev);
        }
        if a.is_big() || e.is_big() {
            return self.pow_big(&a.to_bigint().unwrap(), &ev);
        }
        let ai = match a {
            Value::Int(i) => i,
            _ => unreachable!(),
        };
        let ei = ev.to_i64().unwrap();
        if ei == 0 {
            self.push(Value::Int(1));
            return Ok(());
        }
        match ai {
            0 => self.push(Value::Int(0)),
            1 => self.push(Value::Int(1)),
            -1 => self.push(Value::Int(if ei % 2 == 0 { 1 } else { -1 })),
            _ => {
                if ei > 63 {
                    return Err(self.err(
                        OVERFLOW,
                        format!("`pw` result {ai}**{ei} does not fit int"),
                        Some("pw".into()),
                        "convert the base with `b` for arbitrary-precision big math",
                    ));
                }
                self.push(self.fit_int("pw", BigInt::from_i64(ai).pow_u32(ei as u32))?);
            }
        }
        Ok(())
    }

    fn pow_float(&mut self, a: &Value, e: &Value) -> Result<(), RunError> {
        let fa = self.to_float("pw", a)?;
        let fe = self.to_float("pw", e)?;
        if fa < 0.0 && !(fe.is_finite() && fe.fract() == 0.0) {
            return Err(self.err(
                UB_TRAP,
                "`pw`: a negative base with a fractional exponent has no real result".into(),
                Some("pw".into()),
                "guard the operand's sign first; Zown traps instead of producing nan",
            ));
        }
        if fa == 0.0 && fe < 0.0 {
            return Err(self.err(
                DIV_ZERO,
                "`pw`: zero to a negative power".into(),
                Some("pw".into()),
                "",
            ));
        }
        self.push(num_result(powf_zown(fa, fe)));
        Ok(())
    }

    fn pow_neg_exp(&mut self, a: &Value, ev: &BigInt) -> Result<(), RunError> {
        if a.to_bigint().unwrap().is_zero() {
            return Err(self.err(
                DIV_ZERO,
                "`pw`: zero to a negative power".into(),
                Some("pw".into()),
                "",
            ));
        }
        let fa = self.to_float("pw", a)?;
        let odd = exp_is_odd(ev);
        let fe = match ev.to_f64() {
            Some(f) => f,
            // A huge negative exponent underflows to 0 (Python's OverflowError path).
            None => {
                self.push(Value::Float(if fa < 0.0 && odd { -0.0 } else { 0.0 }));
                return Ok(());
            }
        };
        let mag = fa.abs().powf(fe);
        let r = if fa < 0.0 && odd { -mag } else { mag };
        self.push(num_result(r));
        Ok(())
    }

    fn pow_big(&mut self, base: &BigInt, exp: &BigInt) -> Result<(), RunError> {
        if exp.is_zero() || base == &BigInt::one() {
            self.push(Value::Big(BigInt::one()));
            return Ok(());
        }
        if base.is_zero() {
            self.push(Value::Big(BigInt::zero()));
            return Ok(());
        }
        if base == &BigInt::from_i64(-1) {
            let odd = exp_is_odd(exp);
            self.push(Value::Big(if odd {
                BigInt::from_i64(-1)
            } else {
                BigInt::one()
            }));
            return Ok(());
        }
        let n = exp.to_u32().ok_or_else(|| {
            self.err(
                OVERFLOW,
                "`pw`: exponent does not fit the runtime's big-pow budget".into(),
                Some("pw".into()),
                "",
            )
        })?;
        self.push(Value::Big(base.pow_u32(n)));
        Ok(())
    }

    fn b_roundish(&mut self, op: &str) -> Result<(), RunError> {
        let v = self.pop_num(op)?;
        if let Value::Float(f) = v {
            if !f.is_finite() {
                return self.non_finite_to_int(op, f);
            }
            let n = match op {
                "fl" => BigInt::from_f64_exact(f.floor()).unwrap(),
                "ce" => BigInt::from_f64_exact(f.ceil()).unwrap(),
                "rd" => round_half_even(f),
                _ => unreachable!(),
            };
            self.push(self.fit_int(op, n)?);
        } else {
            self.push(v);
        }
        Ok(())
    }

    fn non_finite_to_int(&self, op: &str, f: f64) -> Result<(), RunError> {
        if f.is_nan() {
            Err(self.err(
                UB_TRAP,
                format!("`{op}` cannot convert nan to an integer"),
                Some(op.to_string()),
                "guard with a comparison: nan is not equal to itself",
            ))
        } else {
            Err(self.err(
                OVERFLOW,
                format!("`{op}` cannot convert {} to an integer", value::fmt_f64(f)),
                Some(op.to_string()),
                "",
            ))
        }
    }

    fn b_to_str(&mut self) -> Result<(), RunError> {
        let v = self.pop("s")?;
        self.push(Value::Str(v.display()));
        Ok(())
    }

    fn b_to_num(&mut self) -> Result<(), RunError> {
        let v = self.pop("n")?;
        if v.is_num() {
            self.push(v);
            return Ok(());
        }
        let Value::Str(s) = v else {
            return Err(self.err(
                TYPE_MISMATCH,
                "`n` expected a string or number".into(),
                Some("n".into()),
                "",
            ));
        };
        let t = s.trim();
        let low = t.to_lowercase();
        let int_like = !low.contains('.')
            && !low.contains('e')
            && low != "inf"
            && low != "-inf"
            && low != "nan";
        if int_like {
            let n = BigInt::parse_dec(t).ok_or_else(|| {
                self.err(
                    TYPE_MISMATCH,
                    format!("`n` cannot parse {s:?} as a number"),
                    Some("n".into()),
                    "",
                )
            })?;
            self.push(self.fit_int("n", n).map_err(|mut e| {
                e.msg = format!("`n`: {t:?} does not fit int");
                e.hint = "use `b` to parse it as an arbitrary-precision big";
                e
            })?);
        } else {
            match low.parse::<f64>() {
                Ok(f) => self.push(Value::Float(f)),
                Err(_) => {
                    return Err(self.err(
                        TYPE_MISMATCH,
                        format!("`n` cannot parse {s:?} as a number"),
                        Some("n".into()),
                        "",
                    ))
                }
            }
        }
        Ok(())
    }

    fn b_big(&mut self) -> Result<(), RunError> {
        match self.pop("b")? {
            Value::Big(b) => self.push(Value::Big(b)),
            Value::Int(n) => self.push(Value::Big(BigInt::from_i64(n))),
            Value::Float(f) => match BigInt::from_f64_exact(f) {
                Some(b) => self.push(Value::Big(b)),
                None => {
                    return Err(self.err(
                        TYPE_MISMATCH,
                        "`b` needs a whole number, got a fractional float".into(),
                        Some("b".into()),
                        "round/floor first",
                    ))
                }
            },
            Value::Str(s) => match BigInt::parse_dec(s.trim()) {
                Some(b) => self.push(Value::Big(b)),
                None => {
                    return Err(self.err(
                        TYPE_MISMATCH,
                        format!("`b` cannot parse {s:?} as a number"),
                        Some("b".into()),
                        "",
                    ))
                }
            },
            other => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!(
                        "`b` expected a number or digit string, got {}",
                        other.type_name()
                    ),
                    Some("b".into()),
                    "",
                ))
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
            return Err(self.err(
                STACK_UNDERFLOW,
                "`rt` (rot) needs three values".into(),
                Some("rt".into()),
                "",
            ));
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

    fn b_attempt(&mut self) -> Result<(), RunError> {
        let blk = self.pop_block("at")?;
        let saved = std::mem::take(&mut self.stack);
        let ran = self.invoke(blk, "at");
        let left = std::mem::replace(&mut self.stack, saved);
        self.seal_attempt(left, ran.err())
    }

    fn b_grant(&mut self) -> Result<(), RunError> {
        let blk = self.pop_block("gr")?;
        let name = self.pop_cap("gr")?;
        let had = self.caps.contains(&name);
        self.caps.insert(name.clone());
        let res = self.invoke(blk, "gr");
        if !had {
            self.caps.remove(&name);
        }
        res
    }

    fn b_require(&mut self) -> Result<(), RunError> {
        let name = self.pop_cap("rq")?;
        if self.caps.contains(&name) {
            return Ok(());
        }
        let mut e = self.err(
            CAP_DENIED,
            format!("operation requires capability `{name}; not granted"),
            Some("rq".into()),
            "grant it with `name [ ... ] gr",
        );
        e.kind = "sec";
        e.cap = Some(name);
        Err(e)
    }

    fn b_have(&mut self) -> Result<(), RunError> {
        let name = self.pop_cap("hv")?;
        let on = self.caps.contains(&name);
        self.push(Value::Int(if on { 1 } else { 0 }));
        Ok(())
    }

    fn b_width(&mut self, signed: bool, bits: u32) -> Result<(), RunError> {
        self.push(Value::Width(WidthTag { signed, bits }));
        Ok(())
    }

    fn b_policy(&mut self, op: &str) -> Result<(), RunError> {
        let w = self.pop_width(op)?;
        let n = self.pop_intish(op)?;
        let result = match op {
            "wr" => n.wrap_width(w.signed, w.bits),
            "st" => {
                let lo = w.lo();
                let hi = w.hi();
                if n < lo {
                    lo
                } else if n > hi {
                    hi
                } else {
                    n
                }
            }
            "ck" => {
                if n < w.lo() || n > w.hi() {
                    return Err(self.err(
                        OVERFLOW,
                        format!("{n} does not fit {} [{}..{}]", w.name(), w.lo(), w.hi()),
                        Some("ck".into()),
                        "use `wr` to wrap, `st` to saturate, or widen the type",
                    ));
                }
                n
            }
            _ => unreachable!(),
        };
        self.push(self.typed(&w, result));
        Ok(())
    }

    fn pop_width(&mut self, op: &str) -> Result<WidthTag, RunError> {
        match self.pop(op)? {
            Value::Width(w) => Ok(w),
            _ => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a width tag (i8..u128) on top"),
                Some(op.to_string()),
                "push a width like u8 before the policy word",
            )),
        }
    }

    fn pop_intish(&mut self, op: &str) -> Result<BigInt, RunError> {
        let v = self.pop(op)?;
        match v {
            Value::Int(n) => Ok(BigInt::from_i64(n)),
            Value::Big(b) => Ok(b),
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 => {
                Ok(BigInt::from_f64_exact(f).unwrap())
            }
            Value::Float(_) => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected an integer, got a fractional float"),
                Some(op.to_string()),
                "round/floor first, or use a float op",
            )),
            other => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected an integer, got {}", other.type_name()),
                Some(op.to_string()),
                "",
            )),
        }
    }

    fn typed(&self, w: &WidthTag, n: BigInt) -> Value {
        if w.fits_int() {
            Value::Int(n.to_i64().expect("a fitting width yields an int"))
        } else {
            Value::Big(n)
        }
    }

    fn b_vec(
        &mut self,
        name: &'static str,
        count: usize,
        is_float: bool,
        signed: bool,
        bits: u32,
    ) -> Result<(), RunError> {
        let blk = self.pop_block(name)?;
        let saved = std::mem::take(&mut self.stack);
        let ran = self.invoke(blk, name);
        let produced = std::mem::replace(&mut self.stack, saved);
        ran?;
        self.build_vec(name, count, is_float, signed, bits, produced)
    }

    fn coerce_lane(
        &self,
        name: &str,
        x: Value,
        is_float: bool,
        signed: bool,
        bits: u32,
    ) -> Result<Lane, RunError> {
        if !x.is_num() {
            return Err(self.err(
                TYPE_MISMATCH,
                format!("`{name}` lanes must be numbers, got {}", x.type_name()),
                Some(name.to_string()),
                "",
            ));
        }
        if is_float {
            return Ok(Lane::Float(self.to_float(name, &x)?));
        }
        let n = match x {
            Value::Float(f) => {
                if !(f.is_finite() && f.fract() == 0.0) {
                    return Err(self.err(
                        TYPE_MISMATCH,
                        format!("`{name}` integer lanes need whole numbers"),
                        Some(name.to_string()),
                        "",
                    ));
                }
                BigInt::from_f64_exact(f).unwrap()
            }
            other => other.to_bigint().unwrap(),
        };
        Ok(Lane::Int(n.wrap_width(signed, bits).to_i64().unwrap()))
    }

    fn b_vbinop(&mut self, op: &str, which: Bin) -> Result<(), RunError> {
        let b = self.pop_vec(op)?;
        let a = self.pop_vec(op)?;
        if a.name != b.name {
            return Err(self.err(
                TYPE_MISMATCH,
                format!(
                    "`{op}` needs matching vector types, got {} and {}",
                    a.name, b.name
                ),
                Some(op.to_string()),
                "",
            ));
        }
        let mut lanes = Vec::with_capacity(a.lanes.len());
        for (x, y) in a.lanes.iter().zip(&b.lanes) {
            lanes.push(match (x, y, a.is_float) {
                (Lane::Float(x), Lane::Float(y), true) => Lane::Float(match which {
                    Bin::Add => x + y,
                    Bin::Sub => x - y,
                    Bin::Mul => x * y,
                }),
                (Lane::Int(x), Lane::Int(y), false) => {
                    let r = match which {
                        Bin::Add => *x as i128 + *y as i128,
                        Bin::Sub => *x as i128 - *y as i128,
                        Bin::Mul => *x as i128 * *y as i128,
                    };
                    Lane::Int(wrap_i128(r, a.signed, a.bits))
                }
                _ => unreachable!(),
            });
        }
        self.push(Value::Vec(VecVal {
            name: a.name,
            count: a.count,
            is_float: a.is_float,
            signed: a.signed,
            bits: a.bits,
            lanes,
        }));
        Ok(())
    }

    fn pop_vec(&mut self, op: &str) -> Result<VecVal, RunError> {
        match self.pop(op)? {
            Value::Vec(v) => Ok(v),
            other => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a vector, got {}", other.type_name()),
                Some(op.to_string()),
                "",
            )),
        }
    }

    fn b_vsum(&mut self) -> Result<(), RunError> {
        let v = self.pop_vec("vsum")?;
        if v.is_float {
            let mut t = 0.0;
            for lane in &v.lanes {
                if let Lane::Float(f) = lane {
                    t += *f;
                }
            }
            self.push(num_result(t));
        } else {
            let mut t = BigInt::zero();
            for lane in &v.lanes {
                if let Lane::Int(i) = lane {
                    t = t.add(&BigInt::from_i64(*i));
                }
            }
            self.push(match t.to_i64() {
                Some(i) => Value::Int(i),
                None => Value::Big(t),
            });
        }
        Ok(())
    }

    fn b_vat(&mut self) -> Result<(), RunError> {
        let idx = self.pop_intish("vat")?;
        let v = self.pop_vec("vat")?;
        let n = v.lanes.len();
        match idx.to_i64() {
            Some(i) if i >= 0 && (i as usize) < n => {
                let lane = v.lanes.into_iter().nth(i as usize).unwrap();
                self.push(match lane {
                    Lane::Int(k) => Value::Int(k),
                    Lane::Float(f) => Value::Float(f),
                });
                Ok(())
            }
            _ => Err(self.err(
                BOUNDS,
                format!(
                    "lane {idx} is outside {} [0..{}]",
                    v.name,
                    n.saturating_sub(1)
                ),
                Some("vat".into()),
                "",
            )),
        }
    }
}

/// Builtin words. The static checker treats a name in this set as resolved.
/// Keep it aligned with `Vm::try_builtin`.
pub fn is_builtin(name: &str) -> bool {
    matches!(
        name,
        "ln" | "tr"
            | "up"
            | "lo"
            | "rv"
            | "pr"
            | "ab"
            | "mx"
            | "mn"
            | "sq"
            | "pw"
            | "fl"
            | "ce"
            | "rd"
            | "s"
            | "n"
            | "b"
            | "dp"
            | "rt"
            | "clr"
            | "at"
            | "gr"
            | "rq"
            | "hv"
            | "wr"
            | "st"
            | "ck"
            | "i8"
            | "i16"
            | "i32"
            | "i64"
            | "i128"
            | "u8"
            | "u16"
            | "u32"
            | "u64"
            | "u128"
            | "f4"
            | "d2"
            | "i4"
            | "b16"
            | "vadd"
            | "vsub"
            | "vmul"
            | "vsum"
            | "vat"
    )
}

fn is_type_pattern(s: &str) -> bool {
    matches!(
        s,
        "int" | "float" | "str" | "bool" | "block" | "cap" | "width" | "vec" | "big" | "res"
    )
}

fn pat_desc(k: &NodeKind) -> String {
    match k {
        NodeKind::Int(v) => v.to_string(),
        NodeKind::Float(v) => value::fmt_f64(*v),
        NodeKind::Str(s) | NodeKind::Name(s) | NodeKind::Op(s) => s.clone(),
        NodeKind::Cap(s) => format!("`{s}"),
        NodeKind::Bind(s) => format!(":{s}"),
        NodeKind::Move(s) => format!("'{s}"),
        NodeKind::Blk(_) => "blk".into(),
    }
}

fn num_is_zero(v: &Value) -> bool {
    match v {
        Value::Int(0) => true,
        Value::Big(b) if b.is_zero() => true,
        Value::Float(f) if *f == 0.0 => true,
        _ => false,
    }
}

/// Collapse a whole float into `int` only when the integer fits (SPEC §11).
fn num_result(v: f64) -> Value {
    match f64_fits_int(v) {
        Some(i) => Value::Int(i),
        None => Value::Float(v),
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

fn powf_zown(base: f64, exp: f64) -> f64 {
    if base < 0.0 && exp.is_finite() && exp.fract() == 0.0 {
        let mag = base.abs().powf(exp);
        if py_mod_f64(exp, 2.0) == 1.0 {
            -mag
        } else {
            mag
        }
    } else {
        base.powf(exp)
    }
}

fn exp_is_odd(e: &BigInt) -> bool {
    e.divmod_floor(&BigInt::from_i64(2)).unwrap().1 == BigInt::one()
}

fn round_half_even(x: f64) -> BigInt {
    if x.fract() == 0.0 {
        return BigInt::from_f64_exact(x).unwrap();
    }
    let f = x.floor();
    let diff = x - f;
    let chosen = if diff < 0.5 {
        f
    } else if diff > 0.5 {
        f + 1.0
    } else if py_mod_f64(f, 2.0) == 0.0 {
        f
    } else {
        f + 1.0
    };
    BigInt::from_f64_exact(chosen).unwrap()
}

fn wrap_i128(n: i128, signed: bool, bits: u32) -> i64 {
    let modulus = 1i128 << bits;
    let mut m = n.rem_euclid(modulus);
    if signed && m >= (modulus >> 1) {
        m -= modulus;
    }
    m as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(src: &str) -> Vm {
        let mut vm = Vm::new(Some("<test>".into()));
        vm.run_str(src).unwrap();
        vm
    }

    fn code(src: &str) -> &'static str {
        let mut vm = Vm::new(None);
        vm.run_str(src).unwrap_err().code()
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
        assert_eq!(code("1 +"), STACK_UNDERFLOW);
        assert_eq!(code("1 0 /"), DIV_ZERO);
        assert_eq!(code("[ 1 2"), "REPAIR_SYNTAX");
    }

    #[test]
    fn checked_int_and_big() {
        assert_eq!(code("9223372036854775807 1 +"), OVERFLOW);
        assert_eq!(code("2 100 pw"), OVERFLOW);
        assert_eq!(code("4 _ sq"), UB_TRAP);
        assert_eq!(run("2 b 100 pw .").out, "1267650600228229401496703205376\n");
        assert_eq!(run("10 b 5 / .").out, "2\n");
        assert_eq!(run("10 b 4 / .").out, "2.5\n");
        assert_eq!(run("5 b 5 == .").out, "1\n");
        assert_eq!(run("10.0 30 pw .").out, "1e+30\n");
        assert_eq!(run("10.5 400 pw .").out, "inf\n");
        assert_eq!(run("10.5 400 pw _ .").out, "-inf\n");
        assert_eq!(run("10.5 400 pw = - .").out, "nan\n");
        assert_eq!(run("7 _ 3 % .").out, "2\n");
        assert_eq!(run("7 3 _ % .").out, "-2\n");
    }

    #[test]
    fn v2_words() {
        assert_eq!(run("300 u8 wr .").out, "44\n");
        assert_eq!(run("5 u64 ck .").out, "5\n");
        assert_eq!(run("1 _ u64 wr .").out, "18446744073709551615\n");
        assert_eq!(run("[ 1 2 3 4 ] i4 vsum .").out, "10\n");
        assert_eq!(
            run("[ 1 2 3 4 ] i4 [ 10 20 30 40 ] i4 vadd .").out,
            "i4(11 22 33 44)\n"
        );
        assert_eq!(run("`s hv .").out, "0\n");
        assert_eq!(run("`s [ `s hv . ] gr").out, "1\n");
        assert_eq!(run("[ 1 0 / ] at .").out, "err(DIV_ZERO)\n");
        assert_eq!(run("5 :x 'x .").out, "5\n");
        assert_eq!(
            run("42 [ [int] [ , $int$ . ] [_] [ , $no$ . ] ] ??").out,
            "int\n"
        );
        assert_eq!(code("[ f @ ] :f f @"), STACK_OVERFLOW);
        assert_eq!(code("1.5 0 /"), DIV_ZERO);
        assert_eq!(code("`s rq"), CAP_DENIED);
    }
}
