"""The `zown` command-line driver.

Usage:
    zown <file.zn>            run a program (shorthand for `run`)
    zown run <file.zn>        run a program
    zown check <file.zn>      parse + lint only; emit a .zerr packet on failure
    zown ast <file.zn>        print the parsed AST (debug)
    zown manifest <file.zn>   generate/update the shadow manifest (<file>.json)
    zown repl                 interactive stack session

Errors are printed in a human form by default, or as a JSON .zerr packet with
--zerr (and optionally written to a file with --zerr-out PATH) so an AI agent can
consume them directly.
"""

from __future__ import annotations

import argparse
import json
import sys

from .errors import ZownError
from .parser import parse
from .vm import VM


def _read(path: str) -> str:
    with open(path, "r", encoding="utf-8") as fh:
        return fh.read()


def _emit_error(e: ZownError, as_json: bool, zerr_out: str | None) -> None:
    if zerr_out:
        with open(zerr_out, "w", encoding="utf-8") as fh:
            fh.write(e.to_json())
            fh.write("\n")
    if as_json:
        sys.stderr.write(e.to_json() + "\n")
    else:
        sys.stderr.write(e.render_human() + "\n")


def cmd_run(args) -> int:
    src = _read(args.file)
    vm = VM(file=args.file)
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return 1
    return 0


def cmd_check(args) -> int:
    src = _read(args.file)
    try:
        parse(src, args.file)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return 1
    sys.stderr.write(f"ok: {args.file} parsed cleanly\n")
    return 0


def cmd_ast(args) -> int:
    src = _read(args.file)
    try:
        nodes = parse(src, args.file)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return 1
    print(json.dumps(_ast_json(nodes), indent=2))
    return 0


def _ast_json(nodes):
    out = []
    for node in nodes:
        tag = node[0]
        if tag == "blk":
            out.append({"blk": _ast_json(node[1])})
        else:
            out.append({tag: node[1]})
    return out


def cmd_manifest(args) -> int:
    from .manifest import write
    try:
        path = write(args.file)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return 1
    sys.stderr.write(f"wrote {path}\n")
    return 0


def cmd_repl(args) -> int:
    vm = VM(file="<repl>")
    sys.stderr.write("Zown repl. Type code; the stack prints after each line. Ctrl-D to exit.\n")
    while True:
        try:
            line = input("z> ")
        except EOFError:
            sys.stderr.write("\n")
            return 0
        if not line.strip():
            continue
        try:
            vm.run_src(line)
        except ZownError as e:
            sys.stderr.write(e.render_human() + "\n")
            continue
        from .vm import _as_str
        snap = " ".join(_as_str(v) for v in vm.stack)
        sys.stderr.write(f"   [{snap}]\n")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="zown", description="Zown language toolchain (reference build)")
    p.add_argument("--zerr", action="store_true", help="emit errors as JSON .zerr packets")
    p.add_argument("--zerr-out", metavar="PATH", help="also write the .zerr packet to PATH")
    sub = p.add_subparsers(dest="cmd")

    pr = sub.add_parser("run", help="run a program")
    pr.add_argument("file")
    pr.set_defaults(func=cmd_run)

    pc = sub.add_parser("check", help="parse/lint only")
    pc.add_argument("file")
    pc.set_defaults(func=cmd_check)

    pa = sub.add_parser("ast", help="print the parsed AST")
    pa.add_argument("file")
    pa.set_defaults(func=cmd_ast)

    pm = sub.add_parser("manifest", help="generate/update the shadow manifest")
    pm.add_argument("file")
    pm.set_defaults(func=cmd_manifest)

    prp = sub.add_parser("repl", help="interactive session")
    prp.set_defaults(func=cmd_repl)

    return p


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = build_parser()

    # shorthand: `zown file.zn` == `zown run file.zn`
    known = {"run", "check", "ast", "manifest", "repl"}
    positional = [a for a in argv if not a.startswith("-")]
    if positional and positional[0] not in known:
        idx = argv.index(positional[0])
        argv = argv[:idx] + ["run"] + argv[idx:]

    args = parser.parse_args(argv)
    if not getattr(args, "func", None):
        parser.print_help()
        return 0
    return args.func(args)


if __name__ == "__main__":
    raise SystemExit(main())
