//! Exclusive byte buffers (SPEC §21). Same rules as `zown/buf.py`.

use std::cell::RefCell;
use std::rc::Rc;

use super::error::{BOUNDS, TYPE_MISMATCH};
use super::value::{BufAlloc, Value};
use super::{RunError, Vm};

pub const BUF_MAX: i64 = 1 << 20;

impl Vm {
    pub(super) fn release_buf(&mut self, v: Value) {
        if let Value::Buf {
            alloc,
            exclusive: true,
        } = &v
        {
            alloc.borrow_mut().data = None;
        }
        if let Value::Stm {
            conn,
            exclusive: true,
        } = v
        {
            conn.close();
        }
    }

    fn pop_buf_int(&mut self, op: &str, what: &str) -> Result<i64, RunError> {
        match self.pop(op)? {
            Value::Int(n) => Ok(n),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected {what}, got {tn}"),
                    Some(op.to_string()),
                    "",
                ))
            }
        }
    }

    fn live<'a>(
        &self,
        alloc: &'a RefCell<BufAlloc>,
        op: &str,
    ) -> Result<std::cell::Ref<'a, BufAlloc>, RunError> {
        let g = alloc.borrow();
        if g.data.is_none() {
            return Err(self.err(
                BOUNDS,
                format!("`{op}` buffer was freed"),
                Some(op.to_string()),
                "the exclusive owner was dropped with `,`",
            ));
        }
        Ok(g)
    }

    pub(super) fn b_bf(&mut self) -> Result<(), RunError> {
        let n = self.pop_buf_int("bf", "an int >= 0")?;
        if n < 0 {
            return Err(self.err(
                TYPE_MISMATCH,
                format!("`bf` expected an int >= 0, got {n}"),
                Some("bf".into()),
                "",
            ));
        }
        if n > BUF_MAX {
            return Err(self.err(
                BOUNDS,
                format!("`bf` length {n} is above the limit {BUF_MAX}"),
                Some("bf".into()),
                "",
            ));
        }
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(vec![0; n as usize]),
                n: n as usize,
            })),
            exclusive: true,
        });
        Ok(())
    }

    pub(super) fn b_bg(&mut self) -> Result<(), RunError> {
        let idx = self.pop_buf_int("bg", "an int index")?;
        let alloc = match self.stack.last() {
            Some(Value::Buf { alloc, .. }) => alloc.clone(),
            Some(other) => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`bg` expected a buf, got {tn}"),
                    Some("bg".into()),
                    "",
                ));
            }
            None => {
                return Err(self.err(
                    super::error::STACK_UNDERFLOW,
                    "`bg` needs a buffer under the index".into(),
                    Some("bg".into()),
                    "",
                ))
            }
        };
        let g = self.live(&alloc, "bg")?;
        let data = g.data.as_ref().unwrap();
        if idx < 0 || idx as usize >= data.len() {
            let msg = if data.is_empty() {
                format!("`bg` index {idx} is outside empty buf")
            } else {
                format!("`bg` index {idx} is outside buf [0..{}]", data.len() - 1)
            };
            return Err(self.err(BOUNDS, msg, Some("bg".into()), ""));
        }
        let byte = data[idx as usize] as i64;
        drop(g);
        self.push(Value::Int(byte));
        Ok(())
    }

    pub(super) fn b_bp(&mut self) -> Result<(), RunError> {
        let byte = self.pop_buf_int("bp", "an int byte")?;
        let idx = self.pop_buf_int("bp", "an int index")?;
        let (alloc, exclusive) = match self.stack.last() {
            Some(Value::Buf { alloc, exclusive }) => (alloc.clone(), *exclusive),
            Some(other) => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`bp` expected a buf, got {tn}"),
                    Some("bp".into()),
                    "",
                ));
            }
            None => {
                return Err(self.err(
                    super::error::STACK_UNDERFLOW,
                    "`bp` needs a buffer under the index".into(),
                    Some("bp".into()),
                    "",
                ))
            }
        };
        if !exclusive {
            return Err(self.err(
                TYPE_MISMATCH,
                "`bp` needs the exclusive buf, got a borrow".into(),
                Some("bp".into()),
                "write through the owner, not a borrow from `=` or a bare name",
            ));
        }
        if !(0..=255).contains(&byte) {
            return Err(self.err(
                BOUNDS,
                format!("`bp` byte {byte} is outside 0..255"),
                Some("bp".into()),
                "",
            ));
        }
        let problem = {
            let mut g = alloc.borrow_mut();
            if g.data.is_none() {
                Some("freed")
            } else if idx < 0 || idx as usize >= g.n {
                Some(if g.n == 0 { "empty" } else { "range" })
            } else {
                g.data.as_mut().unwrap()[idx as usize] = byte as u8;
                None
            }
        };
        match problem {
            Some("freed") => Err(self.err(
                BOUNDS,
                "`bp` buffer was freed".into(),
                Some("bp".into()),
                "the exclusive owner was dropped with `,`",
            )),
            Some("empty") => Err(self.err(
                BOUNDS,
                format!("`bp` index {idx} is outside empty buf"),
                Some("bp".into()),
                "",
            )),
            Some(_) => Err(self.err(
                BOUNDS,
                format!(
                    "`bp` index {idx} is outside buf [0..{}]",
                    alloc.borrow().n - 1
                ),
                Some("bp".into()),
                "",
            )),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{Diag, Vm, BOUNDS, TYPE_MISMATCH};

    fn run(src: &str) -> String {
        let mut vm = Vm::new(None);
        vm.run_str(src).unwrap();
        vm.out
    }

    fn code(src: &str) -> &'static str {
        let mut vm = Vm::new(None);
        match vm.run_str(src) {
            Err(Diag::Run(e)) => e.code,
            other => panic!("expected a runtime fault, got {other:?}"),
        }
    }

    #[test]
    fn write_through_the_owner_and_read_through_a_borrow() {
        assert_eq!(
            run("4 bf = ln . 0 65 bp 0 bg . = :r 1 66 bp r 1 bg .\n"),
            "4\n65\n66\n"
        );
        assert_eq!(code("4 bf = 0 1 bp"), TYPE_MISMATCH);
        assert_eq!(code("4 bf 4 bg"), BOUNDS);
        assert_eq!(code("4 bf = :r , r 0 bg"), BOUNDS);
        assert_eq!(code("1 _ bf"), TYPE_MISMATCH);
    }
}
