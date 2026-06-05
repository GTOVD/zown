//! `zownc` — the native Zown toolchain driver (stage-0, in progress).
//!
//! Today this implements the first parity targets from `docs/PLAN.md` (M2/M3):
//! `zownc lex <file>` tokenizes and `zownc ast <file>` parses a source file with
//! the Rust frontend. The VM, IR, and WASM/native backends land in later
//! milestones.

use std::process::ExitCode;

use zown_lexer::{lex, Token, TokenKind};
use zown_parser::parse;
use zown_vm::Vm;

const USAGE: &str = "\
zownc — Zown native toolchain (stage-0)

USAGE:
    zownc <COMMAND> [ARGS]

COMMANDS:
    run <file.zn>     Run a program (shorthand: `zownc <file.zn>`)
    lex <file.zn>     Tokenize a source file and print the token stream
    ast <file.zn>     Parse a source file and print the AST as JSON
    version           Print version
    help              Show this help

FLAGS:
    --zerr            On error, emit a structured JSON .zerr packet to stderr

STATUS:
    The native compiler is under construction. See docs/PLAN.md for the
    milestone plan (M2 toolchain skeleton -> M14 self-hosting). The Python
    reference interpreter (`zown`) remains the behavioral oracle.";

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let zerr = args.iter().any(|a| a == "--zerr");
    args.retain(|a| a != "--zerr");

    // shorthand: `zownc file.zn` == `zownc run file.zn`
    let known = ["run", "lex", "ast", "version", "help"];
    if let Some(first) = args.first() {
        if !first.starts_with('-') && !known.contains(&first.as_str()) {
            args.insert(0, "run".to_string());
        }
    }

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
        "run" => match args.get(1) {
            Some(path) => cmd_run(path, zerr),
            None => {
                eprintln!("zownc run: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        "lex" => match args.get(1) {
            Some(path) => cmd_lex(path),
            None => {
                eprintln!("zownc lex: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        "ast" => match args.get(1) {
            Some(path) => cmd_ast(path),
            None => {
                eprintln!("zownc ast: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        other => {
            eprintln!("zownc: unknown command {other:?}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn cmd_run(path: &str, zerr: bool) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zownc: cannot read {path:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mut vm = Vm::new(Some(path.to_string()));
    let result = vm.run_str(&src);
    // Print whatever the program emitted before any fault (print-as-you-go parity).
    print!("{}", vm.out);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(d) => {
            if zerr {
                eprintln!("{}", d.to_json(Some(path)));
            } else {
                eprintln!("{}", d.render_human(Some(path)));
            }
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

fn cmd_ast(path: &str) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zownc: cannot read {path:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match parse(&src) {
        Ok(nodes) => {
            println!("{}", zown_ast::to_json(&nodes));
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!(
                "zerr[{}] {path}:{}:{} ({}): {}",
                e.code, e.pos.line, e.pos.col, e.kind, e.msg
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
