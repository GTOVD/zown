//! Std words and the test words (`wd`, `ts`, `pp`).
//!
//! `mx`, `mn`, and `cl` are Zown in `zown/std/core.zn`. This module loads that
//! file and runs the blocks. A trap inside a std word is reported as that word.

use std::rc::Rc;

use zown_ast::Node;

use super::error::{BOUNDS, TEST_FAIL, TYPE_MISMATCH, UNSUPPORTED};
use super::{RunError, Value, Vm};

const STD_SRC: &str = include_str!("../../../../zown/std/core.zn");

/// A word defined with `wd`. Std words are AST. A word defined by a native
/// program is a compiled function; `zown-rt` calls it.
#[derive(Clone)]
pub(super) enum WordBody {
    Ast(Rc<Vec<Node>>),
    Func(u32),
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
