//! Static checker, a port of `zown/checker.py`.
//!
//! A finding is a definite unhandled fault or a violation of a static rule the
//! SPEC mandates. Inside `ty`, faults are the handled path and stay silent.
//! Patterns are read, never executed. See the Python module for the full
//! contract; this file follows it branch for branch.

use std::collections::BTreeSet;

use zown_ast::{Node, NodeKind, Pos};
use zown_parser::{parse, ParseError};
use zown_vm::is_builtin;

const NAME_UNRESOLVED: &str = "NAME_UNRESOLVED";
const CAP_DENIED: &str = "CAP_DENIED";
const BAD_PATTERN: &str = "BAD_PATTERN";
const NO_MATCH: &str = "NO_MATCH";

const PATTERN_TYPES: &[&str] = &[
    "int", "float", "str", "block", "cap", "width", "vec", "big", "res", "color", "node", "cur", "buf",
];
const RES_PATTERNS: &[&str] = &["ok", "err"];
const VEC_CTORS: &[&str] = &["vf", "vd", "vi", "vu"];
const BOX_CTORS: &[&str] = &["rw", "cm"];
const INVOKING_OPS: &[&str] = &["@", ";", "??"];
const INVOKING_WORDS: &[&str] = &["gr", "ty", "vf", "vd", "vi", "vu", "rw", "cm"];

/// One static finding. `render_lines` is the conformance format.
#[derive(Debug, Clone)]
pub struct Diag {
    pub code: &'static str,
    pub msg: String,
    pub op: Option<String>,
    pub cap: Option<String>,
    pub hint: String,
    pub pos: Pos,
    pub file: Option<String>,
}

/// Golden form: `CODE`, `CODE op`, or `CODE op cap`, one finding per line.
/// An empty list renders as `ok`.
pub fn render_lines(diags: &[Diag]) -> String {
    if diags.is_empty() {
        return "ok\n".into();
    }
    let mut out = String::new();
    for d in diags {
        out.push_str(d.code);
        if let Some(op) = &d.op {
            out.push(' ');
            out.push_str(op);
        }
        if let Some(cap) = &d.cap {
            out.push(' ');
            out.push_str(cap);
        }
        out.push('\n');
    }
    out
}

/// Parse and check. A lex/parse error is returned; findings are a (possibly
/// empty) list.
pub fn check_src(src: &str, file: Option<&str>) -> Result<Vec<Diag>, ParseError> {
    let nodes = parse(src)?;
    Ok(Checker::new(file).check(&nodes))
}

struct Checker {
    file: Option<String>,
    diags: Vec<Diag>,
    bound: BTreeSet<String>,
    grantable: BTreeSet<String>,
    uses_at: bool,
}

impl Checker {
    fn new(file: Option<&str>) -> Self {
        Checker {
            file: file.map(str::to_string),
            diags: Vec::new(),
            bound: BTreeSet::new(),
            grantable: BTreeSet::new(),
            uses_at: false,
        }
    }

    fn check(mut self, nodes: &[Node]) -> Vec<Diag> {
        self.collect_binds(nodes);
        self.collect_grantable(nodes);
        self.uses_at = self.uses_word(nodes, "ty");
        let none = BTreeSet::new();
        self.walk(nodes, Some(&none), false, None);
        self.diags
    }

    fn diag(
        &mut self,
        code: &'static str,
        msg: String,
        pos: Pos,
        op: Option<String>,
        cap: Option<String>,
        hint: &str,
    ) {
        self.diags.push(Diag {
            code,
            msg,
            op,
            cap,
            hint: hint.to_string(),
            pos,
            file: self.file.clone(),
        });
    }

    fn is_word(&self, node: Option<&Node>, word: &str) -> bool {
        match node {
            Some(Node {
                kind: NodeKind::Name(n),
                ..
            }) => n == word && !self.bound.contains(word),
            _ => false,
        }
    }

    fn is_op(node: Option<&Node>, sym: &str) -> bool {
        matches!(node, Some(Node { kind: NodeKind::Op(s), .. }) if s == sym)
    }

    fn uses_word(&self, nodes: &[Node], word: &str) -> bool {
        for n in nodes {
            if self.is_word(Some(n), word) {
                return true;
            }
            if let NodeKind::Blk(inner) = &n.kind {
                if self.uses_word(inner, word) {
                    return true;
                }
            }
        }
        false
    }

    fn collect_binds(&mut self, nodes: &[Node]) {
        for n in nodes {
            match &n.kind {
                NodeKind::Bind(name) => {
                    self.bound.insert(name.clone());
                }
                NodeKind::Blk(inner) => self.collect_binds(inner),
                _ => {}
            }
        }
    }

    fn collect_grantable(&mut self, nodes: &[Node]) {
        if self.bound.contains("gr") {
            return;
        }
        self.scan_free_caps(nodes);
    }

    fn scan_free_caps(&mut self, nodes: &[Node]) {
        for (i, n) in nodes.iter().enumerate() {
            if let NodeKind::Cap(name) = &n.kind {
                let nxt = at(nodes, i as isize + 1);
                if !(self.is_word(nxt, "rq") || self.is_word(nxt, "hv")) {
                    self.grantable.insert(name.clone());
                }
            } else if let NodeKind::Blk(inner) = &n.kind {
                self.scan_free_caps(inner);
            }
        }
    }

    fn walk(
        &mut self,
        nodes: &[Node],
        known: Option<&BTreeSet<String>>,
        in_at: bool,
        moved_in: Option<&BTreeSet<String>>,
    ) {
        let mut moved = moved_in.cloned().unwrap_or_default();
        for (i, n) in nodes.iter().enumerate() {
            match &n.kind {
                NodeKind::Name(name) => {
                    if !self.bound.contains(name) && !is_builtin(name) {
                        self.diag(
                            NAME_UNRESOLVED,
                            format!("`{name}` is never bound and is not a builtin"),
                            n.pos,
                            Some(name.clone()),
                            None,
                            "bind it with `:name`, or check the stdlib word list",
                        );
                    } else if moved.contains(name) && !is_builtin(name) && !in_at {
                        self.diag(
                            NAME_UNRESOLVED,
                            format!("`{name}` was moved out by '{name} above and not rebound"),
                            n.pos,
                            Some(name.clone()),
                            None,
                            "a bare name is a borrow; move it last, or rebind it",
                        );
                    }
                }
                NodeKind::Move(name) => {
                    if !self.bound.contains(name) {
                        self.diag(
                            NAME_UNRESOLVED,
                            format!("`'{name}`: {name} is never bound (builtins cannot be moved)"),
                            n.pos,
                            Some(format!("'{name}")),
                            None,
                            "bind it first: value :name",
                        );
                    } else if moved.contains(name) && !in_at {
                        self.diag(
                            NAME_UNRESOLVED,
                            format!("`'{name}`: {name} was already moved out above"),
                            n.pos,
                            Some(format!("'{name}")),
                            None,
                            "rebind it before moving it again",
                        );
                    }
                    moved.insert(name.clone());
                }
                NodeKind::Bind(name) => {
                    moved.remove(name);
                }
                NodeKind::Cap(name) => {
                    let nxt = at(nodes, i as isize + 1);
                    if self.is_word(nxt, "rq") && !in_at {
                        let pos = nxt.unwrap().pos;
                        self.check_require(name, pos, known);
                    }
                }
                NodeKind::Blk(_) => self.walk_block(nodes, i, known, in_at, &moved),
                _ => {}
            }
            let invoking = match &n.kind {
                NodeKind::Op(op) => INVOKING_OPS.contains(&op.as_str()),
                NodeKind::Name(name) => {
                    INVOKING_WORDS.contains(&name.as_str()) && !self.bound.contains(name)
                }
                _ => false,
            };
            if invoking {
                moved.clear();
            }
        }
    }

    fn walk_block(
        &mut self,
        nodes: &[Node],
        i: usize,
        known: Option<&BTreeSet<String>>,
        mut in_at: bool,
        moved: &BTreeSet<String>,
    ) {
        let blk = &nodes[i];
        let NodeKind::Blk(children) = &blk.kind else {
            return;
        };
        let ii = i as isize;
        let prev = at(nodes, ii - 1);
        let prev2 = at(nodes, ii - 2);
        let nxt = at(nodes, ii + 1);
        let nxt2 = at(nodes, ii + 2);
        let nxt3 = at(nodes, ii + 3);

        if Self::is_op(nxt, "??") {
            let subject_is_res = self.is_word(prev, "ty")
                && matches!(
                    prev2,
                    Some(Node {
                        kind: NodeKind::Blk(_),
                        ..
                    })
                );
            self.check_match_arms(blk, children, known, in_at, subject_is_res, moved);
            return;
        }

        let mut inherit: Option<&BTreeSet<String>> = Some(moved);
        let inner: Option<BTreeSet<String>>;
        if Self::is_op(nxt, "@") {
            inner = known.cloned();
        } else if Self::is_op(nxt, ";") {
            inner = known.cloned();
            inherit = None;
        } else if matches!(
            nxt,
            Some(Node {
                kind: NodeKind::Blk(_),
                ..
            })
        ) && Self::is_op(nxt2, ";")
        {
            inner = known.cloned();
        } else if Self::is_op(nxt, "?") && Self::is_op(nxt2, "@") {
            inner = known.cloned();
        } else if matches!(
            nxt,
            Some(Node {
                kind: NodeKind::Blk(_),
                ..
            })
        ) && Self::is_op(nxt2, "?")
            && Self::is_op(nxt3, "@")
        {
            inner = known.cloned();
        } else if self.is_word(nxt, "gr") {
            if let (
                Some(Node {
                    kind: NodeKind::Cap(cap),
                    ..
                }),
                Some(known),
            ) = (prev, known)
            {
                let mut set = known.clone();
                set.insert(cap.clone());
                inner = Some(set);
            } else {
                inner = None;
            }
        } else if self.is_word(nxt, "ty") {
            inner = known.cloned();
            in_at = true;
        } else if let Some(Node {
            kind: NodeKind::Name(name),
            ..
        }) = nxt
        {
            if (VEC_CTORS.contains(&name.as_str()) || BOX_CTORS.contains(&name.as_str()))
                && !self.bound.contains(name)
            {
                inner = known.cloned();
            } else {
                inner = None;
                inherit = None;
            }
        } else {
            inner = None;
            inherit = None;
        }
        self.walk(children, inner.as_ref(), in_at, inherit);
    }

    fn check_require(&mut self, cap: &str, pos: Pos, known: Option<&BTreeSet<String>>) {
        if let Some(known) = known {
            if !known.contains(cap) {
                let held = if known.is_empty() {
                    "nothing".to_string()
                } else {
                    known
                        .iter()
                        .map(|c| format!("`{c}"))
                        .collect::<Vec<_>>()
                        .join(" ")
                };
                self.diag(
                    CAP_DENIED,
                    format!(
                        "`{cap} is required here but the authority in scope is exactly {held}; this `rq` can never succeed"
                    ),
                    pos,
                    Some("rq".into()),
                    Some(cap.to_string()),
                    "run it inside a grant",
                );
                return;
            }
        }
        if !self.grantable.contains(cap) && !self.uses_at {
            self.diag(
                CAP_DENIED,
                format!("`{cap} is required but never granted anywhere in the program"),
                pos,
                Some("rq".into()),
                Some(cap.to_string()),
                "add a grant scope around the caller",
            );
        }
    }

    fn check_match_arms(
        &mut self,
        blk: &Node,
        children: &[Node],
        known: Option<&BTreeSet<String>>,
        in_at: bool,
        subject_is_res: bool,
        moved: &BTreeSet<String>,
    ) {
        if children.len() % 2 != 0 {
            self.diag(
                BAD_PATTERN,
                format!(
                    "match arms are [pattern][body] pairs; got {} blocks",
                    children.len()
                ),
                blk.pos,
                Some("??".into()),
                None,
                "",
            );
            return;
        }
        let mut seen: BTreeSet<String> = BTreeSet::new();
        for pair in children.chunks(2) {
            let (pat, body) = (&pair[0], &pair[1]);
            match (&pat.kind, &body.kind) {
                (NodeKind::Blk(pnodes), NodeKind::Blk(bnodes)) => {
                    if let Some(label) = self.check_pattern(pat, pnodes) {
                        seen.insert(label);
                    }
                    self.walk(bnodes, known, in_at, Some(moved));
                }
                _ => self.diag(
                    BAD_PATTERN,
                    "each match arm is [pattern] [body], both blocks".into(),
                    blk.pos,
                    Some("??".into()),
                    None,
                    "",
                ),
            }
        }
        let covered = seen.contains("any")
            || seen.contains("res")
            || (seen.contains("ok") && seen.contains("err"));
        if subject_is_res && !covered {
            let mut missing = Vec::new();
            if !seen.contains("ok") {
                missing.push("ok");
            }
            if !seen.contains("err") {
                missing.push("err");
            }
            missing.sort();
            self.diag(
                NO_MATCH,
                format!(
                    "a result must be handled exhaustively: no arm for {}",
                    missing.join(" and ")
                ),
                blk.pos,
                Some("??".into()),
                None,
                "add [err] and [ok] arms, or a default [any]",
            );
        }
    }

    fn check_pattern(&mut self, pat: &Node, nodes: &[Node]) -> Option<String> {
        if nodes.len() != 1 {
            self.diag(
                BAD_PATTERN,
                format!("a pattern holds exactly one token, got {}", nodes.len()),
                pat.pos,
                Some("??".into()),
                None,
                "use [int], [42], [$txt$], [ok], or [any]",
            );
            return None;
        }
        let n = &nodes[0];
        match &n.kind {
            NodeKind::Name(s) if s == "any" => Some("any".into()),
            NodeKind::Int(_) | NodeKind::Float(_) | NodeKind::Str(_) => Some("lit".into()),
            NodeKind::Name(s)
                if PATTERN_TYPES.contains(&s.as_str()) || RES_PATTERNS.contains(&s.as_str()) =>
            {
                Some(s.clone())
            }
            other => {
                let label = match other {
                    NodeKind::Name(s) | NodeKind::Op(s) | NodeKind::Cap(s) => s.clone(),
                    NodeKind::Bind(s) => format!(":{s}"),
                    NodeKind::Move(s) => format!("'{s}"),
                    NodeKind::Blk(_) => "blk".into(),
                    NodeKind::Int(v) => v.to_string(),
                    NodeKind::Float(v) => v.to_string(),
                    NodeKind::Str(s) => s.clone(),
                };
                self.diag(
                    BAD_PATTERN,
                    format!("unrecognized pattern `{label}`"),
                    pat.pos,
                    Some("??".into()),
                    None,
                    "a pattern is a type name, a literal, ok/err, or any",
                );
                None
            }
        }
    }
}

fn at(nodes: &[Node], i: isize) -> Option<&Node> {
    if i < 0 {
        None
    } else {
        nodes.get(i as usize)
    }
}
