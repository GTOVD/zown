"""The `zown` command-line driver.

Usage:
    zown <file.zn>            run a program (shorthand for `run`)
    zown run <file.zn>        check, then run a program
    zown check <file.zn>      static checker (M8): names, match shape, capability flow
    zown ast <file.zn>        print the parsed AST (debug)
    zown manifest <file.zn>   generate/update the shadow manifest (<file>.json)
    zown repl                 interactive stack session
    zown tune [N]             how many real-time lanes this machine pins
    zown fmt [-w] <file.zn>   canonical whitespace; -w writes the file
    zown test [path]          run .zn programs (default tests/zn)
    zown mesh [port]          two IPv6 peers exchange a content-addressed bundle
    zown draw <file.zn>       paint the node left on the stack as a PPM
    zown keep export <file> -o <out.sync>
                              run a program and fsync its graph as a sealed bundle
    zown keep verify <out.sync>
                              open a bundle; a changed byte is INTEGRITY_FAIL
    zown watch <file.zn>      run a program, then print its telemetry log
    zown stage <file.zn>      token-dump a file with the Zown lexer; it must match the host
    zown stage parse <file.zn>
                              parse a file with the Zown parser; it must match the host AST

`run` is **gated by the static checker** (PLAN M8c-0, decision D2): a program
with any static finding is rejected before a single node executes. The findings
are reported exactly as `check` reports them, and the process exits with status
2 so a pipeline can tell "rejected by the checker — rework the code" apart from
a lex/parse/runtime `.zerr` (status 1) and success (0). `--unchecked` skips the
gate; it exists to observe the runtime behavior of a rejected program (e.g. to
compare a static diagnosis with the runtime packet) and is not for production.

Errors are printed in a human form by default, or as JSON .zerr packets with
--zerr (and optionally written to a file with --zerr-out PATH) so an AI agent can
consume them directly. A static rejection is a JSON *array* of packets.

Exit codes:  0 ok · 1 lex/parse/runtime error · 2 rejected by the static checker
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


EXIT_OK, EXIT_ERROR, EXIT_REJECTED = 0, 1, 2


def _emit_diags(diags: list[ZownError], args, tail: str) -> None:
    """Report static-checker findings: a JSON array under --zerr (and to
    --zerr-out), else the human form of each followed by `tail`."""
    for d in diags:
        d.file = d.file or args.file
    if args.zerr_out:
        with open(args.zerr_out, "w", encoding="utf-8") as fh:
            fh.write(json.dumps([d.packet() for d in diags], indent=2))
            fh.write("\n")
    if args.zerr:
        sys.stderr.write(json.dumps([d.packet() for d in diags]) + "\n")
    else:
        for d in diags:
            sys.stderr.write(d.render_human() + "\n")
        sys.stderr.write(tail + "\n")


def _static_check(src: str, args) -> tuple[list[ZownError], int | None]:
    """Run the checker. Returns (diags, None) or ([], exit_code) when a lex/parse
    error stopped the check (already reported)."""
    from .checker import check_src
    try:
        return check_src(src, args.file), None
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return [], EXIT_ERROR


def cmd_run(args) -> int:
    src = _read(args.file)
    if not getattr(args, "unchecked", False):
        diags, code = _static_check(src, args)
        if code is not None:
            return code
        if diags:
            _emit_diags(diags, args,
                        f"rejected: {args.file} — {len(diags)} issue(s) found by the "
                        f"static checker; not run (--unchecked overrides)")
            return EXIT_REJECTED
    vm = VM(file=args.file)
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return EXIT_ERROR
    return EXIT_OK


def cmd_check(args) -> int:
    src = _read(args.file)
    diags, code = _static_check(src, args)
    if code is not None:
        return code
    if not diags:
        sys.stderr.write(f"ok: {args.file} — no issues\n")
        return EXIT_OK
    _emit_diags(diags, args, f"{len(diags)} issue(s) in {args.file}")
    return EXIT_REJECTED


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


def cmd_tune(args) -> int:
    from .lanes import tune_report
    if args.n is not None and args.n < 0:
        sys.stderr.write("tune: requested real-time slots must be >= 0\n")
        return 1
    sys.stdout.write(tune_report(args.n))
    return 0


def cmd_fmt(args) -> int:
    from .fmt import format_source
    failed = False
    for path in args.files:
        src = _read(path)
        try:
            text = format_source(src)
        except ZownError as e:
            e.file = path
            _emit_error(e, args.zerr, args.zerr_out)
            failed = True
            continue
        if args.write:
            if text != src:
                with open(path, "w", encoding="utf-8") as fh:
                    fh.write(text)
        else:
            sys.stdout.write(text)
    return EXIT_ERROR if failed else EXIT_OK


def cmd_mesh(args) -> int:
    from .mesh import demo
    try:
        sys.stdout.write(demo(args.port))
    except RuntimeError as exc:
        sys.stderr.write(f"mesh: {exc}\n")
        return EXIT_ERROR
    return EXIT_OK


def cmd_draw(args) -> int:
    from .ui import UiNode, raster
    src = _read(args.file)
    if not getattr(args, "unchecked", False):
        diags, code = _static_check(src, args)
        if code is not None:
            return code
        if diags:
            _emit_diags(diags, args,
                        f"rejected: {args.file} — {len(diags)} issue(s) found by the "
                        f"static checker; not drawn (--unchecked overrides)")
            return EXIT_REJECTED
    vm = VM(file=args.file)
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return EXIT_ERROR
    if len(vm.stack) != 1 or not isinstance(vm.stack[0], UiNode):
        sys.stderr.write("draw: the program must leave one layout node on the stack\n")
        return EXIT_ERROR
    node = vm.stack[0]
    if node.w < 1 or node.h < 1:
        sys.stderr.write("draw: the root has no area\n")
        return EXIT_ERROR
    sys.stdout.buffer.write(raster(node))
    return EXIT_OK


def cmd_test(args) -> int:
    from .tester import run_tests
    return run_tests(args.path)


def cmd_stage(args) -> int:
    from .stage import StageMismatch, stage_file, stage_parse
    parsing = args.mode_or_file == "parse"
    path = args.file if parsing else args.mode_or_file
    if parsing and not path:
        sys.stderr.write("usage: zown stage parse <file.zn>\n")
        return 2
    try:
        sys.stdout.write(stage_parse(path) if parsing else stage_file(path))
    except ZownError as e:
        e.file = e.file or path
        _emit_error(e, args.zerr, args.zerr_out)
        return EXIT_ERROR
    except StageMismatch as e:
        sys.stderr.write(f"stage: {path}\n{e.msg}\n")
        return EXIT_ERROR
    return EXIT_OK


def cmd_watch(args) -> int:
    src = _read(args.file)
    if not getattr(args, "unchecked", False):
        diags, code = _static_check(src, args)
        if code is not None:
            return code
        if diags:
            _emit_diags(diags, args,
                        f"rejected: {args.file} — {len(diags)} issue(s) found by the "
                        f"static checker; not watched (--unchecked overrides)")
            return EXIT_REJECTED
    vm = VM(file=args.file)
    code = EXIT_OK
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        code = EXIT_ERROR
    text = vm.tele.text()
    if text:
        sys.stdout.write(text)
        if not text.endswith("\n"):
            sys.stdout.write("\n")
    return code


def cmd_keep(args) -> int:
    if args.keep_cmd == "export":
        return _keep_export(args)
    return _keep_verify(args)


def _keep_export(args) -> int:
    from .store import seal, write_fsync
    src = _read(args.file)
    if not getattr(args, "unchecked", False):
        diags, code = _static_check(src, args)
        if code is not None:
            return code
        if diags:
            _emit_diags(diags, args,
                        f"rejected: {args.file} — {len(diags)} issue(s) found by the "
                        f"static checker; not sealed (--unchecked overrides)")
            return EXIT_REJECTED
    vm = VM(file=args.file)
    try:
        vm.run_src(src)
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return EXIT_ERROR
    write_fsync(args.output, seal(vm.graph))
    return EXIT_OK


def _keep_verify(args) -> int:
    from .errors import INTEGRITY_FAIL, Pos
    from .store import b_so
    try:
        text = open(args.file, "rb").read().decode("utf-8")
    except UnicodeDecodeError:
        _emit_error(ZownError(
            code=INTEGRITY_FAIL,
            msg="`so` bundle is not the UTF-8 text that was sealed",
            kind="sec", op="so", file=args.file,
            hint="the bundle was changed after it was sealed",
        ), args.zerr, args.zerr_out)
        return EXIT_ERROR
    except OSError as exc:
        sys.stderr.write(f"keep: {exc}\n")
        return EXIT_ERROR
    vm = VM(file=args.file)
    vm.push(text)
    try:
        b_so(vm, Pos(1, 1, 0))
    except ZownError as e:
        e.file = e.file or args.file
        _emit_error(e, args.zerr, args.zerr_out)
        return EXIT_ERROR
    sys.stdout.write(f"ok\n{vm.stack[-1]}\n")
    return EXIT_OK


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="zown", description="Zown language toolchain (reference build)")
    p.add_argument("--zerr", action="store_true", help="emit errors as JSON .zerr packets")
    p.add_argument("--zerr-out", metavar="PATH", help="also write the .zerr packet to PATH")
    p.add_argument("--unchecked", action="store_true",
                   help="run without the static check gate (debugging aid; exit 2 otherwise "
                        "rejects a program the checker faults)")
    sub = p.add_subparsers(dest="cmd")

    pr = sub.add_parser("run", help="static-check, then run a program (exit 2 if rejected)")
    pr.add_argument("file")
    pr.set_defaults(func=cmd_run)

    pc = sub.add_parser("check", help="static checker: name resolution, match shape, capability flow")
    pc.add_argument("file")
    pc.set_defaults(func=cmd_check)

    pa = sub.add_parser("ast", help="print the parsed AST as JSON")
    pa.add_argument("file")
    pa.set_defaults(func=cmd_ast)

    pm = sub.add_parser("manifest", help="generate/update the shadow manifest")
    pm.add_argument("file")
    pm.set_defaults(func=cmd_manifest)

    prp = sub.add_parser("repl", help="interactive session")
    prp.set_defaults(func=cmd_repl)

    pt = sub.add_parser("tune", help="show how many real-time lanes this machine pins")
    pt.add_argument("n", nargs="?", type=int, default=None,
                    help="requested real-time slots (default: min(2, cpus-1))")
    pt.set_defaults(func=cmd_tune)

    pf = sub.add_parser("fmt", help="print a program in canonical form")
    pf.add_argument("-w", "--write", action="store_true", help="write the file in place")
    pf.add_argument("files", nargs="+")
    pf.set_defaults(func=cmd_fmt)

    pte = sub.add_parser("test", help="run Zown programs in a directory (or one file)")
    pte.add_argument("path", nargs="?", default="tests/zn",
                     help="a .zn file or a directory of them (default tests/zn)")
    pte.set_defaults(func=cmd_test)

    pmsh = sub.add_parser("mesh", help="exchange one bundle between two IPv6 peers on this host")
    pmsh.add_argument("port", nargs="?", type=int, default=59770,
                      help="rendezvous UDP port on ::1 (default 59770)")
    pmsh.set_defaults(func=cmd_mesh)

    pd = sub.add_parser("draw", help="paint the layout node a program leaves on the stack")
    pd.add_argument("file")
    pd.set_defaults(func=cmd_draw)

    ps = sub.add_parser("stage", help="check the Zown lexer, or `stage parse` for the parser")
    ps.add_argument("mode_or_file", help="a .zn file, or parse")
    ps.add_argument("file", nargs="?", help="file to parse when the first argument is parse")
    ps.set_defaults(func=cmd_stage)

    pw = sub.add_parser("watch", help="run a program, then print its telemetry log")
    pw.add_argument("file")
    pw.set_defaults(func=cmd_watch)

    pk = sub.add_parser("keep", help="seal a graph into a bundle, or open one")
    ks = pk.add_subparsers(dest="keep_cmd", required=True)
    ke = ks.add_parser("export", help="run a program and fsync its graph")
    ke.add_argument("file")
    ke.add_argument("-o", "--output", required=True, help="bundle path to write")
    ke.set_defaults(func=cmd_keep)
    kv = ks.add_parser("verify", help="open a bundle and print its head")
    kv.add_argument("file")
    kv.set_defaults(func=cmd_keep)

    return p


def main(argv: list[str] | None = None) -> int:
    argv = list(sys.argv[1:] if argv is None else argv)
    parser = build_parser()

    # shorthand: `zown file.zn` == `zown run file.zn`
    known = {"run", "check", "ast", "manifest", "repl", "tune", "fmt", "test", "mesh", "draw", "keep", "watch", "stage"}
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
