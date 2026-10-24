//! In-memory graph and a BLAKE3-sealed bundle (SPEC §26).
//!
//! The durable form is the bundle text. `so` refuses it when the digest does
//! not match. Ed25519 signatures and CRDTs are not this log.

use std::collections::HashSet;

use super::blake3;
use super::error::{INTEGRITY_FAIL, TYPE_MISMATCH};
use super::{RunError, Value, Vm};

const HEADER: &str = "zown-sync 1\n";
const MARKER: &str = "\nblake3 ";

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

pub fn seal(graph: &Graph) -> String {
    let body = format!("{HEADER}{}", graph.snapshot());
    format!("{body}blake3 {}\n", blake3::hex(body.as_bytes()))
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
        let Some(idx) = text.rfind(MARKER) else {
            return Err(self.bundle_shape());
        };
        if !text.ends_with('\n') {
            return Err(self.bundle_shape());
        }
        let body = &text[..idx + 1];
        let claimed = &text[idx + MARKER.len()..text.len() - 1];
        if !is_hex64(claimed) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected a blake3 trailer".into(),
                Some("so".into()),
                "",
            ));
        }
        if blake3::hex(body.as_bytes()) != claimed.to_ascii_lowercase() {
            let mut e = self.err(
                INTEGRITY_FAIL,
                "`so` bundle hash does not match its bytes".into(),
                Some("so".into()),
                "the bundle was changed after it was sealed",
            );
            e.kind = "sec";
            return Err(e);
        }
        if !body.starts_with(HEADER) {
            return Err(self.err(
                TYPE_MISMATCH,
                "`so` expected a zown-sync 1 header".into(),
                Some("so".into()),
                "",
            ));
        }
        self.parse_snapshot("so", &body[HEADER.len()..])
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
    s.len() == 64
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
