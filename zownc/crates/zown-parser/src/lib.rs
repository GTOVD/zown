//! Zown parser (Rust port of `zown/parser.py`).
//!
//! Builds a nested `Node` program from the lexer's token stream. Parse errors
//! mirror the reference: unclosed `[` and stray `]` both report `REPAIR_SYNTAX`
//! with a position, so an agent can self-heal off the structured code.

use zown_ast::{Node, NodeKind};
use zown_lexer::{lex, LexError, Pos, Token, TokenKind};

pub const REPAIR_SYNTAX: &str = "REPAIR_SYNTAX";

/// A parse/lex diagnostic (kind distinguishes the phase).
#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub code: &'static str,
    pub msg: String,
    pub kind: &'static str, // "lex" | "parse"
    pub pos: Pos,
    pub hint: &'static str,
}

impl From<LexError> for ParseError {
    fn from(e: LexError) -> Self {
        ParseError {
            code: e.code,
            msg: e.msg,
            kind: "lex",
            pos: e.pos,
            hint: e.hint,
        }
    }
}

struct Parser {
    toks: Vec<Token>,
    i: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.toks[self.i]
    }

    fn advance(&mut self) -> Token {
        let t = self.toks[self.i].clone();
        self.i += 1;
        t
    }

    fn parse(&mut self) -> Result<Vec<Node>, ParseError> {
        self.sequence(true)
    }

    fn sequence(&mut self, top: bool) -> Result<Vec<Node>, ParseError> {
        let mut nodes = Vec::new();
        loop {
            match &self.peek().kind {
                TokenKind::Eof => {
                    if !top {
                        return Err(ParseError {
                            code: REPAIR_SYNTAX,
                            msg: "unclosed block: missing `]`".into(),
                            kind: "parse",
                            pos: self.peek().pos,
                            hint: "every `[` needs a matching `]`",
                        });
                    }
                    return Ok(nodes);
                }
                TokenKind::RBrack => {
                    if top {
                        return Err(ParseError {
                            code: REPAIR_SYNTAX,
                            msg: "unexpected `]` with no matching `[`".into(),
                            kind: "parse",
                            pos: self.peek().pos,
                            hint: "remove the stray `]` or add a matching `[`",
                        });
                    }
                    self.advance();
                    return Ok(nodes);
                }
                _ => nodes.push(self.node()?),
            }
        }
    }

    fn node(&mut self) -> Result<Node, ParseError> {
        let t = self.advance();
        let pos = t.pos;
        let kind = match t.kind {
            TokenKind::Int(v) => NodeKind::Int(v),
            TokenKind::Float(v) => NodeKind::Float(v),
            TokenKind::Str(s) => NodeKind::Str(s),
            TokenKind::Ident(s) => NodeKind::Name(s),
            TokenKind::Bind(s) => NodeKind::Bind(s),
            TokenKind::Move(s) => NodeKind::Move(s),
            TokenKind::Cap(s) => NodeKind::Cap(s),
            TokenKind::Op(s) => NodeKind::Op(s),
            TokenKind::LBrack => {
                let inner = self.sequence(false)?;
                NodeKind::Blk(inner)
            }
            other => {
                return Err(ParseError {
                    code: REPAIR_SYNTAX,
                    msg: format!("unexpected token {other:?}"),
                    kind: "parse",
                    pos,
                    hint: "",
                })
            }
        };
        Ok(Node::new(kind, pos))
    }
}

/// Parse a token stream into a program.
pub fn parse_tokens(toks: Vec<Token>) -> Result<Vec<Node>, ParseError> {
    Parser { toks, i: 0 }.parse()
}

/// Lex + parse a source string into a program.
pub fn parse(src: &str) -> Result<Vec<Node>, ParseError> {
    let toks = lex(src)?;
    parse_tokens(toks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zown_ast::NodeKind;

    fn kinds(src: &str) -> Vec<NodeKind> {
        parse(src).unwrap().into_iter().map(|n| n.kind).collect()
    }

    #[test]
    fn hello_world_ast() {
        assert_eq!(
            kinds("[$Hello, World!$.]:h h@"),
            vec![
                NodeKind::Blk(vec![
                    Node::new(NodeKind::Str("Hello, World!".into()), Pos::default()),
                    Node::new(NodeKind::Op(".".into()), Pos::default()),
                ]),
                NodeKind::Bind("h".into()),
                NodeKind::Name("h".into()),
                NodeKind::Op("@".into()),
            ]
        );
    }

    #[test]
    fn cap_and_move_nodes() {
        assert_eq!(
            kinds("`s 'x"),
            vec![NodeKind::Cap("s".into()), NodeKind::Move("x".into())]
        );
    }

    #[test]
    fn unclosed_block_errors() {
        let e = parse("[ 1 2").unwrap_err();
        assert_eq!(e.code, REPAIR_SYNTAX);
        assert_eq!(e.kind, "parse");
    }

    #[test]
    fn stray_rbrack_errors() {
        let e = parse("1 ]").unwrap_err();
        assert_eq!(e.code, REPAIR_SYNTAX);
    }

    #[test]
    fn pipe_is_an_operator() {
        assert_eq!(kinds("|"), vec![NodeKind::Op("|".into())]);
    }

    #[test]
    fn lone_paren_is_lex_error() {
        let e = parse("(").unwrap_err();
        assert_eq!(e.kind, "lex");
        assert_eq!(e.code, REPAIR_SYNTAX);
    }

    #[test]
    fn nested_blocks() {
        assert_eq!(
            kinds("[ [ 1 ] ]"),
            vec![NodeKind::Blk(vec![Node::new(
                NodeKind::Blk(vec![Node::new(NodeKind::Int(1), Pos::default())]),
                Pos::default(),
            )])]
        );
    }
}
