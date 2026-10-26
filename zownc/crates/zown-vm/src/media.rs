//! `av` decodes one AV1 still. `ve` encodes one 16×16 4:2:0 frame.
//! `op` decodes one narrowband Opus packet. `oe` encodes one.

use std::cell::RefCell;
use std::rc::Rc;

use super::av1;
use super::opus;
use super::error::{BOUNDS, INTEGRITY_FAIL, TYPE_MISMATCH, UNSUPPORTED};
use super::value::{BufAlloc, Value};
use super::{RunError, Vm};

impl Vm {
    fn take_media(&mut self, op: &str) -> Result<Vec<u8>, RunError> {
        match self.pop(op)? {
            Value::Buf { alloc, exclusive } => {
                let data = {
                    let g = alloc.borrow();
                    match &g.data {
                        Some(d) => d.clone(),
                        None => {
                            drop(g);
                            return Err(self.err(
                                BOUNDS,
                                format!("`{op}` buffer was freed"),
                                Some(op.to_string()),
                                "the exclusive owner was dropped with `,`",
                            ));
                        }
                    }
                };
                if exclusive {
                    alloc.borrow_mut().data = None;
                }
                Ok(data)
            }
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a buf, got {tn}"),
                    Some(op.to_string()),
                    "the packet or the frame is a buf",
                ))
            }
        }
    }

    fn push_media(&mut self, data: Vec<u8>) {
        let n = data.len();
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(data),
                n,
            })),
            exclusive: true,
        });
    }

    fn media_fault(&self, op: &str, code: &'static str, msg: String) -> RunError {
        let code = match code {
            "BOUNDS" => BOUNDS,
            "INTEGRITY_FAIL" => INTEGRITY_FAIL,
            "UNSUPPORTED" => UNSUPPORTED,
            _ => TYPE_MISMATCH,
        };
        self.err(code, msg, Some(op.to_string()), "")
    }

    pub(super) fn b_oe(&mut self) -> Result<(), RunError> {
        let data = self.take_media("oe")?;
        match opus::encode(&data) {
            Ok(pkt) => {
                self.push_media(pkt);
                Ok(())
            }
            Err((code, msg)) => Err(self.media_fault("oe", code, msg)),
        }
    }

    pub(super) fn b_op(&mut self) -> Result<(), RunError> {
        let data = self.take_media("op")?;
        match opus::decode(&data) {
            Ok(pcm) => {
                self.push_media(pcm);
                Ok(())
            }
            Err((code, msg)) => Err(self.media_fault("op", code, msg)),
        }
    }

    pub(super) fn b_av(&mut self) -> Result<(), RunError> {
        let data = self.take_media("av")?;
        match av1::decode(&data) {
            Ok(pic) => {
                self.push_media(pic);
                Ok(())
            }
            Err((code, msg)) => Err(self.media_fault("av", code, msg)),
        }
    }

    pub(super) fn b_ve(&mut self) -> Result<(), RunError> {
        let data = self.take_media("ve")?;
        match av1::encode(&data) {
            Ok(obu) => {
                self.push_media(obu);
                Ok(())
            }
            Err((code, msg)) => Err(self.media_fault("ve", code, msg)),
        }
    }
}
