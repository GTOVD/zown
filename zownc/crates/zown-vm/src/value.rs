//! Zown runtime values (parity with the Python reference VM).

use std::cell::RefCell;
use std::cmp::Ordering;
use std::rc::Rc;

use zown_ast::Node;

use crate::bigint::BigInt;

/// A fixed-width integer tag (`i8` … `ux`). Policy words `wr`/`st`/`ck`
/// reduce a value into it (SPEC §11).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthTag {
    pub signed: bool,
    pub bits: u32,
}

impl WidthTag {
    pub fn name(&self) -> String {
        if self.bits == 128 {
            return if self.signed { "ix" } else { "ux" }.to_string();
        }
        format!("{}{}", if self.signed { 'i' } else { 'u' }, self.bits)
    }

    pub fn lo(&self) -> BigInt {
        if self.signed {
            BigInt::one().shl(self.bits - 1).neg()
        } else {
            BigInt::zero()
        }
    }

    pub fn hi(&self) -> BigInt {
        if self.signed {
            BigInt::one().shl(self.bits - 1).sub(&BigInt::one())
        } else {
            BigInt::one().shl(self.bits).sub(&BigInt::one())
        }
    }

    /// The width's whole range fits the checked 64-bit `int` (else a policy
    /// result is `big` — the type follows the width, not the value).
    pub fn fits_int(&self) -> bool {
        self.lo() >= BigInt::from_i64(i64::MIN) && self.hi() <= BigInt::from_i64(i64::MAX)
    }
}

/// One SIMD lane. Integer lanes are already reduced into their width.
#[derive(Clone, Debug)]
pub enum Lane {
    Int(i64),
    Float(f64),
}

impl Lane {
    fn display(&self) -> String {
        match self {
            Lane::Int(v) => v.to_string(),
            Lane::Float(v) => display_f64(*v),
        }
    }
}

/// A fixed-lane SIMD vector (`vf`, `vd`, `vi`, `vu`).
#[derive(Clone, Debug)]
pub struct VecVal {
    pub name: &'static str,
    pub count: usize,
    pub is_float: bool,
    pub signed: bool,
    pub bits: u32,
    pub lanes: Vec<Lane>,
}

/// `ok(payload)` or `err(code)`. The payload is reachable only through `??`.
#[derive(Clone, Debug)]
pub struct Res {
    pub ok: bool,
    pub val: Box<Value>,
}

/// A runtime value. Blocks are reference-counted quotations.
#[derive(Clone, Debug)]
pub enum Value {
    Int(i64),
    Float(f64),
    /// Arbitrary-precision integer (`b`, or a width whose range exceeds `int`).
    Big(BigInt),
    Str(String),
    Block(Rc<Vec<Node>>),
    /// A block compiled to a native function. `nodes` is the AST length, so
    /// truthiness and `[blk:N]` match the interpreter. The tree-walker never
    /// builds these; the Cranelift backend does.
    Func {
        id: u32,
        nodes: u32,
    },
    /// Capability token (SPEC §12). Holding it is not a grant.
    Cap(String),
    Width(WidthTag),
    Vec(VecVal),
    Res(Res),
    /// `r g b co`. Channels are `0..255`.
    Color(Rgb),
    /// A layout node: `tx`, `bt`, `rw`, or `cm`.
    Ui(UiNode),
    /// A graph query cursor. Names are the vertices it currently sits on.
    Cur(Vec<String>),
    /// A heap buffer. `exclusive` may `bp`. Dropping that value frees `alloc`.
    Buf {
        alloc: Rc<RefCell<BufAlloc>>,
        exclusive: bool,
    },
}

/// Bytes behind a `buf`. `None` means the exclusive owner was dropped.
#[derive(Debug)]
pub struct BufAlloc {
    pub data: Option<Vec<u8>>,
    pub n: usize,
}

/// An sRGB color. The layout contrast test uses these channels linearly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// One node of the retained layout tree. Child `x`/`y` are relative to the parent.
#[derive(Clone, Debug)]
pub struct UiNode {
    pub kind: &'static str,
    pub text: String,
    pub color: Rgb,
    pub x: i64,
    pub y: i64,
    pub w: i64,
    pub h: i64,
    pub children: Vec<UiNode>,
}

impl Value {
    pub fn is_num(&self) -> bool {
        matches!(self, Value::Int(_) | Value::Float(_) | Value::Big(_))
    }

    pub fn is_float(&self) -> bool {
        matches!(self, Value::Float(_))
    }

    pub fn is_big(&self) -> bool {
        matches!(self, Value::Big(_))
    }

    /// Unwrap to the exact integer. Floats must be finite and whole.
    pub fn to_bigint(&self) -> Option<BigInt> {
        match self {
            Value::Int(n) => Some(BigInt::from_i64(*n)),
            Value::Big(b) => Some(b.clone()),
            Value::Float(f) => BigInt::from_f64_exact(*f),
            _ => None,
        }
    }

    /// Truthiness: 0 / 0.0 / "" / empty block are false. A result is truthy
    /// iff it is `ok` (the payload stays wrapped).
    pub fn truthy(&self) -> bool {
        match self {
            Value::Int(v) => *v != 0,
            Value::Float(v) => *v != 0.0,
            Value::Big(v) => !v.is_zero(),
            Value::Str(s) => !s.is_empty(),
            Value::Block(b) => !b.is_empty(),
            Value::Func { nodes, .. } => *nodes != 0,
            Value::Cap(_) | Value::Width(_) | Value::Color(_) | Value::Ui(_) => true,
            Value::Cur(names) => !names.is_empty(),
            Value::Buf { alloc, .. } => alloc.borrow().n > 0,
            Value::Vec(v) => !v.lanes.is_empty(),
            Value::Res(r) => r.ok,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Big(_) => "big",
            Value::Str(_) => "str",
            Value::Block(_) | Value::Func { .. } => "block",
            Value::Cap(_) => "cap",
            Value::Width(_) => "width",
            Value::Vec(_) => "vec",
            Value::Res(_) => "res",
            Value::Color(_) => "color",
            Value::Ui(_) => "node",
            Value::Cur(_) => "cur",
            Value::Buf { .. } => "buf",
        }
    }

    /// String form used by `.`, `pr`, `s`, and `+` concatenation.
    /// Whole floats render as integers only while they fit `int` (`7.0` → `7`);
    /// larger ones keep float notation (`1e+30`).
    pub fn display(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => display_f64(*v),
            Value::Big(v) => v.to_string(),
            Value::Str(s) => s.clone(),
            Value::Block(b) => format!("[blk:{}]", b.len()),
            Value::Func { nodes, .. } => format!("[blk:{nodes}]"),
            Value::Cap(n) => format!("`{n}"),
            Value::Width(w) => w.name(),
            Value::Vec(v) => {
                let lanes: Vec<String> = v.lanes.iter().map(Lane::display).collect();
                format!("{}({})", v.name, lanes.join(" "))
            }
            Value::Res(r) => {
                format!("{}({})", if r.ok { "ok" } else { "err" }, r.val.display())
            }
            Value::Color(c) => format!("co({} {} {})", c.r, c.g, c.b),
            Value::Ui(n) => {
                if n.kind == "tx" || n.kind == "bt" {
                    format!("{}({})", n.kind, n.text)
                } else {
                    format!("{}({})", n.kind, n.children.len())
                }
            }
            Value::Cur(names) => format!("cur({})", names.len()),
            Value::Buf { alloc, exclusive } => buf_text(alloc, *exclusive),
        }
    }

    /// A JSON fragment for the error stack snapshot.
    pub fn snapshot_json(&self) -> String {
        match self {
            Value::Int(v) => v.to_string(),
            Value::Float(v) => {
                if v.is_nan() {
                    "NaN".into()
                } else if *v == f64::INFINITY {
                    "Infinity".into()
                } else if *v == f64::NEG_INFINITY {
                    "-Infinity".into()
                } else {
                    // Debug form is a JSON number (`1e30` / `2.5`).
                    format!("{:?}", v)
                }
            }
            Value::Big(v) => json_str(&format!("big:{v}")),
            Value::Str(s) => json_str(s),
            Value::Block(b) => format!("\"[blk:{}]\"", b.len()),
            Value::Func { nodes, .. } => format!("\"[blk:{nodes}]\""),
            Value::Cap(n) => json_str(&format!("`{n}")),
            Value::Width(w) => json_str(&w.name()),
            Value::Vec(v) => json_str(&format!("{}[{}]", v.name, v.lanes.len())),
            Value::Res(r) => json_str(&format!(
                "{}:{}",
                if r.ok { "ok" } else { "err" },
                snap_flat(&r.val)
            )),
            Value::Color(c) => json_str(&format!("co:{} {} {}", c.r, c.g, c.b)),
            Value::Ui(n) => json_str(&format!("{}:{}", n.kind, n.children.len())),
            Value::Cur(names) => json_str(&format!("cur:{}", names.len())),
            Value::Buf { alloc, exclusive } => json_str(&buf_text(alloc, *exclusive)),
        }
    }

    /// Equality for `==` / `!=`. Numbers compare by value across int/big/float
    /// (exactly, not through `f64`). Blocks compare by identity.
    pub fn zown_eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Str(a), Value::Str(b)) => a == b,
            (Value::Block(a), Value::Block(b)) => Rc::ptr_eq(a, b),
            (Value::Func { id: a, .. }, Value::Func { id: b, .. }) => a == b,
            (Value::Cap(a), Value::Cap(b)) => a == b,
            (Value::Width(a), Value::Width(b)) => a == b,
            (Value::Vec(a), Value::Vec(b)) => {
                a.name == b.name
                    && a.lanes.len() == b.lanes.len()
                    && a.lanes.iter().zip(&b.lanes).all(|(x, y)| match (x, y) {
                        (Lane::Int(i), Lane::Int(j)) => i == j,
                        (Lane::Float(i), Lane::Float(j)) => i == j,
                        _ => false,
                    })
            }
            (Value::Res(a), Value::Res(b)) => a.ok == b.ok && a.val.zown_eq(&b.val),
            (Value::Color(a), Value::Color(b)) => a == b,
            (Value::Ui(a), Value::Ui(b)) => ui_eq(a, b),
            (Value::Cur(a), Value::Cur(b)) => a == b,
            (
                Value::Buf {
                    alloc: a,
                    exclusive: ea,
                },
                Value::Buf {
                    alloc: b,
                    exclusive: eb,
                },
            ) => Rc::ptr_eq(a, b) && ea == eb,
            _ => matches!(num_partial_cmp(self, other), Some(Ordering::Equal)),
        }
    }
}

fn buf_text(alloc: &Rc<RefCell<BufAlloc>>, exclusive: bool) -> String {
    let g = alloc.borrow();
    let body = if g.data.is_none() {
        "-".to_string()
    } else {
        g.n.to_string()
    };
    if exclusive {
        format!("bf({body})")
    } else {
        format!("bf&({body})")
    }
}

fn ui_eq(a: &UiNode, b: &UiNode) -> bool {
    a.kind == b.kind
        && a.text == b.text
        && a.color == b.color
        && a.x == b.x
        && a.y == b.y
        && a.w == b.w
        && a.h == b.h
        && a.children.len() == b.children.len()
        && a.children.iter().zip(&b.children).all(|(x, y)| ui_eq(x, y))
}

/// Ordering of two numbers. `None` when either is `nan` or not a number.
pub fn num_partial_cmp(a: &Value, b: &Value) -> Option<Ordering> {
    if !a.is_num() || !b.is_num() {
        return None;
    }
    match (a, b) {
        (Value::Float(x), Value::Float(y)) => x.partial_cmp(y),
        (Value::Float(f), other) => cmp_f64_num(*f, other),
        (other, Value::Float(f)) => cmp_f64_num(*f, other).map(Ordering::reverse),
        _ => Some(a.to_bigint()?.cmp(&b.to_bigint()?)),
    }
}

fn cmp_f64_num(f: f64, n: &Value) -> Option<Ordering> {
    if f.is_nan() {
        return None;
    }
    if f.is_infinite() {
        return Some(if f.is_sign_positive() {
            Ordering::Greater
        } else {
            Ordering::Less
        });
    }
    let b = n.to_bigint()?;
    if f.fract() == 0.0 {
        return Some(BigInt::from_f64_exact(f)?.cmp(&b));
    }
    // `f` sits strictly between floor(f) and the next integer.
    let fl = BigInt::from_f64_exact(f.floor())?;
    match fl.cmp(&b) {
        Ordering::Less => Some(Ordering::Less),
        _ => Some(Ordering::Greater),
    }
}

/// Whole floats that fit `int` print as that integer; everything else uses
/// CPython's shortest round-trip (`{:?}` here) with a signed, 2-digit exponent.
pub fn display_f64(v: f64) -> String {
    if let Some(i) = f64_fits_int(v) {
        return i.to_string();
    }
    fmt_f64(v)
}

/// `Some` when `v` is a finite whole float whose exact integer value fits `int`.
pub fn f64_fits_int(v: f64) -> Option<i64> {
    if !v.is_finite() || v.fract() != 0.0 {
        return None;
    }
    // `i64::MAX as f64` rounds up to 2^63, which does not fit.
    if !(v >= (i64::MIN as f64) && v < 9223372036854775808.0) {
        return None;
    }
    let i = v as i64;
    if i as f64 == v {
        Some(i)
    } else {
        None
    }
}

pub fn fmt_f64(x: f64) -> String {
    if x.is_nan() {
        return "nan".into();
    }
    if x.is_infinite() {
        return if x.is_sign_negative() {
            "-inf".into()
        } else {
            "inf".into()
        };
    }
    normalize_py_exp(&format!("{:?}", x))
}

fn normalize_py_exp(s: &str) -> String {
    let Some(i) = s.find('e') else {
        return s.to_string();
    };
    let (mant, rest) = s.split_at(i);
    let exp = &rest[1..];
    let (sign, digits) = if let Some(d) = exp.strip_prefix('+') {
        ('+', d)
    } else if let Some(d) = exp.strip_prefix('-') {
        ('-', d)
    } else {
        ('+', exp)
    };
    format!("{mant}e{sign}{:0>2}", digits)
}

fn snap_flat(v: &Value) -> String {
    match v {
        Value::Str(s) => s.clone(),
        Value::Big(b) => format!("big:{b}"),
        Value::Res(r) => format!("{}:{}", if r.ok { "ok" } else { "err" }, snap_flat(&r.val)),
        other => other.display(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_float_spellings() {
        assert_eq!(fmt_f64(2.5), "2.5");
        assert_eq!(fmt_f64(1.5), "1.5");
        assert_eq!(fmt_f64(1e30), "1e+30");
        assert_eq!(fmt_f64(1e16), "1e+16");
        assert_eq!(fmt_f64(1e-5), "1e-05");
        assert_eq!(fmt_f64(0.0001), "0.0001");
        assert_eq!(fmt_f64(2.0f64.sqrt()), "1.4142135623730951");
        assert_eq!(fmt_f64(1.0 / 3.0), "0.3333333333333333");
        assert_eq!(fmt_f64(f64::INFINITY), "inf");
        assert_eq!(fmt_f64(f64::NEG_INFINITY), "-inf");
        assert_eq!(fmt_f64(f64::NAN), "nan");
        assert_eq!(display_f64(7.0), "7");
        assert_eq!(display_f64(1e30), "1e+30");
        assert_eq!(display_f64(-0.0), "0");
    }
}
