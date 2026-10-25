//! Structured runtime diagnostics (`.zerr` packets), parity with `zown/errors.py`.
//!
//! Runtime packets carry `pos` from the AST (M8d-i). Conformance compares the
//! recovery `code` and offending `op`.

use crate::value::json_str;
use zown_ast::Pos;

pub const ZERR_VERSION: &str = "0.2";

pub const STACK_UNDERFLOW: &str = "STACK_UNDERFLOW";
pub const TYPE_MISMATCH: &str = "TYPE_MISMATCH";
pub const NAME_UNRESOLVED: &str = "NAME_UNRESOLVED";
pub const DIV_ZERO: &str = "DIV_ZERO";
pub const NOT_CALLABLE: &str = "NOT_CALLABLE";
pub const UNSUPPORTED: &str = "UNSUPPORTED";
pub const OVERFLOW: &str = "OVERFLOW";
pub const UB_TRAP: &str = "UB_TRAP";
pub const BOUNDS: &str = "BOUNDS";
pub const NO_MATCH: &str = "NO_MATCH";
pub const BAD_PATTERN: &str = "BAD_PATTERN";
pub const STACK_OVERFLOW: &str = "STACK_OVERFLOW";
pub const CAP_DENIED: &str = "CAP_DENIED";
pub const TEST_FAIL: &str = "TEST_FAIL";
pub const INTEGRITY_FAIL: &str = "INTEGRITY_FAIL";
pub const SIG_INVALID: &str = "SIG_INVALID";

#[derive(Debug, Clone)]
pub struct RunError {
    pub code: &'static str,
    pub msg: String,
    pub op: Option<String>,
    pub hint: &'static str,
    /// JSON fragments captured from the operand stack at fault time.
    pub stack: Vec<String>,
    pub file: Option<String>,
    pub pos: Option<Pos>,
    /// `run` for ordinary faults, `sec` for capability denials and integrity failures.
    pub kind: &'static str,
    /// Set on security packets only; omitted from the JSON otherwise.
    pub cap: Option<String>,
}

impl RunError {
    pub fn to_json(&self) -> String {
        let op = match &self.op {
            Some(o) => json_str(o),
            None => "null".to_string(),
        };
        let file = match &self.file {
            Some(f) => json_str(f),
            None => "null".to_string(),
        };
        let pos = match self.pos {
            Some(p) => format!(
                "{{\"line\": {}, \"col\": {}, \"offset\": {}}}",
                p.line, p.col, p.offset
            ),
            None => "null".into(),
        };
        let cap = match &self.cap {
            Some(c) => format!(",\n  \"cap\": {}", json_str(c)),
            None => String::new(),
        };
        format!(
            "{{\n  \"zerr\": \"{ver}\",\n  \"kind\": \"{kind}\",\n  \"code\": \"{code}\",\n  \"msg\": {msg},\n  \"op\": {op},\n  \"pos\": {pos},\n  \"stack\": [{stack}],\n  \"hint\": {hint},\n  \"file\": {file}{cap}\n}}",
            ver = ZERR_VERSION,
            kind = self.kind,
            code = self.code,
            msg = json_str(&self.msg),
            op = op,
            stack = self.stack.join(", "),
            hint = json_str(self.hint),
            file = file,
            cap = cap,
        )
    }

    pub fn render_human(&self) -> String {
        let op = self
            .op
            .as_ref()
            .map(|o| format!(" (op `{o}`)"))
            .unwrap_or_default();
        let loc = self.file.as_deref().unwrap_or("");
        let head = if loc.is_empty() {
            format!("zerr[{}]", self.code)
        } else {
            format!("zerr[{}] {}", self.code, loc)
        };
        let mut out = format!("{head}{op}: {}", self.msg);
        if !self.hint.is_empty() {
            out.push_str(&format!("\n  hint: {}", self.hint));
        }
        out
    }
}
