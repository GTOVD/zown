//! `ae` `ad` `es` `ev` `xk`. Keys are bufs. Inputs are consumed; a borrow is not freed.

use std::cell::RefCell;
use std::rc::Rc;

use super::buf::BUF_MAX;
use super::chacha::{open_box, seal};
use super::curve::{sign, verify, x25519};
use super::error::{BOUNDS, INTEGRITY_FAIL, SIG_INVALID, TYPE_MISMATCH};
use super::value::{BufAlloc, Value};
use super::{RunError, Vm};

impl Vm {
    pub(super) fn take_buf(&mut self, op: &str) -> Result<Vec<u8>, RunError> {
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
                    "keys and messages are bufs",
                ))
            }
        }
    }

    fn push_buf(&mut self, data: Vec<u8>) {
        let n = data.len();
        self.push(Value::Buf {
            alloc: Rc::new(RefCell::new(BufAlloc {
                data: Some(data),
                n,
            })),
            exclusive: true,
        });
    }

    pub(super) fn bad_len(&self, op: &str, what: &str, n: usize) -> RunError {
        self.err(
            TYPE_MISMATCH,
            format!("`{op}` expected {what} of {n} bytes"),
            Some(op.to_string()),
            "",
        )
    }

    pub(super) fn sec(&self, code: &'static str, op: &str, msg: String, hint: &'static str) -> RunError {
        let mut e = self.err(code, msg, Some(op.to_string()), hint);
        e.kind = "sec";
        e
    }

    pub(super) fn b_ae(&mut self) -> Result<(), RunError> {
        let aad = self.take_buf("ae")?;
        let plain = self.take_buf("ae")?;
        let nonce = self.take_buf("ae")?;
        let key = self.take_buf("ae")?;
        if key.len() != 32 {
            return Err(self.bad_len("ae", "a key", 32));
        }
        if nonce.len() != 12 {
            return Err(self.bad_len("ae", "a nonce", 12));
        }
        if plain.len() + 16 > BUF_MAX as usize {
            return Err(self.err(
                BOUNDS,
                format!("`ae` seal is larger than {BUF_MAX} bytes"),
                Some("ae".into()),
                "",
            ));
        }
        self.push_buf(seal(&key, &nonce, &plain, &aad));
        Ok(())
    }

    pub(super) fn b_ad(&mut self) -> Result<(), RunError> {
        let aad = self.take_buf("ad")?;
        let boxed = self.take_buf("ad")?;
        let nonce = self.take_buf("ad")?;
        let key = self.take_buf("ad")?;
        if key.len() != 32 {
            return Err(self.bad_len("ad", "a key", 32));
        }
        if nonce.len() != 12 {
            return Err(self.bad_len("ad", "a nonce", 12));
        }
        if boxed.len() < 16 {
            return Err(self.bad_len("ad", "a seal", 16));
        }
        match open_box(&key, &nonce, &boxed, &aad) {
            Some(plain) => {
                self.push_buf(plain);
                Ok(())
            }
            None => Err(self.sec(
                INTEGRITY_FAIL,
                "ad",
                "`ad` tag does not match".into(),
                "the ciphertext, aad, key, or nonce changed",
            )),
        }
    }

    pub(super) fn b_es(&mut self) -> Result<(), RunError> {
        let message = self.take_buf("es")?;
        let seed = self.take_buf("es")?;
        if seed.len() != 32 {
            return Err(self.bad_len("es", "a seed", 32));
        }
        self.push_buf(sign(&seed, &message).to_vec());
        Ok(())
    }

    pub(super) fn b_ev(&mut self) -> Result<(), RunError> {
        let sig = self.take_buf("ev")?;
        let message = self.take_buf("ev")?;
        let public = self.take_buf("ev")?;
        if public.len() != 32 {
            return Err(self.bad_len("ev", "a public key", 32));
        }
        if sig.len() != 64 {
            return Err(self.bad_len("ev", "a signature", 64));
        }
        if !verify(&public, &message, &sig) {
            return Err(self.sec(
                SIG_INVALID,
                "ev",
                "`ev` signature is not valid".into(),
                "the signature, message, or public key does not match",
            ));
        }
        self.push(Value::Int(1));
        Ok(())
    }

    pub(super) fn b_xk(&mut self) -> Result<(), RunError> {
        let point = self.take_buf("xk")?;
        let scalar = self.take_buf("xk")?;
        if scalar.len() != 32 || point.len() != 32 {
            return Err(self.bad_len("xk", "a scalar and a point", 32));
        }
        self.push_buf(x25519(&scalar, &point).to_vec());
        Ok(())
    }
}
