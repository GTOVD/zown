//! In-memory graph, a BLAKE3 bundle, an Ed25519 trailer, and a transparency log.
//!
//! `so` checks the hash, then a signature when the bundle has one, then an
//! inclusion proof when the bundle has one. A BLAKE3-only bundle still opens.
//! CRDTs are not this log.

use std::collections::HashSet;

use super::blake3;
use super::curve::{public_key, sign, verify};
use super::error::{BOUNDS, INTEGRITY_FAIL, SIG_INVALID, TYPE_MISMATCH};
use super::{RunError, Value, Vm};

const HEADER: &str = "zown-sync 1\n";

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub verts: Vec<String>,
    pub edges: Vec<(String, String, String)>,
}

impl Graph {
    pub fn add_vert(&mut self, name: &str) {
        if !self.verts.iter().any(|v| v == name) {
            self.verts.push(name.to_string());
        }
    }

    pub fn add_edge(&mut self, src: &str, label: &str, dst: &str) {
        self.add_vert(src);
        self.add_vert(dst);
        let edge = (src.to_string(), label.to_string(), dst.to_string());
        if !self.edges.iter().any(|e| e == &edge) {
            self.edges.push(edge);
        }
    }

    pub fn snapshot(&self) -> String {
        if self.verts.is_empty() && self.edges.is_empty() {
            return String::new();
        }
        let mut lines = Vec::new();
        for name in &self.verts {
            lines.push(format!("v {name}"));
        }
        for (src, label, dst) in &self.edges {
            lines.push(format!("e {src} {label} {dst}"));
        }
        let mut text = lines.join("\n");
        text.push('\n');
        text
    }

    pub fn head(&self) -> String {
        blake3::hex(self.snapshot().as_bytes())
    }

    pub fn follow(&self, names: &[String], label: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for src in names {
            for (frm, lab, dst) in &self.edges {
                if frm == src && lab == label && seen.insert(dst.clone()) {
                    out.push(dst.clone());
                }
            }
        }
        out
    }
}

fn bundle_body(graph: &Graph) -> String {
    format!("{HEADER}{}", graph.snapshot())
}

pub fn seal(graph: &Graph) -> String {
    let body = bundle_body(graph);
    format!("{body}blake3 {}\n", blake3::hex(body.as_bytes()))
}

fn k_below(n: usize) -> usize {
    let mut k = 1usize;
    while (k << 1) < n {
        k <<= 1;
    }
    k
}

fn leaf_hash(digest: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(33);
    buf.push(0);
    buf.extend_from_slice(digest);
    blake3::hash(&buf)
}

fn node_hash(left: &[u8; 32], right: &[u8; 32]) -> [u8; 32] {
    let mut buf = Vec::with_capacity(65);
    buf.push(1);
    buf.extend_from_slice(left);
    buf.extend_from_slice(right);
    blake3::hash(&buf)
}

fn mth(leaves: &[[u8; 32]]) -> [u8; 32] {
    if leaves.len() == 1 {
        return leaf_hash(&leaves[0]);
    }
    let k = k_below(leaves.len());
    node_hash(&mth(&leaves[..k]), &mth(&leaves[k..]))
}

fn audit_path(leaves: &[[u8; 32]], index: usize) -> Vec<[u8; 32]> {
    let n = leaves.len();
    if n == 1 {
        return Vec::new();
    }
    let k = k_below(n);
    if index < k {
        let mut path = audit_path(&leaves[..k], index);
        path.push(mth(&leaves[k..]));
        path
    } else {
        let mut path = audit_path(&leaves[k..], index - k);
        path.push(mth(&leaves[..k]));
        path
    }
}

/// Fold a proof. The `usize` is how many sibling hashes were consumed.
fn fold(digest: &[u8; 32], index: usize, size: usize, path: &[[u8; 32]]) -> Option<([u8; 32], usize)> {
    if size == 1 {
        if index != 0 {
            return None;
        }
        return Some((leaf_hash(digest), 0));
    }
    let k = k_below(size);
    if index < k {
        let (acc, used) = fold(digest, index, k, path)?;
        let sib = path.get(used)?;
        Some((node_hash(&acc, sib), used + 1))
    } else {
        let (acc, used) = fold(digest, index - k, size - k, path)?;
        let sib = path.get(used)?;
        Some((node_hash(sib, &acc), used + 1))
    }
}

fn proof_ok(digest: &[u8; 32], index: usize, size: usize, path: &[[u8; 32]], root: &[u8; 32]) -> bool {
    if size == 0 || index >= size {
        return false;
    }
    match fold(digest, index, size, path) {
        Some((got, used)) => used == path.len() && &got == root,
        None => false,
    }
}

fn hex_bytes(data: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for byte in data {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if text.len() % 2 != 0 {
        return None;
    }
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(text.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = hex_nibble(bytes[i])?;
        let lo = hex_nibble(bytes[i + 1])?;
        out.push((hi << 4) | lo);
        i += 2;
    }
    Some(out)
}

fn hex_nibble(c: u8) -> Option<u8> {
    match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        b'A'..=b'F' => Some(c - b'A' + 10),
        _ => None,
    }
}

fn parse_dec(text: &str) -> Option<usize> {
    if text.is_empty() || !text.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

impl Vm {
    fn need_name(&self, op: &str, v: Value) -> Result<String, RunError> {
        match v {
            Value::Str(s) if !s.is_empty() && !s.chars().any(|c| c.is_whitespace()) => Ok(s),
            Value::Str(_) => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a single-word name, got a name with whitespace"),
                Some(op.to_string()),
                "names in the graph are one word, with no spaces",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a single-word name, got {tn}"),
                    Some(op.to_string()),
                    "names in the graph are one word, with no spaces",
                ))
            }
        }
    }

    pub(super) fn b_gv(&mut self) -> Result<(), RunError> {
        let raw = self.pop("gv")?;
        let name = self.need_name("gv", raw)?;
        self.graph.add_vert(&name);
        Ok(())
    }

    pub(super) fn b_ge(&mut self) -> Result<(), RunError> {
        let raw_dst = self.pop("ge")?;
        let raw_label = self.pop("ge")?;
        let raw_src = self.pop("ge")?;
        let dst = self.need_name("ge", raw_dst)?;
        let label = self.need_name("ge", raw_label)?;
        let src = self.need_name("ge", raw_src)?;
        self.graph.add_edge(&src, &label, &dst);
        Ok(())
    }

    pub(super) fn b_gs(&mut self) -> Result<(), RunError> {
        let text = self.graph.snapshot();
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_gl(&mut self) -> Result<(), RunError> {
        let text = match self.pop("gl")? {
            Value::Str(s) => s,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`gl` expected a snapshot string, got {tn}"),
                    Some("gl".into()),
                    "",
                ));
            }
        };
        self.graph = self.parse_snapshot("gl", &text)?;
        Ok(())
    }

    pub(super) fn b_gh(&mut self) -> Result<(), RunError> {
        let head = self.graph.head();
        self.push(Value::Str(head));
        Ok(())
    }

    pub(super) fn b_sb(&mut self) -> Result<(), RunError> {
        let text = seal(&self.graph);
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_so(&mut self) -> Result<(), RunError> {
        let text = match self.pop("so")? {
            Value::Str(s) => s,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`so` expected a bundle string, got {tn}"),
                    Some("so".into()),
                    "",
                ));
            }
        };
        self.graph = self.open_bundle(&text)?;
        let head = self.graph.head();
        self.push(Value::Str(head));
        Ok(())
    }

    pub(super) fn b_sl(&mut self) -> Result<(), RunError> {
        let seed = self.take_buf("sl")?;
        if seed.len() != 32 {
            return Err(self.bad_len("sl", "a seed", 32));
        }
        let body = bundle_body(&self.graph);
        let sig = sign(&seed, body.as_bytes());
        let public = public_key(&seed);
        let text = format!(
            "{}ed25519 {} {}\n",
            seal(&self.graph),
            hex_bytes(&public),
            hex_bytes(&sig)
        );
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn b_la(&mut self) -> Result<(), RunError> {
        let digest = blake3::hash(bundle_body(&self.graph).as_bytes());
        self.log.push(digest);
        self.push(Value::Int(self.log.len() as i64));
        Ok(())
    }

    pub(super) fn b_lp(&mut self) -> Result<(), RunError> {
        let digest = blake3::hash(bundle_body(&self.graph).as_bytes());
        let index = match self.log.iter().position(|leaf| leaf == &digest) {
            Some(index) => index,
            None => {
                return Err(self.err(
                    BOUNDS,
                    "`lp` has no log entry for this bundle".into(),
                    Some("lp".into()),
                    "append it with la first",
                ))
            }
        };
        let siblings = audit_path(&self.log, index);
        let mut line = format!("include {index} {}", self.log.len());
        for sib in &siblings {
            line.push(' ');
            line.push_str(&hex_bytes(sib));
        }
        line.push('\n');
        self.push(Value::Str(line));
        Ok(())
    }

    pub(super) fn b_lr(&mut self) -> Result<(), RunError> {
        let text = if self.log.is_empty() {
            String::new()
        } else {
            hex_bytes(&mth(&self.log))
        };
        self.push(Value::Str(text));
        Ok(())
    }

    pub(super) fn op_graph_open(&mut self) -> Result<(), RunError> {
        let raw = self.pop("»")?;
        let name = self.need_name("»", raw)?;
        let names = if self.graph.verts.iter().any(|v| v == &name) {
            vec![name]
        } else {
            Vec::new()
        };
        self.push(Value::Cur(names));
        Ok(())
    }

    pub(super) fn op_graph_follow(&mut self) -> Result<(), RunError> {
        let raw = self.pop("→")?;
        let label = self.need_name("→", raw)?;
        let cur = match self.pop("→")? {
            Value::Cur(names) => names,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`→` expected a cursor, got {tn}"),
                    Some("→".into()),
                    "open a query with »",
                ));
            }
        };
        let names = self.graph.follow(&cur, &label);
        self.push(Value::Cur(names));
        Ok(())
    }

    pub(super) fn op_graph_close(&mut self) -> Result<(), RunError> {
        let names = match self.pop("«")? {
            Value::Cur(names) => names,
            other => {
                let tn = other.type_name();
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`«` expected a cursor, got {tn}"),
                    Some("«".into()),
                    "",
                ));
            }
        };
        self.push(Value::Str(names.join(" ")));
        Ok(())
    }

    fn open_bundle(&self, text: &str) -> Result<Graph, RunError> {
        if !text.ends_with('\n') {
            return Err(self.bundle_shape());
        }
        let lines: Vec<&str> = text[..text.len() - 1].split('\n').collect();
        let mut found = None;
        for (i, line) in lines.iter().enumerate() {
            if line.starts_with("blake3 ") {
                found = Some(i);
            }
        }
        let Some(idx) = found else {
            return Err(self.bundle_shape());
        };
        let body = format!("{}\n", lines[..idx].join("\n"));
        let claimed = &lines[idx]["blake3 ".len()..];
        if !is_hex64(claimed) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected a blake3 trailer".into(),
                Some("so".into()),
                "",
            ));
        }
        let digest = blake3::hash(body.as_bytes());
        if blake3::hex(body.as_bytes()) != claimed.to_ascii_lowercase() {
            return Err(self.sec(
                INTEGRITY_FAIL,
                "so",
                "`so` bundle hash does not match its bytes".into(),
                "the bundle was changed after it was sealed",
            ));
        }
        if !body.starts_with(HEADER) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected a zown-sync 1 header".into(),
                Some("so".into()),
                "",
            ));
        }
        let mut rest = &lines[idx + 1..];
        let mut signed: Option<&str> = None;
        let mut included: Option<&str> = None;
        if rest.first().is_some_and(|line| line.starts_with("ed25519 ")) {
            signed = Some(rest[0]);
            rest = &rest[1..];
        }
        if rest.first().is_some_and(|line| line.starts_with("include ")) {
            included = Some(rest[0]);
            rest = &rest[1..];
        }
        if !rest.is_empty() {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected only an ed25519 line and an include line after blake3".into(),
                Some("so".into()),
                "",
            ));
        }
        if let Some(line) = signed {
            self.check_sig(&body, line)?;
        }
        if let Some(line) = included {
            self.check_proof(&digest, line)?;
        }
        self.parse_snapshot("so", &body[HEADER.len()..])
    }

    fn check_sig(&self, body: &str, line: &str) -> Result<(), RunError> {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts.len() != 3 || !is_hex_len(parts[1], 64) || !is_hex_len(parts[2], 128) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected an ed25519 public key and signature".into(),
                Some("so".into()),
                "",
            ));
        }
        let public = unhex(parts[1]).ok_or_else(|| self.err(
            TYPE_MISMATCH,
            "`so` expected an ed25519 public key and signature".into(),
            Some("so".into()),
            "",
        ))?;
        let sig = unhex(parts[2]).ok_or_else(|| self.err(
            TYPE_MISMATCH,
            "`so` expected an ed25519 public key and signature".into(),
            Some("so".into()),
            "",
        ))?;
        if !verify(&public, body.as_bytes(), &sig) {
            return Err(self.sec(
                SIG_INVALID,
                "so",
                "`so` signature is not valid".into(),
                "the signature, bundle, or public key does not match",
            ));
        }
        Ok(())
    }

    fn check_proof(&self, digest: &[u8; 32], line: &str) -> Result<(), RunError> {
        let parts: Vec<&str> = line.split(' ').collect();
        if parts.len() < 3 || parts[0] != "include" {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected an include proof".into(),
                Some("so".into()),
                "",
            ));
        }
        let (Some(index), Some(size)) = (parse_dec(parts[1]), parse_dec(parts[2])) else {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected an include index and tree size".into(),
                Some("so".into()),
                "",
            ));
        };
        let mut path = Vec::new();
        for token in &parts[3..] {
            if !is_hex64(token) {
                return Err(self.err(
                    TYPE_MISMATCH,
                    "`so` expected include siblings as 64 hex digits".into(),
                    Some("so".into()),
                    "",
                ));
            }
            let raw = unhex(token).ok_or_else(|| self.err(
                TYPE_MISMATCH,
                "`so` expected include siblings as 64 hex digits".into(),
                Some("so".into()),
                "",
            ))?;
            let mut sib = [0u8; 32];
            sib.copy_from_slice(&raw);
            path.push(sib);
        }
        if size < 1 || index >= size {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected an include index inside the tree".into(),
                Some("so".into()),
                "",
            ));
        }
        if self.log.is_empty() || size != self.log.len() || !proof_ok(digest, index, size, &path, &mth(&self.log)) {
            return Err(self.sec(
                INTEGRITY_FAIL,
                "so",
                "`so` inclusion proof does not match the log".into(),
                "the proof, the leaf, or this VM's log does not agree",
            ));
        }
        Ok(())
    }

    fn bundle_shape(&self) -> RunError {
        self.err(
            TYPE_MISMATCH,
            "`so` expected a zown-sync bundle".into(),
            Some("so".into()),
            "seal the graph with sb",
        )
    }

    fn parse_snapshot(&self, op: &str, text: &str) -> Result<Graph, RunError> {
        let mut graph = Graph::default();
        if text.is_empty() {
            return Ok(graph);
        }
        if !text.ends_with('\n') {
            return Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` snapshot must end with a newline"),
                Some(op.to_string()),
                "",
            ));
        }
        let body = &text[..text.len() - 1];
        for line in body.split('\n') {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() == 2 && parts[0] == "v" {
                let name = self.need_name(op, Value::Str(parts[1].to_string()))?;
                graph.add_vert(&name);
            } else if parts.len() == 4 && parts[0] == "e" {
                let src = self.need_name(op, Value::Str(parts[1].to_string()))?;
                let label = self.need_name(op, Value::Str(parts[2].to_string()))?;
                let dst = self.need_name(op, Value::Str(parts[3].to_string()))?;
                graph.add_edge(&src, &label, &dst);
            } else {
                return Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` cannot read snapshot line '{line}'"),
                    Some(op.to_string()),
                    "",
                ));
            }
        }
        Ok(graph)
    }
}

fn is_hex64(s: &str) -> bool {
    is_hex_len(s, 64)
}

fn is_hex_len(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Diag, Value, Vm, INTEGRITY_FAIL};

    #[test]
    fn empty_head_is_blake3_of_nothing() {
        assert_eq!(
            Graph::default().head(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
    }

    #[test]
    fn a_changed_bundle_is_integrity_fail() {
        let mut vm = Vm::new(None);
        vm.run_str("$a$ gv sb").unwrap();
        let Value::Str(bundle) = &vm.stack[0] else {
            panic!("sb pushes a string");
        };
        let bad = bundle.replacen('a', "b", 1);
        let literal = bad.replace('\n', "\\n");
        let mut vm = Vm::new(None);
        let err = match vm.run_str(&format!("${literal}$ so")) {
            Err(Diag::Run(e)) => e,
            other => panic!("expected a runtime fault, got {other:?}"),
        };
        assert_eq!(err.code, INTEGRITY_FAIL);
        assert_eq!(err.kind, "sec");
    }

    #[test]
    fn two_hop_query_matches_the_oracle_head() {
        let mut vm = Vm::new(None);
        vm.run_str(
            "$a$ gv\n$b$ gv\n$c$ gv\n$a$ $knows$ $b$ ge\n$b$ $knows$ $c$ ge\n\
             $a$ » $knows$ → « .\n$a$ » $knows$ → $knows$ → « .\n\
             $a$ » $nope$ → « .\ngh .\n",
        )
        .unwrap();
        assert_eq!(
            vm.out,
            "b\nc\n\ncb8d08bdb86a701fd4e9f1d104d1748e82d14c78702aab2b87c407da8061de82\n"
        );
    }
}
