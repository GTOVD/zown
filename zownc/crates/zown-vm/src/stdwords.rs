//! Std words and the test words (`wd`, `ts`, `pp`).
//!
//! `mx`, `mn`, and `cl` are Zown in `zown/std/core.zn`. This module loads that
//! file and runs the blocks. A trap inside a std word is reported as that word.

use std::rc::Rc;

use zown_ast::{Node, NodeKind};

use super::curve::{public_key, verify};
use super::error::{BOUNDS, SIG_INVALID, TEST_FAIL, TYPE_MISMATCH, UNSUPPORTED};
use super::value::display_f64;
use super::{RunError, Value, Vm};

const STD_SRC: &str = include_str!("../../../../zown/std/core.zn");

/// A word defined with `wd`. Std words are AST. A word defined by a native
/// program is a compiled function; `zown-rt` calls it.
#[derive(Clone)]
pub enum WordBody {
    Ast(Rc<Vec<Node>>),
    Func(u32),
}

fn code_esc(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\r', "\\r").replace('\n', "\\n")
}

fn code_lines(nodes: &[Node], depth: usize, out: &mut String) {
    let ind = "  ".repeat(depth);
    for node in nodes {
        match &node.kind {
            NodeKind::Blk(inner) => {
                out.push_str(&ind);
                out.push_str("blk\n");
                code_lines(inner, depth + 1, out);
                out.push_str(&ind);
                out.push_str("end\n");
            }
            other => {
                let (tag, shown) = match other {
                    NodeKind::Int(n) => ("int", n.to_string()),
                    NodeKind::Float(f) => ("float", display_f64(*f)),
                    NodeKind::Str(s) => ("str", code_esc(s)),
                    NodeKind::Name(s) => ("name", s.clone()),
                    NodeKind::Bind(s) => ("bind", s.clone()),
                    NodeKind::Move(s) => ("move", s.clone()),
                    NodeKind::Cap(s) => ("cap", s.clone()),
                    NodeKind::Op(s) => ("op", s.clone()),
                    NodeKind::Blk(_) => unreachable!(),
                };
                out.push_str(&ind);
                out.push_str(tag);
                out.push(' ');
                out.push_str(&shown);
                out.push('\n');
            }
        }
    }
}

fn is_word_name(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => chars.all(|c| c.is_ascii_alphanumeric()),
        _ => false,
    }
}

impl Vm {
    pub(super) fn load_std(&mut self) {
        self.run_str(STD_SRC).unwrap_or_else(|e| {
            panic!(
                "std/core.zn failed: {}",
                e.render_human(Some("std/core.zn"))
            )
        });
    }

    pub(super) fn run_word(&mut self, name: &str, body: Rc<Vec<Node>>) -> Result<(), RunError> {
        match self.invoke(body, name) {
            Ok(()) => Ok(()),
            Err(mut e) => {
                e.op = Some(name.to_string());
                Err(e)
            }
        }
    }

    pub(super) fn b_wd(&mut self) -> Result<(), RunError> {
        let name = match self.pop("wd")? {
            Value::Str(s) if is_word_name(&s) => s,
            _ => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`wd` expected a word name (a letter, then letters or digits)".into(),
                    Some("wd".into()),
                    "write the name as a string: [ ... ] $mx$ wd",
                ))
            }
        };
        let body = self.pop_block("wd")?;
        self.words.insert(name, WordBody::Ast(body));
        Ok(())
    }

    pub(super) fn b_cs(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("cs")?;
        let mut text = String::new();
        code_lines(&body, 0, &mut text);
        self.push_buf(text.into_bytes());
        Ok(())
    }

    /// `$name$ seed sig [test] [body] sw`.
    pub(super) fn b_sw(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("sw")?;
        let test = self.pop_block("sw")?;
        let mut message = String::new();
        code_lines(&body, 0, &mut message);
        let name = self.begin_swap(message.as_bytes())?;
        self.install_ast(&name, body);
        let depth = self.stack.len();
        let ran = self.invoke(test, "sw");
        if let Err(e) = ran {
            self.rollback_swap();
            return Err(e);
        }
        if let Err(e) = self.finish_test("sw", depth) {
            self.rollback_swap();
            return Err(e);
        }
        self.commit_swap();
        Ok(())
    }

    pub fn begin_swap(&mut self, message: &[u8]) -> Result<String, RunError> {
        let sig = self.take_buf("sw")?;
        let seed = self.take_buf("sw")?;
        let name = match self.pop("sw")? {
            Value::Str(s) if is_word_name(&s) => s,
            _ => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`sw` expected a word name (a letter, then letters or digits)".into(),
                    Some("sw".into()),
                    "",
                ))
            }
        };
        if sig.len() != 64 {
            return Err(self.bad_len("sw", "a signature", 64));
        }
        if seed.len() != 32 {
            return Err(self.bad_len("sw", "a seed", 32));
        }
        if !self.words.contains_key(&name) {
            return Err(self.err(
                BOUNDS,
                format!("`sw` has no word named {name}"),
                Some("sw".into()),
                "register it with wd first",
            ));
        }
        let public = public_key(&seed);
        if !verify(&public, message, &sig) {
            return Err(self.sec(
                SIG_INVALID,
                "sw",
                "`sw` signature is not valid".into(),
                "sign the canonical text from cs",
            ));
        }
        Ok(name)
    }

    pub fn install_ast(&mut self, name: &str, body: Rc<Vec<Node>>) {
        let old = self.words.get(name).cloned().expect("word");
        self.swap_undo.push((name.to_string(), old));
        self.words.insert(name.to_string(), WordBody::Ast(body));
    }

    pub fn install_func(&mut self, name: &str, id: u32) {
        let old = self.words.get(name).cloned().expect("word");
        self.swap_undo.push((name.to_string(), old));
        self.words.insert(name.to_string(), WordBody::Func(id));
    }

    pub fn rollback_swap(&mut self) {
        if let Some((name, body)) = self.swap_undo.pop() {
            self.words.insert(name, body);
        }
    }

    pub fn commit_swap(&mut self) {
        if let Some((name, _)) = self.swap_undo.pop() {
            self.tele.borrow_mut().line("swap", &name);
        }
    }

    /// The native id of a word defined by compiled code, if it has one.
    pub fn func_word(&self, name: &str) -> Option<u32> {
        match self.words.get(name) {
            Some(WordBody::Func(id)) => Some(*id),
            _ => None,
        }
    }

    /// `[body] $name$ wd` when `body` was compiled to a native function.
    pub fn register_func_word(&mut self, name: Value, id: u32) -> Result<(), RunError> {
        let name = match name {
            Value::Str(s) if is_word_name(&s) => s,
            _ => {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`wd` expected a word name (a letter, then letters or digits)".into(),
                    Some("wd".into()),
                    "write the name as a string: [ ... ] $mx$ wd",
                ))
            }
        };
        self.words.insert(name, WordBody::Func(id));
        Ok(())
    }

    pub(super) fn b_ts(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("ts")?;
        let depth = self.stack.len();
        self.invoke(body, "ts")?;
        self.finish_test("ts", depth)
    }

    pub(super) fn b_pp(&mut self) -> Result<(), RunError> {
        let body = self.pop_block("pp")?;
        let k = self.pop_count("pp")?;
        for _ in 0..k {
            let depth = self.stack.len();
            self.invoke(body.clone(), "pp")?;
            self.finish_test("pp", depth)?;
        }
        Ok(())
    }

    /// Pop a plain `int` repeat count. Negative is `BOUNDS`.
    pub fn pop_count(&mut self, op: &str) -> Result<i64, RunError> {
        match self.pop(op)? {
            Value::Int(n) if n >= 0 => Ok(n),
            Value::Int(_) => Err(self.err(
                BOUNDS,
                format!("`{op}` repeat count must be >= 0"),
                Some(op.to_string()),
                "",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected an int, got {tn}"),
                    Some(op.to_string()),
                    "the count is a plain int",
                ))
            }
        }
    }

    /// After a test body: exactly one new value, and it is truthy.
    pub fn finish_test(&mut self, op: &str, depth: usize) -> Result<(), RunError> {
        let left = self.stack.len() as isize - depth as isize;
        if left != 1 {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` body must leave exactly one value, left {left}"),
                Some(op.to_string()),
                "the body pushes 1 to pass",
            ));
        }
        let v = self.pop(op)?;
        if v.truthy() {
            Ok(())
        } else {
            Err(self.err(
                TEST_FAIL,
                format!("`{op}` body left a false value"),
                Some(op.to_string()),
                "a passing test leaves 1",
            ))
        }
    }

    pub(super) fn missing_std(&self, name: &str) -> RunError {
        self.err(
            UNSUPPORTED,
            format!("`{name}` is a std word and std/core.zn did not define it"),
            Some(name.to_string()),
            "",
        )
    }
}
