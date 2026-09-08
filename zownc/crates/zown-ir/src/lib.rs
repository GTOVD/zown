//! Zown IR — a small, explicit instruction form the backends consume.
//!
//! The AST is convenient for the frontend but tree-shaped; codegen wants flat,
//! linear instruction sequences with blocks separated into an addressable table.
//! The IR provides exactly that (milestone M5 in `docs/PLAN.md`):
//!
//!   * A `IrProgram` is a table of `IrBlock`s plus the `main` block id.
//!   * Each block is a flat `Vec<Instr>`.
//!   * `[ ... ]` quotations become separate blocks; `ConstBlock(id)` pushes a
//!     reference to one (mirroring how the VM treats blocks as first-class
//!     values).
//!
//! Lowering is a lossless, structural flattening of the AST. `unlower` rebuilds
//! the exact AST, which is how we verify the IR faithfully represents a program
//! before any backend trusts it (see `conformance/ir_roundtrip.py`).

use zown_ast::{Node, NodeKind, Pos};

/// One IR instruction.
#[derive(Debug, Clone, PartialEq)]
pub enum Instr {
    ConstInt(i64),
    ConstFloat(f64),
    ConstStr(String),
    /// Push a reference to block `id` in the program's block table.
    ConstBlock(usize),
    /// `:name` — pop the top of stack and bind it.
    Bind(String),
    /// `'name` — push the binding and unbind it (SPEC §20).
    Move(String),
    /// `` `name `` — push a capability token (SPEC §12).
    ConstCap(String),
    /// Resolve a name: user binding first, else a builtin word.
    Load(String),
    /// A value operator: `+ - * / % _ == != < > <= >= && || ! = , \ & .`
    Op(String),
    /// `@` — pop a block and execute it.
    Invoke,
    /// `?` — select a block by a condition.
    Select,
    /// `;` — while: `[cond] [body] ;`.
    While,
}

/// A flat instruction sequence (a lowered block / quotation).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct IrBlock(pub Vec<Instr>);

/// A whole program: a block table and the entry block id.
#[derive(Debug, Clone, PartialEq)]
pub struct IrProgram {
    pub blocks: Vec<IrBlock>,
    pub main: usize,
}

struct Builder {
    blocks: Vec<IrBlock>,
}

impl Builder {
    fn lower_seq(&mut self, nodes: &[Node]) -> usize {
        let mut instrs = Vec::with_capacity(nodes.len());
        for n in nodes {
            match &n.kind {
                NodeKind::Int(v) => instrs.push(Instr::ConstInt(*v)),
                NodeKind::Float(v) => instrs.push(Instr::ConstFloat(*v)),
                NodeKind::Str(s) => instrs.push(Instr::ConstStr(s.clone())),
                NodeKind::Name(s) => instrs.push(Instr::Load(s.clone())),
                NodeKind::Bind(s) => instrs.push(Instr::Bind(s.clone())),
                NodeKind::Move(s) => instrs.push(Instr::Move(s.clone())),
                NodeKind::Cap(s) => instrs.push(Instr::ConstCap(s.clone())),
                NodeKind::Op(s) => instrs.push(match s.as_str() {
                    "@" => Instr::Invoke,
                    "?" => Instr::Select,
                    ";" => Instr::While,
                    _ => Instr::Op(s.clone()),
                }),
                NodeKind::Blk(inner) => {
                    let id = self.lower_seq(inner);
                    instrs.push(Instr::ConstBlock(id));
                }
            }
        }
        self.blocks.push(IrBlock(instrs));
        self.blocks.len() - 1
    }
}

/// Lower an AST program into IR.
pub fn lower(program: &[Node]) -> IrProgram {
    let mut b = Builder { blocks: Vec::new() };
    let main = b.lower_seq(program);
    IrProgram { blocks: b.blocks, main }
}

/// Rebuild the AST from IR (the exact inverse of `lower`). Used to prove lowering
/// is lossless against the oracle AST.
pub fn unlower(prog: &IrProgram) -> Vec<Node> {
    unlower_block(prog, prog.main)
}

fn unlower_block(prog: &IrProgram, id: usize) -> Vec<Node> {
    prog.blocks[id]
        .0
        .iter()
        .map(|instr| {
            let kind = match instr {
                Instr::ConstInt(v) => NodeKind::Int(*v),
                Instr::ConstFloat(v) => NodeKind::Float(*v),
                Instr::ConstStr(s) => NodeKind::Str(s.clone()),
                Instr::ConstBlock(bid) => NodeKind::Blk(unlower_block(prog, *bid)),
                Instr::Bind(s) => NodeKind::Bind(s.clone()),
                Instr::Move(s) => NodeKind::Move(s.clone()),
                Instr::ConstCap(s) => NodeKind::Cap(s.clone()),
                Instr::Load(s) => NodeKind::Name(s.clone()),
                Instr::Op(s) => NodeKind::Op(s.clone()),
                Instr::Invoke => NodeKind::Op("@".into()),
                Instr::Select => NodeKind::Op("?".into()),
                Instr::While => NodeKind::Op(";".into()),
            };
            Node::new(kind, Pos::default())
        })
        .collect()
}

/// Human-readable IR dump (`zownc ir`).
pub fn pretty(prog: &IrProgram) -> String {
    let mut out = format!("; zown ir  ({} blocks)\nmain = b{}\n\n", prog.blocks.len(), prog.main);
    for (i, blk) in prog.blocks.iter().enumerate() {
        out.push_str(&format!("b{i}:\n"));
        if blk.0.is_empty() {
            out.push_str("  ; (empty)\n");
        }
        for instr in &blk.0 {
            out.push_str("  ");
            out.push_str(&instr_str(instr));
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

fn instr_str(instr: &Instr) -> String {
    match instr {
        Instr::ConstInt(v) => format!("push.i   {v}"),
        Instr::ConstFloat(v) => format!("push.f   {v}"),
        Instr::ConstStr(s) => format!("push.s   {s:?}"),
        Instr::ConstBlock(id) => format!("push.blk b{id}"),
        Instr::Bind(s) => format!("bind     {s}"),
        Instr::Move(s) => format!("move     {s}"),
        Instr::ConstCap(s) => format!("push.cap {s}"),
        Instr::Load(s) => format!("load     {s}"),
        Instr::Op(s) => format!("op       {s}"),
        Instr::Invoke => "invoke".into(),
        Instr::Select => "select".into(),
        Instr::While => "while".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zown_ast::Node;

    fn ast(src: &str) -> Vec<Node> {
        zown_parser::parse(src).unwrap()
    }

    // Note: zown-ir does not depend on zown-parser at build time; tests pull it
    // in via dev-dependency. (Declared below.)

    #[test]
    fn roundtrip_hello() {
        let a = ast("[$Hello, World!$.]:h h@");
        let ir = lower(&a);
        assert_eq!(unlower(&ir), a);
    }

    #[test]
    fn roundtrip_nested_and_control() {
        for src in [
            "1 [$y$] [$n$] ? @ .",
            "0:c [ c 3 < ] [ c . c 1 + :c ] ;",
            "[ [ 1 2 + ] @ ] @ .",
            "10 20 * .",
        ] {
            let a = ast(src);
            let ir = lower(&a);
            assert_eq!(unlower(&ir), a, "roundtrip failed for {src:?}");
        }
    }

    #[test]
    fn roundtrip_cap_and_move() {
        let a = ast("`s [ 'x ] :f");
        let ir = lower(&a);
        assert_eq!(unlower(&ir), a);
    }

    #[test]
    fn block_table_separates_quotations() {
        let ir = lower(&ast("[ 1 ] [ 2 ]"));
        // two quotations + the main block
        assert_eq!(ir.blocks.len(), 3);
    }
}
