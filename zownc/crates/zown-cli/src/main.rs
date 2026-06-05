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
    ir <file.zn>      Lower to Zown IR and print it
    irast <file.zn>   Lower to IR then rebuild the AST as JSON (round-trip check)
    wat <file.zn>     Compile to WebAssembly text (.wat) and print it
    build <file.zn>   Compile to WebAssembly (-o <path>; .wasm = binary, else .wat)
    version           Print version
    help              Show this help

NOTE:
    The WASM backend (M6) compiles the full v0.1 language: ints, floats,
    strings, blocks, control flow, bindings, and the stdlib words. `build`
    emits binary `.wasm` when -o ends in .wasm, otherwise `.wat`. Both run
    under `wasmtime` (WASI). Native desktop binaries come in M7.

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

    // extract `-o <path>` (used by `build`)
    let mut out_path: Option<String> = None;
    if let Some(i) = args.iter().position(|a| a == "-o") {
        if i + 1 < args.len() {
            out_path = Some(args[i + 1].clone());
            args.drain(i..=i + 1);
        } else {
            eprintln!("zownc: -o requires a path");
            return ExitCode::FAILURE;
        }
    }

    // shorthand: `zownc file.zn` == `zownc run file.zn`
    let known = ["run", "lex", "ast", "ir", "irast", "wat", "build", "version", "help"];
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
        "ir" => match args.get(1) {
            Some(path) => cmd_ir(path, false),
            None => {
                eprintln!("zownc ir: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        "irast" => match args.get(1) {
            Some(path) => cmd_ir(path, true),
            None => {
                eprintln!("zownc irast: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        "wat" => match args.get(1) {
            Some(path) => cmd_wat(path, None),
            None => {
                eprintln!("zownc wat: missing <file.zn>\n\n{USAGE}");
                ExitCode::FAILURE
            }
        },
        "build" => match args.get(1) {
            Some(path) => {
                let out = out_path.unwrap_or_else(|| default_out(path));
                cmd_wat(path, Some(&out))
            }
            None => {
                eprintln!("zownc build: missing <file.zn>\n\n{USAGE}");
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

fn cmd_ir(path: &str, roundtrip: bool) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zownc: cannot read {path:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    match parse(&src) {
        Ok(nodes) => {
            let prog = zown_ir::lower(&nodes);
            if roundtrip {
                // Lower -> unlower -> AST JSON; should be byte-identical to `ast`.
                println!("{}", zown_ast::to_json(&zown_ir::unlower(&prog)));
            } else {
                print!("{}", zown_ir::pretty(&prog));
            }
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

fn default_out(path: &str) -> String {
    match path.strip_suffix(".zn") {
        Some(stem) => format!("{stem}.wat"),
        None => format!("{path}.wat"),
    }
}

fn cmd_wat(path: &str, out: Option<&str>) -> ExitCode {
    let src = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("zownc: cannot read {path:?}: {e}");
            return ExitCode::FAILURE;
        }
    };
    let nodes = match parse(&src) {
        Ok(n) => n,
        Err(e) => {
            eprintln!(
                "zerr[{}] {path}:{}:{} ({}): {}",
                e.code, e.pos.line, e.pos.col, e.kind, e.msg
            );
            return ExitCode::FAILURE;
        }
    };
    let prog = zown_ir::lower(&nodes);

    // Binary `.wasm` when the output path asks for it; `.wat` text otherwise.
    if let Some(path) = out {
        if path.ends_with(".wasm") {
            return match zown_wasm::emit_wasm(&prog) {
                Ok(bytes) => match std::fs::write(path, &bytes) {
                    Ok(()) => {
                        eprintln!("wrote {path} ({} bytes)", bytes.len());
                        ExitCode::SUCCESS
                    }
                    Err(e) => {
                        eprintln!("zownc: cannot write {path:?}: {e}");
                        ExitCode::FAILURE
                    }
                },
                Err(msg) => {
                    eprintln!("zownc build: {msg}");
                    ExitCode::FAILURE
                }
            };
        }
    }

    match zown_wasm::emit_wat(&prog) {
        Ok(wat) => match out {
            Some(path) => match std::fs::write(path, &wat) {
                Ok(()) => {
                    eprintln!("wrote {path}");
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("zownc: cannot write {path:?}: {e}");
                    ExitCode::FAILURE
                }
            },
            None => {
                print!("{wat}");
                ExitCode::SUCCESS
            }
        },
        Err(msg) => {
            eprintln!("zownc build: {msg}");
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
