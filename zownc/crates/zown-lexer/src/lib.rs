//! Zown lexer (Rust port of the Python reference lexer).
//!
//! A two-state machine: code mode reads self-delimiting symbol tokens; a `$`
//! flips into literal mode where bytes are swept verbatim until the next `$`.
//! This is the first real piece of the native toolchain (`zownc`); it is kept
//! deliberately faithful to `zown/lexer.py` so the two can be differentially
//! tested. See `docs/PLAN.md` milestone M3.

/// Source position. `offset` is a character index (matches the Python reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Pos {
    pub line: u32,
    pub col: u32,
    pub offset: usize,
}

/// Checked 64-bit `int` range (SPEC §1/§11, decision D1). Literals outside it
/// are a lex-time `OVERFLOW`; arbitrary precision is the explicit `big` (`b`).
pub const INT_MIN: i64 = i64::MIN;
pub const INT_MAX: i64 = i64::MAX;

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i64),
    Float(f64),
    Str(String),
    Ident(String),
    /// `:name` — bind the top of stack to `name`.
    Bind(String),
    /// `'name` — move the binding out (push + unbind). SPEC §20.
    Move(String),
    /// `` `name `` — a capability token. SPEC §12.
    Cap(String),
    LBrack,
    RBrack,
    /// An operator symbol (e.g. `+`, `==`, `@`, `??`).
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
pub const OVERFLOW: &str = "OVERFLOW";

const TWO_CHAR_OPS: [&str; 7] = ["==", "!=", "<=", ">=", "&&", "||", "??"];
// `|` is the lane pipe. `||` is matched first, so a lone pipe is not `or`.
const ONE_CHAR_OPS: &str = "@.,=\\&+-*/%<>!?;_|^";

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
        Lexer {
            src: src.chars().collect(),
            i: 0,
            line: 1,
            col: 1,
        }
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
        Pos {
            line: self.line,
            col: self.col,
            offset: self.i,
        }
    }

    fn err(&self, msg: impl Into<String>, hint: &'static str) -> LexError {
        self.err_code(REPAIR_SYNTAX, msg, hint)
    }

    fn err_code(&self, code: &'static str, msg: impl Into<String>, hint: &'static str) -> LexError {
        LexError {
            code,
            msg: msg.into(),
            pos: self.pos(),
            hint,
        }
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
                out.push(Token {
                    kind: TokenKind::LBrack,
                    pos: p,
                    span: (start, 1),
                });
                continue;
            }
            if c == ']' {
                let (p, start) = (self.pos(), self.i);
                self.advance();
                out.push(Token {
                    kind: TokenKind::RBrack,
                    pos: p,
                    span: (start, 1),
                });
                continue;
            }
            if c == ':' {
                out.push(self.read_bind()?);
                continue;
            }
            if c == '\'' {
                out.push(self.read_move()?);
                continue;
            }
            if c == '`' {
                out.push(self.read_cap()?);
                continue;
            }
            if c.is_ascii_digit() {
                out.push(self.read_number()?);
                continue;
            }
            if is_ident_start(c) {
                out.push(self.read_ident());
                continue;
            }
            if c == '~' {
                out.push(self.read_lane()?);
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
        out.push(Token {
            kind: TokenKind::Eof,
            pos: self.pos(),
            span: (self.i, 0),
        });
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
                let nxt = if self.i < self.src.len() {
                    self.advance()
                } else {
                    '\0'
                };
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
        Ok(Token {
            kind: TokenKind::Str(buf),
            pos: p,
            span: (start, len),
        })
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
        Ok(Token {
            kind: TokenKind::Bind(name),
            pos: p,
            span: (start, self.i - start),
        })
    }

    fn read_named(&mut self, kind_tag: &str) -> Result<(String, Pos, (usize, usize)), LexError> {
        let (p, start) = (self.pos(), self.i);
        self.advance(); // the sigil
        match self.peek(0) {
            Some(c) if is_ident_start(c) => {}
            _ => return Err(self.err(
                format!("expected an identifier after `{kind_tag}`"),
                match kind_tag {
                    "'" => {
                        "move form is 'name: push x's value and unbind it (a bare name is a borrow)"
                    }
                    "`" => "capability form is `name, e.g. `s (net-send); see SPEC Part II §12",
                    _ => "expected an identifier after the sigil",
                },
            )),
        }
        let mut name = String::new();
        while let Some(c) = self.peek(0) {
            if is_ident_cont(c) {
                name.push(self.advance());
            } else {
                break;
            }
        }
        Ok((name, p, (start, self.i - start)))
    }

    fn read_move(&mut self) -> Result<Token, LexError> {
        let (name, pos, span) = self.read_named("'")?;
        Ok(Token {
            kind: TokenKind::Move(name),
            pos,
            span,
        })
    }

    fn read_cap(&mut self) -> Result<Token, LexError> {
        let (name, pos, span) = self.read_named("`")?;
        Ok(Token {
            kind: TokenKind::Cap(name),
            pos,
            span,
        })
    }

    fn read_number(&mut self) -> Result<Token, LexError> {
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
        if is_float {
            // IEEE-754: may round, or be inf — never a lex error (SPEC §11).
            let v: f64 = text.parse().unwrap();
            return Ok(Token {
                kind: TokenKind::Float(v),
                pos: p,
                span,
            });
        }
        match text.parse::<i64>() {
            Ok(n) => Ok(Token { kind: TokenKind::Int(n), pos: p, span }),
            Err(_) => Err(self.err_code(
                OVERFLOW,
                format!("integer literal {text} does not fit int (max {INT_MAX})"),
                "int is a checked 64-bit integer; for arbitrary precision write it as a string and convert: $digits$ b",
            )),
        }
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
        Token {
            kind: TokenKind::Ident(text),
            pos: p,
            span: (start, self.i - start),
        }
    }

    /// `~name` is one operator. `~m` and `~n` are lanes; other names stay open.
    fn read_lane(&mut self) -> Result<Token, LexError> {
        let (p, start) = (self.pos(), self.i);
        self.advance(); // '~'
        match self.peek(0) {
            Some(c) if is_ident_start(c) => {}
            _ => {
                return Err(self.err(
                    "expected a lane name after `~`",
                    "`~m` registers the input lane and `~n` the net lane",
                ))
            }
        }
        while let Some(c) = self.peek(0) {
            if is_ident_cont(c) {
                self.advance();
            } else {
                break;
            }
        }
        let text: String = self.src[start..self.i].iter().collect();
        Ok(Token {
            kind: TokenKind::Op(text),
            pos: p,
            span: (start, self.i - start),
        })
    }

    fn read_op(&mut self) -> Result<Option<Token>, LexError> {
        let (p, start) = (self.pos(), self.i);
        if self.i + 1 < self.src.len() {
            let two: String = self.src[self.i..self.i + 2].iter().collect();
            if TWO_CHAR_OPS.contains(&two.as_str()) {
                self.advance();
                self.advance();
                return Ok(Some(Token {
                    kind: TokenKind::Op(two),
                    pos: p,
                    span: (start, 2),
                }));
            }
        }
        let c = self.src[self.i];
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
        assert_eq!(
            kinds("$Hello, World!$"),
            vec![TokenKind::Str("Hello, World!".into())]
        );
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
        assert_eq!(
            kinds("5:x"),
            vec![TokenKind::Int(5), TokenKind::Bind("x".into())]
        );
    }

    #[test]
    fn lane_ops() {
        assert_eq!(
            kinds("~m ~n | ^ ||"),
            vec![
                TokenKind::Op("~m".into()),
                TokenKind::Op("~n".into()),
                TokenKind::Op("|".into()),
                TokenKind::Op("^".into()),
                TokenKind::Op("||".into()),
            ]
        );
    }

    #[test]
    fn tilde_needs_a_name() {
        assert_eq!(lex("~").unwrap_err().code, REPAIR_SYNTAX);
        assert_eq!(lex("~1").unwrap_err().code, REPAIR_SYNTAX);
    }

    #[test]
    fn capability_and_move() {
        assert_eq!(
            kinds("`s 'x"),
            vec![TokenKind::Cap("s".into()), TokenKind::Move("x".into())]
        );
    }

    #[test]
    fn match_op_is_two_char() {
        assert_eq!(
            kinds("a ?? b ?"),
            vec![
                TokenKind::Ident("a".into()),
                TokenKind::Op("??".into()),
                TokenKind::Ident("b".into()),
                TokenKind::Op("?".into())
            ]
        );
    }

    #[test]
    fn int_literal_overflow() {
        let e = lex("9223372036854775808").unwrap_err(); // i64::MAX + 1
        assert_eq!(e.code, OVERFLOW);
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
