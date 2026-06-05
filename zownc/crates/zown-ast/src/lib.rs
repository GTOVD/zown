//! Zown AST.
//!
//! Mirrors the node forms of the Python reference (`zown/parser.py`). The JSON
//! rendering is intentionally shape-compatible with `zown ast` so the two
//! frontends can be differentially tested (see `docs/PLAN.md` M3).

/// A program node. Blocks nest a sub-program.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    Int(i64),
    Float(f64),
    Str(String),
    /// Push the value bound to this name (or run a builtin word).
    Name(String),
    /// `:name` — pop the top of stack and bind it to this name.
    Bind(String),
    /// An operator symbol.
    Op(String),
    /// `[ ... ]` — a first-class block (quotation).
    Blk(Vec<Node>),
}

impl Node {
    fn tag(&self) -> &'static str {
        match self {
            Node::Int(_) => "int",
            Node::Float(_) => "float",
            Node::Str(_) => "str",
            Node::Name(_) => "name",
            Node::Bind(_) => "bind",
            Node::Op(_) => "op",
            Node::Blk(_) => "blk",
        }
    }
}

/// Render a program to JSON matching the Python `zown ast` shape: an array whose
/// elements are single-key objects, e.g. `{"op": "+"}` or `{"blk": [ ... ]}`.
pub fn to_json(nodes: &[Node]) -> String {
    let mut out = String::new();
    render_array(nodes, 0, &mut out);
    out
}

fn render_array(nodes: &[Node], indent: usize, out: &mut String) {
    if nodes.is_empty() {
        out.push_str("[]");
        return;
    }
    out.push_str("[\n");
    let pad = "  ".repeat(indent + 1);
    for (i, n) in nodes.iter().enumerate() {
        out.push_str(&pad);
        render_node(n, indent + 1, out);
        if i + 1 < nodes.len() {
            out.push(',');
        }
        out.push('\n');
    }
    out.push_str(&"  ".repeat(indent));
    out.push(']');
}

fn render_node(n: &Node, indent: usize, out: &mut String) {
    out.push_str("{\n");
    let pad = "  ".repeat(indent + 1);
    out.push_str(&pad);
    out.push('"');
    out.push_str(n.tag());
    out.push_str("\": ");
    match n {
        Node::Int(v) => out.push_str(&v.to_string()),
        Node::Float(v) => out.push_str(&fmt_float(*v)),
        Node::Str(s) | Node::Name(s) | Node::Bind(s) | Node::Op(s) => out.push_str(&json_str(s)),
        Node::Blk(inner) => render_array(inner, indent + 1, out),
    }
    out.push('\n');
    out.push_str(&"  ".repeat(indent));
    out.push('}');
}

/// Format a float the way Python's `json` does for our cases (integers as `x.0`).
fn fmt_float(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{v:.1}")
    } else {
        format!("{v}")
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
