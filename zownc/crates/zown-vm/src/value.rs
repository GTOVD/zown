//! Zown runtime values (parity with the Python reference VM).

use std::rc::Rc;

use zown_ast::Node;

/// A runtime value. Blocks are reference-counted quotations.
#[derive(Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(String),
    Block(Rc<Vec<Node>>),
}

impl Value {
    /// Truthiness: 0 / 0.0 / "" / empty block are false.
    pub fn truthy(&self) -> bool {
        match self {
            Value::Int(v) => *v != 0,
            Value::Float(v) => *v != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Block(b) => !b.is_empty(),
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "str",
            Value::Block(_) => "block",
        }
    }

    /// String form used by `.`, `pr`, `s`, and `+` concatenation. Mirrors the
    /// Python `_as_str`: whole-valued floats render as integers; blocks render
    /// as `[blk:N]`.
    pub fn display(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => {
                if v.fract() == 0.0 && v.is_finite() {
                    (*v as i64).to_string()
                } else {
                    v.to_string()
                }
            }
            Value::Str(s) => s.clone(),
            Value::Block(b) => format!("[blk:{}]", b.len()),
        }
    }

    /// A JSON fragment for the error stack snapshot (numbers as numbers, strings
    /// quoted, blocks as a short tag) — matching the reference `.zerr` packet.
    pub fn snapshot_json(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => v.to_string(),
            Value::Str(s) => json_str(s),
            Value::Block(b) => format!("\"[blk:{}]\"", b.len()),
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::Float(v) => Some(*v),
            _ => None,
        }
    }

    /// Equality for `==` / `!=`. Numbers compare across int/float; strings compare
    /// by contents; blocks by pointer identity; mixed categories are unequal.
    pub fn zown_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Block(a), Value::Block(b)) => Rc::ptr_eq(a, b),
            _ => match (self.as_f64(), other.as_f64()) {
                (Some(a), Some(b)) => a == b,
                _ => false,
            },
        }
    }
}

pub fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
