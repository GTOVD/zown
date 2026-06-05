"""Zown reference tests.

Runs with plain `python3 tests/test_zown.py` (no dependencies) and is also
pytest-compatible. Each `test_*` function asserts; the bottom runner discovers
and executes them, printing a summary.
"""

import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from zown.errors import STACK_UNDERFLOW, ZownError, REPAIR_SYNTAX, DIV_ZERO, NAME_UNRESOLVED
from zown.lexer import lex, T_INT, T_STR, T_OP, T_BIND
from zown.vm import VM, Block


def run(src):
    """Run source, returning (printed_output, vm)."""
    buf = io.StringIO()
    vm = VM(file="<test>", out=buf)
    vm.run_src(src)
    return buf.getvalue(), vm


# --- lexer ---------------------------------------------------------------------
def test_lex_dollar_string():
    toks = lex("$Hello, World!$")
    assert toks[0].kind == T_STR
    assert toks[0].value == "Hello, World!"


def test_lex_number_vs_print_dot():
    # `5.` is push-5 then print, NOT the float 5.0
    toks = [t for t in lex("5.") if t.kind != "EOF"]
    assert toks[0].kind == T_INT and toks[0].value == 5
    assert toks[1].kind == T_OP and toks[1].value == "."
    # `5.5` IS a float
    toks2 = [t for t in lex("5.5") if t.kind != "EOF"]
    assert toks2[0].value == 5.5


def test_lex_two_char_ops():
    vals = [t.value for t in lex("== != <= >= && ||") if t.kind == T_OP]
    assert vals == ["==", "!=", "<=", ">=", "&&", "||"]


def test_lex_bind():
    toks = lex("5:x")
    assert toks[0].kind == T_INT
    assert toks[1].kind == T_BIND and toks[1].value == "x"


def test_lex_lone_pipe_reserved():
    try:
        lex("|")
        assert False, "expected reserved-pipe error"
    except ZownError as e:
        assert e.code == REPAIR_SYNTAX


# --- vm: core ------------------------------------------------------------------
def test_hello_world():
    out, _ = run("[$Hello, World!$.]:h h@")
    assert out == "Hello, World!\n"


def test_arithmetic_rpn():
    _, vm = run("10 20 *")
    assert vm.stack == [200]
    _, vm = run("7 3 -")
    assert vm.stack == [4]
    _, vm = run("10 4 /")  # whole-valued float collapses to int
    assert vm.stack == [2.5]
    _, vm = run("10 3 %")
    assert vm.stack == [1]


def test_string_concat_with_plus():
    _, vm = run("$foo$ $bar$ +")
    assert vm.stack == ["foobar"]
    _, vm = run("$n=$ 5 +")
    assert vm.stack == ["n=5"]


def test_bindings_push_value():
    _, vm = run("5:x x x +")
    assert vm.stack == [10]


def test_select_and_invoke():
    _, vm = run("1 [$y$] [$n$] ? @")
    assert vm.stack == ["y"]
    _, vm = run("0 [$y$] [$n$] ? @")
    assert vm.stack == ["n"]


def test_while_loop_counts():
    out, _ = run("0:c [ c 3 < ] [ c . c 1 + :c ] ;")
    assert out == "0\n1\n2\n"


def test_comparisons():
    for src, want in [("3 4 <", 1), ("4 3 <", 0), ("5 5 ==", 1),
                      ("5 6 !=", 1), ("4 4 >=", 1), ("2 9 <=", 1)]:
        _, vm = run(src)
        assert vm.stack == [want], (src, vm.stack)


def test_stack_ops():
    _, vm = run("1 2 =")        # dup
    assert vm.stack == [1, 2, 2]
    _, vm = run("1 2 \\")       # swap
    assert vm.stack == [2, 1]
    _, vm = run("1 2 ,")        # drop
    assert vm.stack == [1]
    _, vm = run("7 8 &")        # over
    assert vm.stack == [7, 8, 7]
    _, vm = run("1 2 3 rt")     # rot: a b c -> b c a
    assert vm.stack == [2, 3, 1]


# --- builtins ------------------------------------------------------------------
def test_builtin_words():
    _, vm = run("$  hi  $ tr")
    assert vm.stack == ["hi"]
    _, vm = run("$abc$ up")
    assert vm.stack == ["ABC"]
    _, vm = run("$ABC$ lo")
    assert vm.stack == ["abc"]
    _, vm = run("$hello$ ln")
    assert vm.stack == [5]
    _, vm = run("9 sq")
    assert vm.stack == [3]
    _, vm = run("2 10 pw")
    assert vm.stack == [1024]
    _, vm = run("$ -3 $ tr n ab")
    assert vm.stack == [3]


# --- errors --------------------------------------------------------------------
def test_stack_underflow_packet():
    try:
        run("1 +")
        assert False
    except ZownError as e:
        assert e.code == STACK_UNDERFLOW
        pkt = e.packet()
        assert pkt["op"] == "+"
        assert pkt["kind"] == "run"
        assert "hint" in pkt


def test_div_zero():
    try:
        run("1 0 /")
        assert False
    except ZownError as e:
        assert e.code == DIV_ZERO


def test_unresolved_name():
    try:
        run("zz")
        assert False
    except ZownError as e:
        assert e.code == NAME_UNRESOLVED


def test_unclosed_block_is_parse_error():
    try:
        run("[ 1 2")
        assert False
    except ZownError as e:
        assert e.kind == "parse" and e.code == REPAIR_SYNTAX


# --- runner --------------------------------------------------------------------
def _main():
    tests = sorted(
        (name, fn) for name, fn in globals().items()
        if name.startswith("test_") and callable(fn)
    )
    passed = failed = 0
    for name, fn in tests:
        try:
            fn()
            passed += 1
        except Exception as exc:  # noqa: BLE001
            failed += 1
            print(f"FAIL {name}: {exc!r}")
    print(f"\n{passed} passed, {failed} failed ({len(tests)} total)")
    return 1 if failed else 0


if __name__ == "__main__":
    raise SystemExit(_main())
