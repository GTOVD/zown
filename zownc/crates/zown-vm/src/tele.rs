//! Logical control-plane log (SPEC §27). Same lines as `zown/tele.py`.

use std::collections::HashMap;

use super::error::TYPE_MISMATCH;
use super::{RunError, Value, Vm};

#[derive(Clone, Debug)]
pub struct Telemetry {
    pub content: String,
    pub version: String,
    pub trace: String,
    spans: Vec<String>,
    wants: Vec<(String, String)>,
    have: HashMap<String, String>,
    remedy: HashMap<String, String>,
    events: Vec<String>,
    seq: u64,
}

impl Default for Telemetry {
    fn default() -> Self {
        Telemetry {
            content: "-".into(),
            version: "-".into(),
            trace: "-".into(),
            spans: Vec::new(),
            wants: Vec::new(),
            have: HashMap::new(),
            remedy: HashMap::new(),
            events: Vec::new(),
            seq: 0,
        }
    }
}

impl Telemetry {
    pub fn line(&mut self, kind: &str, rest: &str) {
        self.seq += 1;
        let line = if rest.is_empty() {
            format!(
                "{} {} {} {} {kind}",
                self.seq, self.content, self.version, self.trace
            )
        } else {
            format!(
                "{} {} {} {} {kind} {rest}",
                self.seq, self.content, self.version, self.trace
            )
        };
        self.events.push(line);
    }

    pub fn text(&self) -> String {
        self.events.join("\n")
    }

    pub fn fault(&mut self, code: &str, op: Option<&str>) {
        let note = self.remedy.get(code).map(String::as_str).unwrap_or("-");
        let op = op.unwrap_or("-");
        let rest = format!("{code} {op} {note}");
        self.line("fault", &rest);
    }

    pub fn drift(&self) -> String {
        let mut lines = Vec::new();
        for (name, want) in &self.wants {
            match self.have.get(name) {
                Some(have) if have == want => {}
                Some(have) => lines.push(format!("{name} {want} {have}")),
                None => lines.push(format!("{name} {want} -")),
            }
        }
        if lines.is_empty() {
            "ok".into()
        } else {
            lines.join("\n")
        }
    }

    fn set_want(&mut self, name: String, value: String) {
        if let Some(slot) = self.wants.iter_mut().find(|(n, _)| n == &name) {
            slot.1 = value;
        } else {
            self.wants.push((name, value));
        }
    }
}

impl Vm {
    fn need_word(&self, op: &str, v: Value) -> Result<String, RunError> {
        match v {
            Value::Str(s) if !s.is_empty() && !s.chars().any(|c| c.is_whitespace()) => Ok(s),
            Value::Str(_) => Err(self.err(
                TYPE_MISMATCH,
                format!("`{op}` expected a single-word name, got a name with whitespace"),
                Some(op.to_string()),
                "this field is one word, with no spaces",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`{op}` expected a single-word name, got {tn}"),
                    Some(op.to_string()),
                    "this field is one word, with no spaces",
                ))
            }
        }
    }

    fn need_note(&self, v: Value) -> Result<String, RunError> {
        match v {
            Value::Str(s) if !s.is_empty() && !s.chars().any(|c| c == '\n' || c == '\r') => Ok(s),
            Value::Str(s) if s.is_empty() => Err(self.err(
                TYPE_MISMATCH,
                "`nt` expected a one-line note, got an empty string".into(),
                Some("nt".into()),
                "",
            )),
            Value::Str(_) => Err(self.err(
                TYPE_MISMATCH,
                "`nt` expected a one-line note, got a string with a newline".into(),
                Some("nt".into()),
                "",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`nt` expected a one-line note, got {tn}"),
                    Some("nt".into()),
                    "",
                ))
            }
        }
    }

    fn need_hex(&self, v: Value) -> Result<String, RunError> {
        match v {
            Value::Str(s) if is_hex64(&s) => Ok(s.to_ascii_lowercase()),
            Value::Str(_) => Err(self.err(
                TYPE_MISMATCH,
                "`ch` expected 64 hex digits, got a string that is not 64 hex digits".into(),
                Some("ch".into()),
                "hash the text with h3 first",
            )),
            other => {
                let tn = other.type_name();
                Err(self.err(
                    TYPE_MISMATCH,
                    format!("`ch` expected 64 hex digits, got {tn}"),
                    Some("ch".into()),
                    "hash the text with h3 first",
                ))
            }
        }
    }

    pub(super) fn b_ch(&mut self) -> Result<(), RunError> {
        let raw = self.pop("ch")?;
        let hex = self.need_hex(raw)?;
        self.tele.borrow_mut().content = hex;
        Ok(())
    }

    pub(super) fn b_vr(&mut self) -> Result<(), RunError> {
        let raw = self.pop("vr")?;
        let word = self.need_word("vr", raw)?;
        self.tele.borrow_mut().version = word;
        Ok(())
    }

    pub(super) fn b_td(&mut self) -> Result<(), RunError> {
        let raw = self.pop("td")?;
        let word = self.need_word("td", raw)?;
        self.tele.borrow_mut().trace = word;
        Ok(())
    }

    pub(super) fn b_sp(&mut self) -> Result<(), RunError> {
        let raw = self.pop("sp")?;
        let name = self.need_word("sp", raw)?;
        let mut tele = self.tele.borrow_mut();
        tele.spans.push(name.clone());
        tele.line("open", &name);
        Ok(())
    }

    pub(super) fn b_se(&mut self) -> Result<(), RunError> {
        let empty = self.tele.borrow().spans.is_empty();
        if empty {
            return Err(self.err(
                TYPE_MISMATCH,
                "`se` has no open span".into(),
                Some("se".into()),
                "open one with $name$ sp",
            ));
        }
        let mut tele = self.tele.borrow_mut();
        let name = tele.spans.pop().unwrap();
        tele.line("close", &name);
        Ok(())
    }

    pub(super) fn b_nt(&mut self) -> Result<(), RunError> {
        let raw = self.pop("nt")?;
        let text = self.need_note(raw)?;
        self.tele.borrow_mut().line("note", &text);
        Ok(())
    }

    pub(super) fn b_wn(&mut self) -> Result<(), RunError> {
        let raw_value = self.pop("wn")?;
        let raw_name = self.pop("wn")?;
        let value = self.need_word("wn", raw_value)?;
        let name = self.need_word("wn", raw_name)?;
        let mut tele = self.tele.borrow_mut();
        tele.set_want(name.clone(), value.clone());
        tele.line("want", &format!("{name} {value}"));
        Ok(())
    }

    pub(super) fn b_ob(&mut self) -> Result<(), RunError> {
        let raw_value = self.pop("ob")?;
        let raw_name = self.pop("ob")?;
        let value = self.need_word("ob", raw_value)?;
        let name = self.need_word("ob", raw_name)?;
        let mut tele = self.tele.borrow_mut();
        tele.have.insert(name.clone(), value.clone());
        tele.line("obs", &format!("{name} {value}"));
        let want = tele
            .wants
            .iter()
            .find(|(n, _)| n == &name)
            .map(|(_, w)| w.clone());
        if let Some(want) = want {
            if want == value {
                tele.line("meet", &name);
            } else {
                tele.line("drift", &format!("{name} {want} {value}"));
            }
        }
        Ok(())
    }

    pub(super) fn b_df(&mut self) -> Result<(), RunError> {
        let report = self.tele.borrow().drift();
        self.push(Value::Str(report));
        Ok(())
    }

    pub(super) fn b_rm(&mut self) -> Result<(), RunError> {
        let raw_note = self.pop("rm")?;
        let raw_code = self.pop("rm")?;
        let note = self.need_word("rm", raw_note)?;
        let code = self.need_word("rm", raw_code)?;
        self.tele.borrow_mut().remedy.insert(code, note);
        Ok(())
    }

    pub(super) fn b_lg(&mut self) -> Result<(), RunError> {
        let text = self.tele.borrow().text();
        self.push(Value::Str(text));
        Ok(())
    }
}

fn is_hex64(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F'))
}

#[cfg(test)]
mod tests {
    use crate::Vm;

    #[test]
    fn intent_log_matches_the_oracle() {
        let mut vm = Vm::new(None);
        vm.run_str(include_str!("../../../../conformance/cases_v2/intent.zn"))
            .unwrap();
        assert_eq!(
            vm.out,
            include_str!("../../../../conformance/cases_v2/intent.out")
        );
    }
}
