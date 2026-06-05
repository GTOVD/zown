//! `zownc` — the native Zown toolchain driver (stage-0, in progress).
//!
//! Today this implements the first parity target from `docs/PLAN.md` (M2/M3):
//! `zownc lex <file>` tokenizes a source file using the Rust lexer. The parser,
//! VM, IR, and WASM/native backends land in subsequent milestones.

use std::process::ExitCode;

use zown_lexer::{lex, Token, TokenKind};

const USAGE: &str = "\
zownc — Zown native toolchain (stage-0)

USAGE:
    zownc <COMMAND> [ARGS]

COMMANDS:
    lex <file.zn>     Tokenize a source file and print the token stream
    version           Print version
    help              Show this help

STATUS:
    The native compiler is under construction. See docs/PLAN.md for the
    milestone plan (M2 toolchain skeleton -> M14 self-hosting). The Python
    reference interpreter (`zown`) remains the behavioral oracle.";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    match cmd {
        "help" | "-h" | "--help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        "version" | "-V" | "--version" => {
            println!("zownc {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        "lex" => match args.get(1) {
            Some(path) => cmd_lex(path),
            None => {
                eprintln!("zownc lex: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("zownc: unknown command {other:?}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_lex(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zownc: cannot read {path:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match lex(&src) {
        Ok(tokens) => {
            for t in &tokens {
                println!("{}", render(t));
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            // Human form; --zerr JSON parity comes with the shared diagnostics crate.
            eprintln!(
                "zerr[{}] {path}:{}:{} : {}\n  hint: {}",
                e.code, e.pos.line, e.pos.col, e.msg, e.hint
            );
            ExitCode::FAILURE
        }
    }
}

fn render(t: &Token) -> String {
    let (l, c) = (t.pos.line, t.pos.col);
    let body = match &t.kind {
        TokenKind::Int(v) => format!("INT      {v}"),
        TokenKind::Float(v) => format!("FLOAT    {v}"),
        TokenKind::Str(v) => format!("STR      {v:?}"),
        TokenKind::Ident(v) => format!("IDENT    {v}"),
        TokenKind::Bind(v) => format!("BIND     :{v}"),
        TokenKind::LBrack => "LBRACK   [".to_string(),
        TokenKind::RBrack => "RBRACK   ]".to_string(),
        TokenKind::Op(v) => format!("OP       {v}"),
        TokenKind::Eof => "EOF".to_string(),
    };
    format!("{l:>3}:{c:<3} {body}")
}
