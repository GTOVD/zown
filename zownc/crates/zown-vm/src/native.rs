//! Hooks the Cranelift backend uses to drive a `Vm`. Block bodies run as native
//! functions. `??` reads a block's instruction shape; `at`, `gr`, and the vector
//! constructors call the native function. Values and words stay on this VM so
//! the two executions cannot drift.

use zown_ast::{Node, NodeKind, Pos};

use super::error::{RunError, BOUNDS, NO_MATCH, NOT_CALLABLE};
use super::value::{Res, VecVal};
use super::{Value, Vm};

/// One instruction of a compiled block, kept so `??` can read patterns.
/// The tree-walker still uses the AST; this is the native block's shape.
#[derive(Clone, Debug)]
pub enum NatOp {
    Int(i64),
    Float(f64),
    Str(String),
    Name(String),
    Op(String),
    Block(u32),
    Cap(String),
    Bind(String),
    Move(String),
}

impl Vm {
    pub fn apply_op(&mut self, op: &str) -> Result<(), RunError> {
        self.pos = None;
        self.exec_op(op)
    }

    pub fn apply_name(&mut self, name: &str) -> Result<(), RunError> {
        self.pos = None;
        self.exec_name(name)
    }

    pub fn apply_bind(&mut self, name: &str) -> Result<(), RunError> {
        self.pos = None;
        let v = self.pop(&format!(":{name}"))?;
        self.env.insert(name.to_string(), v);
        Ok(())
    }

    pub fn apply_move(&mut self, name: &str) -> Result<(), RunError> {
        self.pos = None;
        self.exec_move(name)
    }

    /// `cond [then] [else] ?` for native block values.
    pub fn apply_select(&mut self) -> Result<(), RunError> {
        self.pos = None;
        let els = self.pop_block_value("?")?;
        let then = self.pop_block_value("?")?;
        let cond = self.pop("?")?;
        self.push(if cond.truthy() { then } else { els });
        Ok(())
    }

    pub fn pop_truthy(&mut self, op: &str) -> Result<bool, RunError> {
        self.pos = None;
        Ok(self.pop(op)?.truthy())
    }

    pub fn pop_func_id(&mut self, op: &str) -> Result<u32, RunError> {
        self.pos = None;
        match self.pop(op)? {
            Value::Func { id, .. } => Ok(id),
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

    fn pop_block_value(&mut self, op: &str) -> Result<Value, RunError> {
        let v = self.pop(op)?;
        match v {
            Value::Func { .. } | Value::Block(_) => Ok(v),
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
}

impl Vm {
    pub fn is_bound(&self, name: &str) -> bool {
        self.env.contains_key(name)
    }

    pub fn pop_value(&mut self, op: &str) -> Result<Value, RunError> {
        self.pop(op)
    }

    pub fn pop_cap_name(&mut self, op: &str) -> Result<String, RunError> {
        self.pop_cap(op)
    }

    pub fn push_value(&mut self, v: Value) {
        self.push(v);
    }

    pub fn take_stack(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.stack)
    }

    pub fn set_stack(&mut self, stack: Vec<Value>) {
        self.stack = stack;
    }

    /// Record `name` as held. Returns whether this grant newly added it.
    pub fn hold_cap(&mut self, name: String) -> bool {
        self.caps.insert(name)
    }

    pub fn drop_cap(&mut self, name: &str) {
        self.caps.remove(name);
    }

    /// Wrap an `at` body. The outer stack is already restored; `produced` is
    /// what the body left, and `failed` is the fault `at` turns into `err`.
    pub fn seal_attempt(
        &mut self,
        produced: Vec<Value>,
        failed: Option<RunError>,
    ) -> Result<(), RunError> {
        match failed {
            Some(e) => {
                self.push(Value::Res(Res {
                    ok: false,
                    val: Box::new(Value::Str(e.code.to_string())),
                }));
                Ok(())
            }
            None if produced.len() == 1 => {
                let val = produced.into_iter().next().unwrap();
                self.push(Value::Res(Res {
                    ok: true,
                    val: Box::new(val),
                }));
                Ok(())
            }
            None => Err(self.err(
                BOUNDS,
                format!(
                    "`ty` body must leave exactly one value, left {}",
                    produced.len()
                ),
                Some("ty".into()),
                "bind inputs first (5 :x) and leave the single result on the stack",
            )),
        }
    }

    pub fn build_vec(
        &mut self,
        name: &'static str,
        count: usize,
        is_float: bool,
        signed: bool,
        bits: u32,
        produced: Vec<Value>,
    ) -> Result<(), RunError> {
        if produced.len() != count {
            return Err(self.err(
                BOUNDS,
                format!("`{name}` needs {count} lanes, got {}", produced.len()),
                Some(name.to_string()),
                "the block must leave exactly the lane count",
            ));
        }
        let mut lanes = Vec::with_capacity(count);
        for x in produced {
            lanes.push(self.coerce_lane(name, x, is_float, signed, bits)?);
        }
        self.push(Value::Vec(VecVal {
            name,
            count,
            is_float,
            signed,
            bits,
            lanes,
        }));
        Ok(())
    }

    pub fn bad_arm_count(&self, n: usize) -> RunError {
        self.bad_pat(format!(
            "match arms are [pattern][body] pairs; got {n} blocks"
        ))
    }

    pub fn bad_arm_shape(&self) -> RunError {
        self.bad_pat("each arm is [pattern] [body], both blocks".into())
    }

    pub fn unmatched(&self, subject: &Value) -> RunError {
        let what = match subject {
            Value::Res(r) if !r.ok => "err result".to_string(),
            Value::Res(_) => "ok result".to_string(),
            other => format!("{} subject", other.type_name()),
        };
        let hint = if matches!(subject, Value::Res(_)) {
            "handle it: [err] [ ... ], or add a default arm [any] [ ... ]"
        } else {
            "add a default arm: [any] [ ... ]"
        };
        self.err(
            NO_MATCH,
            format!("no arm matched the {what}"),
            Some("??".into()),
            hint,
        )
    }

    /// Decide one native pattern block. `ops` is that block's instruction list.
    pub fn match_native(&self, subject: &Value, ops: &[NatOp]) -> Result<Option<Value>, RunError> {
        if ops.len() != 1 {
            return Err(self.bad_pat(format!(
                "a pattern holds exactly one token, got {}",
                ops.len()
            )));
        }
        let node = nat_node(&ops[0]);
        self.pattern_matches(subject, &[node])
    }
}

fn nat_node(op: &NatOp) -> Node {
    let kind = match op {
        NatOp::Int(v) => NodeKind::Int(*v),
        NatOp::Float(v) => NodeKind::Float(*v),
        NatOp::Str(s) => NodeKind::Str(s.clone()),
        NatOp::Name(s) => NodeKind::Name(s.clone()),
        NatOp::Op(s) => NodeKind::Op(s.clone()),
        NatOp::Block(_) => NodeKind::Blk(Vec::new()),
        NatOp::Cap(s) => NodeKind::Cap(s.clone()),
        NatOp::Bind(s) => NodeKind::Bind(s.clone()),
        NatOp::Move(s) => NodeKind::Move(s.clone()),
    };
    Node::new(kind, Pos::default())
}
