//! Zown parser (Rust port of `zown/parser.py`).
//!
//! Builds a nested `Node` program from the lexer's token stream. Parse errors
//! mirror the reference: unclosed `[` and stray `]` both report `REPAIR_SYNTAX`
//! with a position, so an agent can self-heal off the structured code.

use zown_ast::Node;
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
        ParseError { code: e.code, msg: e.msg, kind: "lex", pos: e.pos, hint: e.hint }
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
        Ok(match t.kind {
            TokenKind::Int(v) => Node::Int(v),
            TokenKind::Float(v) => Node::Float(v),
            TokenKind::Str(s) => Node::Str(s),
            TokenKind::Ident(s) => Node::Name(s),
            TokenKind::Bind(s) => Node::Bind(s),
            TokenKind::Op(s) => Node::Op(s),
            TokenKind::LBrack => {
                let inner = self.sequence(false)?;
                Node::Blk(inner)
            }
            // RBrack / Eof are handled by `sequence`; reaching here is a bug.
            other => {
                return Err(ParseError {
                    code: REPAIR_SYNTAX,
                    msg: format!("unexpected token {other:?}"),
                    kind: "parse",
                    pos: t.pos,
                    hint: "",
                })
            }
        })
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
    use zown_ast::Node;

    #[test]
    fn hello_world_ast() {
        let ast = parse("[$Hello, World!$.]:h h@").unwrap();
        assert_eq!(
            ast,
            vec![
                Node::Blk(vec![Node::Str("Hello, World!".into()), Node::Op(".".into())]),
                Node::Bind("h".into()),
                Node::Name("h".into()),
                Node::Op("@".into()),
            ]
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
    fn reserved_pipe_is_lex_error() {
        let e = parse("|").unwrap_err();
        assert_eq!(e.kind, "lex");
    }

    #[test]
    fn nested_blocks() {
        let ast = parse("[ [ 1 ] ]").unwrap();
        assert_eq!(ast, vec![Node::Blk(vec![Node::Blk(vec![Node::Int(1)])])]);
    }
}
