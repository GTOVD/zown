//! Zown lexer (Rust port of the Python reference lexer).
//!
//! A two-state machine: code mode reads self-delimiting symbol tokens; a `$`
//! flips into literal mode where bytes are swept verbatim until the next `$`.
//! This is the first real piece of the native toolchain (`zownc`); it is kept
//! deliberately faithful to `zown/lexer.py` so the two can be differentially
//! tested. See `docs/PLAN.md` milestone M3.

/// Source position. `offset` is a character index (matches the Python reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    /// `:name` — bind the top of stack to `name`.
    Bind(String),
    LBrack,
    RBrack,
    /// An operator symbol (e.g. `+`, `==`, `@`).
    Op(String),
    Eof,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub pos: Pos,
    /// Raw source span as (offset, length) in characters, for tooling/zero-copy.
    pub span: (usize, usize),
}

/// A structured lex error mirroring the `.zerr` packet (kind = "lex").
#[derive(Debug, Clone, PartialEq)]
pub struct LexError {
    pub code: &'static str,
    pub msg: String,
    pub pos: Pos,
    pub hint: &'static str,
}

pub const REPAIR_SYNTAX: &str = "REPAIR_SYNTAX";

const TWO_CHAR_OPS: [&str; 6] = ["==", "!=", "<=", ">=", "&&", "||"];
const ONE_CHAR_OPS: &str = "@.,=\\&+-*/%<>!?;_";

fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic()
}
fn is_ident_cont(c: char) -> bool {
    c.is_ascii_alphanumeric()
}

pub struct Lexer {
    src: Vec<char>,
    i: usize,
    line: u32,
    col: u32,
}

impl Lexer {
    pub fn new(src: &str) -> Self {
        Lexer { src: src.chars().collect(), i: 0, line: 1, col: 1 }
    }

    fn peek(&self, k: usize) -> Option<char> {
        self.src.get(self.i + k).copied()
    }

    fn advance(&mut self) -> char {
        let c = self.src[self.i];
        self.i += 1;
        if c == '\n' {
            self.line += 1;
            self.col = 1;
        } else {
            self.col += 1;
        }
        c
    }

    fn pos(&self) -> Pos {
        Pos { line: self.line, col: self.col, offset: self.i }
    }

    fn err(&self, msg: impl Into<String>, hint: &'static str) -> LexError {
        LexError { code: REPAIR_SYNTAX, msg: msg.into(), pos: self.pos(), hint }
    }

    pub fn tokens(&mut self) -> Result<Vec<Token>, LexError> {
        let mut out = Vec::new();
        while self.i < self.src.len() {
            let c = self.src[self.i];
            if c == ' ' || c == '\t' || c == '\r' || c == '\n' {
                self.advance();
                continue;
            }
            if c == '#' {
                while self.i < self.src.len() && self.src[self.i] != '\n' {
                    self.advance();
                }
                continue;
            }
            if c == '$' {
                out.push(self.read_string()?);
                continue;
            }
            if c == '[' {
                let (p, start) = (self.pos(), self.i);
                self.advance();
                out.push(Token { kind: TokenKind::LBrack, pos: p, span: (start, 1) });
                continue;
            }
            if c == ']' {
                let (p, start) = (self.pos(), self.i);
                self.advance();
                out.push(Token { kind: TokenKind::RBrack, pos: p, span: (start, 1) });
                continue;
            }
            if c == ':' {
                out.push(self.read_bind()?);
                continue;
            }
            if c.is_ascii_digit() {
                out.push(self.read_number());
                continue;
            }
            if is_ident_start(c) {
                out.push(self.read_ident());
                continue;
            }
            match self.read_op()? {
                Some(t) => out.push(t),
                None => {
                    return Err(self.err(
                        format!("unexpected character {:?}", c),
                        "character is not a Zown operator, literal, or identifier start",
                    ))
                }
            }
        }
        out.push(Token { kind: TokenKind::Eof, pos: self.pos(), span: (self.i, 0) });
        Ok(out)
    }

    fn read_string(&mut self) -> Result<Token, LexError> {
        let (p, start) = (self.pos(), self.i);
        self.advance(); // opening $
        let mut buf = String::new();
        loop {
            if self.i >= self.src.len() {
                return Err(self.err(
                    "unterminated string literal",
                    "add a closing `$`; strings are bounded by $...$",
                ));
            }
            let c = self.advance();
            if c == '\\' {
                let nxt = if self.i < self.src.len() { self.advance() } else { '\0' };
                buf.push(match nxt {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '0' => '\0',
                    '\\' => '\\',
                    '$' => '$',
                    other => other,
                });
                continue;
            }
            if c == '$' {
                break;
            }
            buf.push(c);
        }
        let len = self.i - start;
        Ok(Token { kind: TokenKind::Str(buf), pos: p, span: (start, len) })
    }

    fn read_bind(&mut self) -> Result<Token, LexError> {
        let (p, start) = (self.pos(), self.i);
        self.advance(); // the ':'
        match self.peek(0) {
            Some(c) if is_ident_start(c) => {}
            _ => {
                return Err(self.err(
                    "expected an identifier after `:`",
                    "binding form is `:name`, e.g. `5:x` binds 5 to x",
                ))
            }
        }
        let mut name = String::new();
        while let Some(c) = self.peek(0) {
            if is_ident_cont(c) {
                name.push(self.advance());
            } else {
                break;
            }
        }
        Ok(Token { kind: TokenKind::Bind(name), pos: p, span: (start, self.i - start) })
    }

    fn read_number(&mut self) -> Token {
        let (p, start) = (self.pos(), self.i);
        while matches!(self.peek(0), Some(c) if c.is_ascii_digit()) {
            self.advance();
        }
        let mut is_float = false;
        // `.` is the print op; only a decimal point if a digit follows.
        if self.peek(0) == Some('.') && matches!(self.peek(1), Some(c) if c.is_ascii_digit()) {
            is_float = true;
            self.advance();
            while matches!(self.peek(0), Some(c) if c.is_ascii_digit()) {
                self.advance();
            }
        }
        let text: String = self.src[start..self.i].iter().collect();
        let span = (start, self.i - start);
        let kind = if is_float {
            TokenKind::Float(text.parse().unwrap())
        } else {
            TokenKind::Int(text.parse().unwrap())
        };
        Token { kind, pos: p, span }
    }

    fn read_ident(&mut self) -> Token {
        let (p, start) = (self.pos(), self.i);
        while let Some(c) = self.peek(0) {
            if is_ident_cont(c) {
                self.advance();
            } else {
                break;
            }
        }
        let text: String = self.src[start..self.i].iter().collect();
        Token { kind: TokenKind::Ident(text), pos: p, span: (start, self.i - start) }
    }

    fn read_op(&mut self) -> Result<Option<Token>, LexError> {
        let (p, start) = (self.pos(), self.i);
        if self.i + 1 < self.src.len() {
            let two: String = self.src[self.i..self.i + 2].iter().collect();
            if TWO_CHAR_OPS.contains(&two.as_str()) {
                self.advance();
                self.advance();
                return Ok(Some(Token { kind: TokenKind::Op(two), pos: p, span: (start, 2) }));
            }
        }
        let c = self.src[self.i];
        if c == '|' {
            return Err(self.err(
                "lone `|` is reserved",
                "use `||` for logical or; `|` is reserved for the future atomic-pipe operator",
            ));
        }
        if ONE_CHAR_OPS.contains(c) {
            self.advance();
            return Ok(Some(Token {
                kind: TokenKind::Op(c.to_string()),
                pos: p,
                span: (start, 1),
            }));
        }
        Ok(None)
    }
}

/// Convenience: lex a whole source string.
pub fn lex(src: &str) -> Result<Vec<Token>, LexError> {
    Lexer::new(src).tokens()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(src: &str) -> Vec<TokenKind> {
        lex(src)
            .unwrap()
            .into_iter()
            .map(|t| t.kind)
            .filter(|k| !matches!(k, TokenKind::Eof))
            .collect()
    }

    #[test]
    fn dollar_string() {
        assert_eq!(kinds("$Hello, World!$"), vec![TokenKind::Str("Hello, World!".into())]);
    }

    #[test]
    fn number_vs_print_dot() {
        assert_eq!(
            kinds("5."),
            vec![TokenKind::Int(5), TokenKind::Op(".".into())]
        );
        assert_eq!(kinds("5.5"), vec![TokenKind::Float(5.5)]);
    }

    #[test]
    fn two_char_ops() {
        assert_eq!(
            kinds("== != <= >= && ||"),
            vec![
                TokenKind::Op("==".into()),
                TokenKind::Op("!=".into()),
                TokenKind::Op("<=".into()),
                TokenKind::Op(">=".into()),
                TokenKind::Op("&&".into()),
                TokenKind::Op("||".into()),
            ]
        );
    }

    #[test]
    fn bind() {
        assert_eq!(kinds("5:x"), vec![TokenKind::Int(5), TokenKind::Bind("x".into())]);
    }

    #[test]
    fn lone_pipe_reserved() {
        let e = lex("|").unwrap_err();
        assert_eq!(e.code, REPAIR_SYNTAX);
    }

    #[test]
    fn hello_world_shape() {
        // [$Hello, World!$.]:h h@
        assert_eq!(
            kinds("[$Hello, World!$.]:h h@"),
            vec![
                TokenKind::LBrack,
                TokenKind::Str("Hello, World!".into()),
                TokenKind::Op(".".into()),
                TokenKind::RBrack,
                TokenKind::Bind("h".into()),
                TokenKind::Ident("h".into()),
                TokenKind::Op("@".into()),
            ]
        );
    }
}
