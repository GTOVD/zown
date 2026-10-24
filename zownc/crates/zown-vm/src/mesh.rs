//! Local content store and node identity. The UDP exchange lives in `zown/mesh.py`;
//! both VMs share the BLAKE3 address and the `~p` / `~f` store.

use super::blake3::{hash, hex};
use super::error::{BOUNDS, TYPE_MISMATCH};
use super::{RunError, Value, Vm};

pub fn node_id() -> [u8; 32] {
    if let Ok(raw) = std::env::var("ZOWN_NODE") {
        if !raw.is_empty() {
            return hash(raw.as_bytes());
        }
    }
    random_id()
}

fn random_id() -> [u8; 32] {
    if let Ok(mut file) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let mut buf = [0u8; 32];
        if file.read_exact(&mut buf).is_ok() {
            return buf;
        }
    }
    hash(b"zown-node-fallback")
}

pub fn ula(id: &[u8; 32]) -> String {
    let mut raw = [0u8; 16];
    raw.copy_from_slice(&id[..16]);
    raw[0] = 0xfd;
    let mut out = String::new();
    for i in 0..8 {
        if i > 0 {
            out.push(':');
        }
        let value = u16::from_be_bytes([raw[i * 2], raw[i * 2 + 1]]);
        out.push_str(&format!("{value:04x}"));
    }
    out
}

fn parse_hash(text: &str) -> Option<String> {
    if text.len() != 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(text.to_ascii_lowercase())
}

impl Vm {
    pub(super) fn b_h3(&mut self) -> Result<(), RunError> {
        match self.pop("h3")? {
            Value::Str(s) => {
                let digest = hex(s.as_bytes());
                self.push(Value::Str(digest));
                Ok(())
            }
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`h3` expected a string, got {tn}"),
                    Some("h3".into()),
                    "BLAKE3 hashes the string's UTF-8 bytes",
                ))
            }
        }
    }

    pub(super) fn b_ni(&mut self) -> Result<(), RunError> {
        let text = hex_bytes(&self.node_id);
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_ua(&mut self) -> Result<(), RunError> {
        self.push(Value::Str(ula(&self.node_id)));
        Ok(())
    }

    pub(super) fn op_publish(&mut self) -> Result<(), RunError> {
        match self.pop("~p")? {
            Value::Str(s) => {
                let digest = hex(s.as_bytes());
                self.bundles.insert(digest.clone(), s);
                self.push(Value::Str(digest));
                Ok(())
            }
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`~p` expected a string, got {tn}"),
                    Some("~p".into()),
                    "publish the bundle's bytes as a string",
                ))
            }
        }
    }

    pub(super) fn op_fetch(&mut self) -> Result<(), RunError> {
        let text = match self.pop("~f")? {
            Value::Str(s) => s,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`~f` expected a 64-digit BLAKE3 hex hash, got {tn}"),
                    Some("~f".into()),
                    "publish first; ~p pushes the hash",
                ));
            }
        };
        let Some(digest) = parse_hash(&text) else {
            return Err(self.err(
                TYPE_MISMATCH,
                "`~f` expected a 64-digit BLAKE3 hex hash".into(),
                Some("~f".into()),
                "publish first; ~p pushes the hash",
            ));
        };
        let Some(body) = self.bundles.get(&digest).cloned() else {
            return Err(self.err(
                BOUNDS,
                "`~f` has no bundle for that hash".into(),
                Some("~f".into()),
                "the hash is the address; publish it with ~p",
            ));
        };
        self.push(Value::Str(body));
        Ok(())
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}
