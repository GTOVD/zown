"""Zown reference tests.

Runs with plain `python3 tests/test_zown.py` (no dependencies) and is also
pytest-compatible. Each `test_*` function asserts; the bottom runner discovers
and executes them, printing a summary.
"""

import io
import json
import os
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from zown.errors import (
    STACK_UNDERFLOW, ZownError, REPAIR_SYNTAX, DIV_ZERO, NAME_UNRESOLVED,
    CAP_DENIED, TYPE_MISMATCH, OVERFLOW, BOUNDS, NO_MATCH, BAD_PATTERN,
)
from zown.lexer import lex, T_INT, T_STR, T_OP, T_BIND, T_CAP
from zown.vm import VM, Block, Cap, WidthTag, Vec


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


# --- capabilities (v0.2) -------------------------------------------------------
def test_lex_capability_token():
    toks = [t for t in lex("`s") if t.kind != "EOF"]
    assert toks[0].kind == T_CAP and toks[0].value == "s"


def test_lex_bare_backtick_is_error():
    try:
        lex("` ")
        assert False, "expected error for a bare backtick"
    except ZownError as e:
        assert e.code == REPAIR_SYNTAX


def test_capability_token_pushes_cap_value():
    _, vm = run("`net")
    assert vm.stack == [Cap("net")]


def test_zero_authority_by_default():
    _, vm = run("`s hv")
    assert vm.stack == [0]  # nothing granted to start


def test_grant_scopes_then_restores():
    out, vm = run("`s [ `s rq $ok$ . `s hv . ] gr `s hv")
    assert out == "ok\n1\n"     # required ok; held inside the grant
    assert vm.stack == [0]      # authority restored after the block


def test_nested_grants_unwind_independently():
    out, vm = run("`r [ `s [ `r rq `s rq $both$ . ] gr `s hv ] gr")
    assert out == "both\n"
    assert vm.stack == [0]      # inner `s grant ended; outer `r still held but unprobed


def test_require_without_grant_is_cap_denied():
    try:
        run("`s rq")
        assert False
    except ZownError as e:
        assert e.code == CAP_DENIED
        pkt = e.packet()
        assert pkt["kind"] == "sec"
        assert pkt["op"] == "rq"
        assert pkt["cap"] == "s"


def test_require_needs_a_capability_value():
    try:
        run("5 rq")
        assert False
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "rq"


# --- fixed-width numerics (v0.2) -----------------------------------------------
def test_width_tag_pushes_value():
    _, vm = run("u8")
    assert vm.stack == [WidthTag(False, 8)]
    _, vm = run("i16")
    assert vm.stack == [WidthTag(True, 16)] and vm.stack[0].lo == -32768


def test_wrap_two_complement():
    for src, want in [("300 u8 wr", 44), ("1 _ u8 wr", 255), ("128 i8 wr", -128),
                      ("65536 u16 wr", 0), ("255 u8 wr", 255)]:
        _, vm = run(src)
        assert vm.stack == [want], (src, vm.stack)


def test_saturate_clamps():
    for src, want in [("300 u8 st", 255), ("1 _ u8 st", 0), ("200 _ i8 st", -128),
                      ("200 i8 st", 127), ("50 i8 st", 50)]:
        _, vm = run(src)
        assert vm.stack == [want], (src, vm.stack)


def test_check_passes_in_range():
    _, vm = run("127 i8 ck 255 u8 ck")
    assert vm.stack == [127, 255]


def test_check_overflow_is_structured():
    try:
        run("256 u8 ck")
        assert False
    except ZownError as e:
        assert e.code == OVERFLOW and e.op == "ck"


def test_width_policy_needs_a_width_tag():
    try:
        run("5 wr")
        assert False
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "wr"


def test_width_policy_rejects_fractional_float():
    try:
        run("3.5 u8 wr")
        assert False
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "wr"


# --- SIMD vectors (v0.2) -------------------------------------------------------
def test_vec_construct_and_shape():
    _, vm = run("[ 1 2 3 4 ] i4")
    v = vm.stack[0]
    assert isinstance(v, Vec) and v.name == "i4" and v.lanes == [1, 2, 3, 4]


def test_vec_elementwise_ops():
    _, vm = run("[ 1 2 3 4 ] i4 [ 10 20 30 40 ] i4 vadd")
    assert vm.stack[0].lanes == [11, 22, 33, 44]
    _, vm = run("[ 5 5 5 5 ] i4 [ 1 2 3 4 ] i4 vsub")
    assert vm.stack[0].lanes == [4, 3, 2, 1]


def test_vec_integer_lanes_wrap():
    src = "[ 250 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 ] b16 " \
          "[ 10 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 ] b16 vadd 0 vat"
    _, vm = run(src)
    assert vm.stack == [4]  # 260 wraps to 4 in u8


def test_vec_sum_and_at():
    _, vm = run("[ 1 2 3 4 ] i4 vsum")
    assert vm.stack == [10]
    _, vm = run("[ 7 8 9 10 ] i4 2 vat")
    assert vm.stack == [9]


def test_vec_wrong_arity_is_bounds():
    try:
        run("[ 1 2 3 ] i4")
        assert False
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "i4"


def test_vec_type_mismatch():
    try:
        run("[ 1 2 3 4 ] i4 [ 1.0 2.0 ] d2 vadd")
        assert False
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "vadd"


# --- pattern matching (v0.2) ---------------------------------------------------
def _match_label(subject_src):
    arms = "[ [int] [ , $int$ ] [str] [ , $str$ ] [float] [ , $float$ ] " \
           "[_] [ , $other$ ] ]"
    _, vm = run(f"{subject_src} {arms} ??")
    return vm.stack[-1]


def test_match_by_type():
    assert _match_label("42") == "int"
    assert _match_label("$hi$") == "str"
    assert _match_label("3.5") == "float"
    assert _match_label("[ 1 ]") == "other"  # a block hits the default arm here


def test_match_literal_and_subject_in_body():
    # body runs with the matched subject on top of the stack
    _, vm = run("7 [ [7] [ ] [_] [ , 0 ] ] ??")
    assert vm.stack == [7]


def test_match_first_arm_wins():
    _, vm = run("5 [ [int] [ , $a$ ] [int] [ , $b$ ] ] ??")
    assert vm.stack == ["a"]


def test_match_no_arm_is_no_match():
    try:
        run("9 [ [1] [ , $x$ ] ] ??")
        assert False
    except ZownError as e:
        assert e.code == NO_MATCH and e.op == "??"


def test_match_odd_arms_is_bad_pattern():
    try:
        run("1 [ [int] ] ??")
        assert False
    except ZownError as e:
        assert e.code == BAD_PATTERN and e.op == "??"


def test_match_unknown_pattern_is_bad_pattern():
    try:
        run("1 [ [ foo ] [ , $x$ ] ] ??")
        assert False
    except ZownError as e:
        assert e.code == BAD_PATTERN and e.op == "??"


def test_match_op_is_two_char():
    from zown.lexer import lex, T_OP
    toks = [t for t in lex("a ?? b ?") if t.kind == T_OP]
    assert [t.value for t in toks] == ["??", "?"]


# --- static checker (M8a) ------------------------------------------------------
def _check(src):
    from zown.checker import check_src
    return check_src(src, "<test>")


def test_check_clean_program_has_no_diags():
    assert _check("5 3 + . [ =, ] :dup2") == []


def test_check_flags_unbound_name():
    diags = _check("5 frobnicate .")
    assert len(diags) == 1
    assert diags[0].code == NAME_UNRESOLVED and diags[0].op == "frobnicate"


def test_check_accepts_name_bound_anywhere():
    # binding is global; a use before its textual bind still resolves
    assert _check("x . 5 :x") == []


def test_check_does_not_flag_builtins_or_widths():
    assert _check("ln up 300 u8 wr [ 1 2 3 4 ] i4 vsum") == []


def test_check_match_patterns_are_not_names():
    # `int`/`str` inside patterns must not be reported as unbound names
    assert _check("42 [ [int] [ , $i$ ] [str] [ , $s$ ] [_] [ , $o$ ] ] ??") == []


def test_check_match_body_names_are_checked():
    diags = _check("42 [ [int] [ , bogus ] [_] [ , $o$ ] ] ??")
    assert len(diags) == 1 and diags[0].code == NAME_UNRESOLVED


def test_check_flags_unrecognized_pattern_statically():
    diags = _check("1 [ [ foo ] [ , $x$ ] ] ??")
    assert len(diags) == 1 and diags[0].code == BAD_PATTERN


def test_check_flags_odd_match_arms_statically():
    diags = _check("1 [ [int] ] ??")
    assert len(diags) == 1 and diags[0].code == BAD_PATTERN


# --- manifest v2 (v0.2) --------------------------------------------------------
def _gen_manifest(src, prior=None):
    from zown.manifest import generate, manifest_path
    d = tempfile.mkdtemp()
    znp = os.path.join(d, "app.zn")
    open(znp, "w", encoding="utf-8").write(src)
    if prior is not None:
        open(manifest_path(znp), "w", encoding="utf-8").write(json.dumps(prior))
    return generate(znp)


def test_manifest_v2_scaffolds_fields():
    m = _gen_manifest("[ `s rq `r rq $hi$ . ]:get  5:n")
    assert m["language"] == "Zown v0.2"
    # module provenance block exists with crypto fields unset (never fabricated)
    assert set(m["module"]) >= {"content", "author", "sig", "ver", "deps"}
    assert m["module"]["content"] is None and m["module"]["ver"] == "0.0.0"
    get = m["symbols"]["get"]
    assert get["type"] == "block"
    assert get["caps"] == ["`r", "`s"]              # discovered from the block body
    assert get["sec"] == {"ct": False, "secret": False}
    assert get["tele"] == {"latency": False, "errors": []}
    assert get["i18n"] == {"keys": []}
    assert m["symbols"]["n"]["caps"] == []          # a value binding needs no caps


def test_manifest_builtins_stay_lean():
    m = _gen_manifest("$ hi $ tr")
    tr = m["symbols"]["tr"]
    assert tr["type"] == "builtin" and tr["alias"] == "trim"
    assert "caps" not in tr and "sec" not in tr      # builtins keep the v1 shape


def test_manifest_never_clobbers_and_merges_caps():
    prior = {
        "language": "Zown v0.2",
        "source": "app.zn",
        "module": {"author": "zown:node:abc", "ver": "1.2.0"},
        "symbols": {
            "get": {
                "type": "block",
                "alias": "http_get",
                "desc": "Fetch a resource.",
                "ai_hint": "idempotent",
                "caps": ["`k"],                      # hand-added, not in the source
                "sec": {"ct": True, "secret": False},
                "i18n": {"keys": ["err.timeout"]},
            }
        },
    }
    m = _gen_manifest("[ `s rq $x$ . ]:get", prior=prior)
    get = m["symbols"]["get"]
    assert get["alias"] == "http_get"                # authored prose preserved
    assert get["desc"] == "Fetch a resource."
    assert get["caps"] == ["`k", "`s"]               # hand-added kept + discovered merged
    assert get["sec"] == {"ct": True, "secret": False}
    assert get["i18n"] == {"keys": ["err.timeout"]}
    assert m["module"]["author"] == "zown:node:abc"  # authored provenance preserved
    assert m["module"]["ver"] == "1.2.0"


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
