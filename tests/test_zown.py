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
    CAP_DENIED, TYPE_MISMATCH, OVERFLOW, BOUNDS, NO_MATCH, BAD_PATTERN, UB_TRAP,
    STACK_OVERFLOW,
)
from zown.lexer import lex, T_INT, T_STR, T_OP, T_BIND, T_CAP, INT_MAX, INT_MIN
from zown.vm import VM, Big, Block, Cap, Res, WidthTag, Vec


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


def test_retired_spellings_are_not_aliases():
    # A token that used to mean two things is unbound, not a synonym.
    for src in ("vadd", "vsub", "vmul", "vsum", "vat", "clr", "i128", "u128", "i4", "f4", "b16", "d2"):
        _fails(src, NAME_UNRESOLVED)


def test_plus_and_star_are_numeric_only():
    _fails("$foo$ $bar$ +", TYPE_MISMATCH, "+")
    _fails("$ab$ 3 *", TYPE_MISMATCH, "*")


def test_string_concat_and_repeat_are_their_own_words():
    _, vm = run("$foo$ $bar$ ct")
    assert vm.stack == ["foobar"]
    _, vm = run("$n=$ 5 s ct")
    assert vm.stack == ["n=5"]
    _, vm = run("$ab$ 3 rp")
    assert vm.stack == ["ababab"]
    _, vm = run("$ab$ 2 _ rp $ab$ 0 rp")
    assert vm.stack == ["", ""]


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


# --- numeric model: checked int + big (M8c-i, D1) -----------------------------
def _fails(src, code, op=None):
    try:
        run(src)
        assert False, f"expected {code} from {src!r}"
    except ZownError as e:
        assert e.code == code, (src, e.code, e.msg)
        if op is not None:
            assert e.op == op, (src, e.op)
        return e


def test_int_is_checked_i64():
    _, vm = run(f"{INT_MAX} 0 +")
    assert vm.stack == [INT_MAX]
    _fails(f"{INT_MAX} 1 +", OVERFLOW, "+")
    _fails(f"{INT_MAX} _ 1 - _", OVERFLOW, "_")     # -(INT_MIN) does not fit
    _fails(f"{INT_MAX} 2 *", OVERFLOW, "*")
    _fails(f"{INT_MAX} _ 1 - ab", OVERFLOW, "ab")
    _fails("2 63 pw", OVERFLOW, "pw")
    _fails("2 100 pw", OVERFLOW, "pw")               # refused before computing a giant
    _, vm = run("2 62 pw")
    assert vm.stack == [1 << 62]


def test_int_literal_must_fit():
    e = _fails(str(INT_MAX + 1), OVERFLOW)
    assert e.kind == "lex"
    assert lex(str(INT_MAX))[0].value == INT_MAX


def test_big_is_explicit_and_exact():
    _, vm = run("5 b")
    assert vm.stack == [Big(5)]
    _, vm = run("2 b 100 pw")
    assert vm.stack == [Big(2 ** 100)]
    _, vm = run("$99999999999999999999$ b 1 +")       # a literal beyond int: string + b
    assert vm.stack == [Big(10 ** 20)]
    _, vm = run("5 b 3 +")                            # big ∘ int -> big
    assert vm.stack == [Big(8)]
    out, _ = run("2 b 70 pw .")
    assert out == str(2 ** 70) + "\n"


def test_big_division_modulo_and_float_promotion():
    _, vm = run("10 b 5 /")
    assert vm.stack == [Big(2)]                       # exact quotient stays big
    _, vm = run("10 b 4 /")
    assert vm.stack == [2.5]                          # inexact -> float
    _, vm = run("7 b 3 %")
    assert vm.stack == [Big(1)]
    _, vm = run("5 b 2.5 +")
    assert vm.stack == [7.5]                          # any float -> float
    _fails("10 b 0 /", DIV_ZERO, "/")


def test_big_compares_and_matches_by_value():
    _, vm = run("5 b 5 ==")
    assert vm.stack == [1]
    _, vm = run("5 b 6 <")
    assert vm.stack == [1]
    _, vm = run("0 b !")
    assert vm.stack == [1]                            # big zero is falsy
    _, vm = run("5 b [ [big] [ , $big$ ] [int] [ , $int$ ] ] ??")
    assert vm.stack == ["big"]                        # a big is not an int
    _, vm = run("5 b [ [5] [ , $five$ ] [any] [ , $other$ ] ] ??")
    assert vm.stack == ["five"]                       # literal patterns compare by value


def test_big_math_words():
    _, vm = run("9 b sq")
    assert vm.stack == [Big(3)]                       # perfect square stays big
    _, vm = run("3 b _ ab")
    assert vm.stack == [Big(3)]
    _, vm = run("7 b 4 mx")
    assert vm.stack == [Big(7)]
    _fails("$x$ b", TYPE_MISMATCH, "b")
    _fails("2.5 b", TYPE_MISMATCH, "b")


def test_width_policy_result_type_is_static():
    _, vm = run("300 u8 wr")
    assert vm.stack == [44]                           # u8 fits int -> int
    _, vm = run("5 u64 ck")
    assert vm.stack == [Big(5)]                       # u64's range exceeds int -> big
    _, vm = run("1 _ u64 wr")
    assert vm.stack == [Big(2 ** 64 - 1)]
    _, vm = run("5 b u8 wr")                          # a big input reduces like any int
    assert vm.stack == [5]


def test_float_is_ieee_and_non_finite_conversions_trap():
    out, _ = run("10.5 400 pw . 10.5 400 pw _ . 10.5 400 pw = - .")
    assert out == "inf\n-inf\nnan\n"                  # overflow to inf is a value, not a crash
    out, _ = run("10.0 30 pw .")
    assert out == "1e+30\n"                           # whole float beyond int stays a float
    _fails("10.5 400 pw fl", OVERFLOW, "fl")
    _fails("10.5 400 pw = - fl", UB_TRAP, "fl")       # nan -> int is undefined: trapped
    _fails("10.0 30 pw rd", OVERFLOW, "rd")
    _fails("4 _ sq", UB_TRAP, "sq")
    _fails("2 _ 0.5 pw", UB_TRAP, "pw")
    _fails("0 1 _ pw", DIV_ZERO, "pw")


def test_to_num_respects_int_range():
    _fails("$99999999999999999999$ n", OVERFLOW, "n")
    _, vm = run("$99999999999999999999$ b")
    assert vm.stack == [Big(10 ** 20 - 1)]
    _, vm = run("$inf$ n")
    assert vm.stack == [float("inf")]


def test_big_snapshot_is_json_safe():
    e = _fails("5 b $x$ -", TYPE_MISMATCH, "-")
    pkt = e.packet()
    assert pkt["stack"] == ["big:5"]                  # snapshot after the offending pop
    json.dumps(pkt)


# --- SIMD vectors (v0.2) -------------------------------------------------------
def test_vec_construct_and_shape():
    _, vm = run("[ 1 2 3 4 ] vi")
    v = vm.stack[0]
    assert isinstance(v, Vec) and v.name == "vi" and v.lanes == [1, 2, 3, 4]


def test_vec_elementwise_ops():
    _, vm = run("[ 1 2 3 4 ] vi [ 10 20 30 40 ] vi va")
    assert vm.stack[0].lanes == [11, 22, 33, 44]
    _, vm = run("[ 5 5 5 5 ] vi [ 1 2 3 4 ] vi vs")
    assert vm.stack[0].lanes == [4, 3, 2, 1]


def test_vec_integer_lanes_wrap():
    src = "[ 250 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 ] vu " \
          "[ 10 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 ] vu va 0 at"
    _, vm = run(src)
    assert vm.stack == [4]  # 260 wraps to 4 in u8


def test_vec_sum_and_at():
    _, vm = run("[ 1 2 3 4 ] vi vh")
    assert vm.stack == [10]
    _, vm = run("[ 7 8 9 10 ] vi 2 at")
    assert vm.stack == [9]


def test_vec_wrong_arity_is_bounds():
    try:
        run("[ 1 2 3 ] vi")
        assert False
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "vi"


def test_vec_type_mismatch():
    try:
        run("[ 1 2 3 4 ] vi [ 1.0 2.0 ] vd va")
        assert False
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "va"


# --- pattern matching (v0.2) ---------------------------------------------------
def _match_label(subject_src):
    arms = "[ [int] [ , $int$ ] [str] [ , $str$ ] [float] [ , $float$ ] " \
           "[any] [ , $other$ ] ]"
    _, vm = run(f"{subject_src} {arms} ??")
    return vm.stack[-1]


def test_match_by_type():
    assert _match_label("42") == "int"
    assert _match_label("$hi$") == "str"
    assert _match_label("3.5") == "float"
    assert _match_label("[ 1 ]") == "other"  # a block hits the default arm here


def test_match_literal_and_subject_in_body():
    # body runs with the matched subject on top of the stack
    _, vm = run("7 [ [7] [ ] [any] [ , 0 ] ] ??")
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


# --- linear bindings: 'x move (M8c-iv, D4) -------------------------------------
def test_lex_move_token():
    from zown.lexer import T_MOVE
    toks = [t for t in lex("'x") if t.kind != "EOF"]
    assert toks[0].kind == T_MOVE and toks[0].value == "x"
    _fails("' ", REPAIR_SYNTAX)                          # a bare apostrophe


def test_move_pushes_and_unbinds():
    out, vm = run("5 :x 'x . 7 :x x .")                  # move out, rebind, use again
    assert out == "5\n7\n" and vm.env == {"x": 7}
    _fails("5 :x 'x . x", NAME_UNRESOLVED, "x")         # a bare name after a move
    _fails("5 :x 'x 'x", NAME_UNRESOLVED, "'x")         # moving twice
    _fails("'zz", NAME_UNRESOLVED, "'zz")               # never bound
    _fails("'ln", NAME_UNRESOLVED, "'ln")               # builtins are not bindings
    _, vm = run("[ 1 2 + ] :f 'f @")                    # a block moves out and runs
    assert vm.stack == [3] and vm.env == {}


def test_check_use_after_move():
    for src in ("5 :x 'x , x", "5 :x 'x 'x", "5 :x 'x 3 + . x",
                "'x [ x ] @ 5 :x",                         # inline block inherits the mark
                "5 :x 'x , [ x ] [ ] ;",                   # the loop condition runs first
                "5 :x 'x , 1 [ x ] [ ] ? @"):              # selected branch runs at once
        diags = _check(src)
        assert [d.code for d in diags] == [NAME_UNRESOLVED], src
    assert [d.op for d in _check("'zz")] == ["'zz"]     # never bound: well-formedness


def test_check_move_stays_silent_when_it_cannot_know():
    assert _check("5 :x 'x , 6 :x x") == []              # rebound
    assert _check("[ 1 ] :f 5 :x 'x , f @ x") == []      # f @ may have rebound x
    assert _check("5 :s 's , s") == []                   # falls back to the builtin s
    assert _check("0 :c 5 :x 'x , [ 5 :x c 1 < ] [ x ] ;") == []   # cond rebinds before body
    assert _check("5 :x 'x , [ x ] ty [ [any] [ , ] ] ??") == []    # inside at: handled
    assert [d.code for d in _check("5 :x [ 'x , x ] :g")] == [NAME_UNRESOLVED]  # within one block


# --- no undefined behavior: structured traps (M8c-iii) -------------------------
def test_unbounded_recursion_is_a_structured_trap():
    e = _fails("[ f @ ] :f f @", STACK_OVERFLOW, "@")
    assert e.pos is not None                          # points at the recursive call site
    _fails("[ g @ ] :g `s [ g @ ] gr", STACK_OVERFLOW)  # through gr too


def test_depth_counts_nesting_not_iterations():
    out, _ = run("0 :n [ n 300 < ] [ n 1 + :n ] ; n .")
    assert out == "300\n"                             # loops never accumulate depth
    _, vm = run("[ 1 ] :o [ o @ o @ ] :p p @")
    assert vm.stack == [1, 1] and vm.depth == 0       # depth unwinds after each invoke


def test_recursion_trap_is_a_catchable_result():
    _, vm = run("[ f @ ] :f [ f @ ] ty")
    assert vm.stack == [Res(False, "STACK_OVERFLOW")]


def test_deep_block_nesting_is_a_parse_error_not_a_crash():
    from zown.parser import MAX_NESTING
    e = _fails("[" * (MAX_NESTING + 1) + "]" * (MAX_NESTING + 1), REPAIR_SYNTAX)
    assert e.kind == "parse"
    _, vm = run("[" * MAX_NESTING + "]" * MAX_NESTING + " ,")
    assert vm.stack == []


def test_defined_behavior_rows():
    _fails("1.5 0 /", DIV_ZERO, "/")                  # float division by zero traps too
    _fails("1 0 %", DIV_ZERO, "%")
    _, vm = run("7 _ 3 %  7 3 _ %")
    assert vm.stack == [2, -2]                        # modulo takes the divisor's sign
    _, vm = run("$ab$ 2 _ rp  $ab$ 0 rp")
    assert vm.stack == ["", ""]                       # repeat with count <= 0 is ""
    _fails("[ 1 2 3 4 ] vi 4 at", BOUNDS, "at")     # lane index out of range


# --- result values (M8c-ii, D3) ------------------------------------------------
def test_ty_wraps_the_single_value_or_the_error_code():
    _, vm = run("5 :x 2 :y [ x y / ] ty")
    assert vm.stack == [Res(True, 2.5)]
    _, vm = run("1 :x 0 :y [ x y / ] ty")
    assert vm.stack == [Res(False, "DIV_ZERO")]
    _, vm = run("[ `s rq 1 ] ty")                    # a security fault is a value too
    assert vm.stack == [Res(False, "CAP_DENIED")]
    _, vm = run("7 [ 1 ] ty")                        # isolated stack: outer values untouched
    assert vm.stack == [7, Res(True, 1)]


def test_ty_body_must_leave_exactly_one_value():
    _fails("[ 1 2 ] ty", BOUNDS, "ty")
    _fails("[ ] ty", BOUNDS, "ty")
    _, vm = run("5 [ 2 / ] ty")                      # the body cannot see the outer 5
    assert vm.stack == [5, Res(False, "STACK_UNDERFLOW")]


def test_result_payload_is_reachable_only_through_match():
    out, _ = run("[ 6 ] ty [ [ok] [ 2 * . ] [err] [ . ] ] ??")
    assert out == "12\n"                             # the ok arm receives the unwrapped 6
    out, _ = run("[ 1 0 / ] ty [ [ok] [ . ] [err] [ . ] ] ??")
    assert out == "DIV_ZERO\n"                       # the err arm receives the code
    _, vm = run("[ 1 ] ty [ [res] [ ] ] ??")
    assert vm.stack == [Res(True, 1)]                # [res] is the type pattern: no unwrap
    _fails("[ 1 ] ty 1 +", TYPE_MISMATCH, "+")       # opaque to every other operator
    _fails("[ 1 0 / ] ty [ [ok] [ . ] ] ??", NO_MATCH, "??")


def test_result_truthiness_is_its_ok_flag():
    _, vm = run("[ 1 ] ty ! [ 1 0 / ] ty !")
    assert vm.stack == [0, 1]
    out, _ = run("[ 1 0 / ] ty [ $good$ ] [ $bad$ ] ? @ .")
    assert out == "bad\n"


def test_result_display_and_snapshot():
    out, _ = run("[ 5 ] ty . [ 1 0 / ] ty .")
    assert out == "ok(5)\nerr(DIV_ZERO)\n"
    e = _fails("[ 5 ] ty $x$ -", TYPE_MISMATCH)
    assert e.packet()["stack"] == ["ok:5"]           # after popping the offending $x$


def test_check_result_match_must_be_exhaustive():
    assert [d.code for d in _check("[ 1 0 / ] ty [ [ok] [ . ] ] ??")] == [NO_MATCH]
    assert [d.code for d in _check("[ 1 ] ty [ [err] [ . ] ] ??")] == [NO_MATCH]
    assert _check("[ 1 ] ty [ [ok] [ . ] [err] [ . ] ] ??") == []
    assert _check("[ 1 ] ty [ [ok] [ . ] [any] [ , ] ] ??") == []
    assert _check("[ 1 ] ty [ [res] [ , ] ] ??") == []
    assert _check("5 [ [ok] [ . ] ] ??") == []        # subject not known to be a result: silent


def test_check_faults_inside_ty_are_the_handled_path():
    # a capability denial inside `ty` is caught -> no static CAP_DENIED
    assert _check("[ `s rq 1 ] ty [ [err] [ . ] [ok] [ , ] ] ??") == []
    # ...and an escaped block that might run under `ty` is not judged either
    assert _check("[ `s rq ] :f  f ty [ [err] [ . ] [ok] [ , ] ] ??") == []
    # but well-formedness still holds inside `ty`: a typo is a typo
    assert [d.code for d in _check("[ zz ] ty [ [any] [ , ] ] ??")] == [NAME_UNRESOLVED]


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
    assert _check("ln up 300 u8 wr [ 1 2 3 4 ] vi vh") == []


def test_check_match_patterns_are_not_names():
    # `int`/`str` inside patterns must not be reported as unbound names
    assert _check("42 [ [int] [ , $i$ ] [str] [ , $s$ ] [any] [ , $o$ ] ] ??") == []


def test_check_match_body_names_are_checked():
    diags = _check("42 [ [int] [ , bogus ] [any] [ , $o$ ] ] ??")
    assert len(diags) == 1 and diags[0].code == NAME_UNRESOLVED


def test_check_flags_unrecognized_pattern_statically():
    diags = _check("1 [ [ foo ] [ , $x$ ] ] ??")
    assert len(diags) == 1 and diags[0].code == BAD_PATTERN


def test_check_flags_odd_match_arms_statically():
    diags = _check("1 [ [int] ] ??")
    assert len(diags) == 1 and diags[0].code == BAD_PATTERN


# --- static capability flow (M8b) ----------------------------------------------
def _denied(src):
    """The CAP_DENIED diagnostics for src (and assert nothing else was raised)."""
    diags = _check(src)
    assert all(d.code == CAP_DENIED for d in diags), [d.code for d in diags]
    return diags


def test_capflow_top_level_require_is_static_cap_denied():
    diags = _denied("`s rq")
    assert len(diags) == 1
    d = diags[0]
    assert d.op == "rq" and d.cap == "s" and d.kind == "check"
    assert d.packet()["cap"] == "s"          # same packet shape as the runtime error


def test_capflow_literal_grant_is_clean():
    assert _denied("`s [ `s rq $ok$ . ] gr") == []


def test_capflow_wrong_capability_granted():
    # exact authority in scope is {r}; `s can never be satisfied there
    diags = _denied("`r [ `s rq ] gr")
    assert len(diags) == 1 and diags[0].cap == "s"


def test_capflow_nested_grants_accumulate():
    assert _denied("`s [ `r [ `s rq `r rq ] gr ] gr") == []


def test_capflow_authority_ends_with_the_grant_block():
    src = "`s [ `s rq ] gr  `s rq"
    diags = _denied(src)
    assert len(diags) == 1
    assert diags[0].pos.offset > src.index("gr")        # only the trailing rq is flagged


def test_capflow_dynamic_extent_is_respected():
    # grants are dynamic, not lexical: a bound block invoked under a grant is fine
    assert _denied("[ `s rq ] :send   `s [ send @ ] gr") == []
    # ...and so is a block that reaches `gr` through the stack rather than literally
    assert _denied("[ `s rq ]  `s \\ gr") == []
    # ...or a grant whose capability arrived through a name
    assert _denied("`s :k  k [ `s rq ] gr") == []


def test_capflow_never_granted_anywhere_is_denied_even_when_escaped():
    diags = _denied("[ `s rq ] :send   `r [ send @ ] gr")
    assert len(diags) == 1 and "never granted" in diags[0].msg
    assert len(_denied("[ `s rq ] :f")) == 1    # bound and never granted -> denied


def test_capflow_inline_consumers_inherit_authority():
    # @ / ; / ? @ / ?? / vector constructors run a literal block in the caller's scope
    assert _denied("`s [ [ `s rq ] @ ] gr") == []
    assert _denied("`s [ 1 [ [int] [ , `s rq ] ] ?? ] gr") == []
    assert _denied("`s [ [ `s rq 1 2 3 4 ] vi , ] gr") == []
    for src in ("[ `s rq ] @", "0 [ `s rq ] [ ] ? @", "0 [ ] [ `s rq ] ? @",
                "[ `s hv ] [ `s rq ] ;", "[ `s rq ] [ ] ;", "1 [ [int] [ , `s rq ] ] ??"):
        assert len(_denied(src)) == 1, src


def test_capflow_shadowed_words_are_honored():
    assert _denied("5 :rq  `s rq") == []                 # no require word in this program
    assert len(_denied("5 :gr  `s [ `s rq ] gr")) == 1  # nothing can ever be granted


def test_check_corpus_has_no_false_positives():
    # Soundness guard: every program that runs successfully must check clean.
    import glob
    from zown.checker import check_src
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    files = []
    for d in ("examples", os.path.join("conformance", "cases"),
              os.path.join("conformance", "cases_v2")):
        files += glob.glob(os.path.join(root, d, "*.zn"))
    assert files
    for f in files:
        with open(f, encoding="utf-8") as fh:
            diags = check_src(fh.read(), f)
        assert diags == [], (f, [(d.code, d.op) for d in diags])


# --- cli: `run` is gated by `check` (M8c-0, D2) --------------------------------
def _cli(argv, src):
    """Run the CLI on a temp program; return (exit_code, stdout, stderr)."""
    import contextlib
    from zown.cli import main
    d = tempfile.mkdtemp()
    path = os.path.join(d, "prog.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(argv + [path])
    return code, out.getvalue(), err.getvalue()


def test_cli_run_refuses_a_program_the_checker_faults():
    code, out, err = _cli(["--zerr", "run"], "`s rq $never$ .")
    assert code == 2                                   # rejected, distinct from a runtime error
    assert out == ""                                   # nothing executed
    pkts = json.loads(err)
    assert isinstance(pkts, list) and pkts[0]["code"] == CAP_DENIED
    assert pkts[0]["kind"] == "check" and pkts[0]["zerr"] == "0.2"


def test_cli_run_clean_program_executes():
    code, out, err = _cli(["run"], "[$hi$.]:h h@")
    assert code == 0 and out == "hi\n" and err == ""


def test_cli_run_unchecked_reaches_the_runtime_error():
    code, out, err = _cli(["--unchecked", "--zerr", "run"], "`s rq")
    assert code == 1
    pkt = json.loads(err)
    assert pkt["code"] == CAP_DENIED and pkt["kind"] == "sec"   # the runtime packet


def test_cli_run_shorthand_is_gated_too():
    code, _, err = _cli(["--zerr"], "zz .")           # `zown file.zn` == `zown run file.zn`
    assert code == 2 and json.loads(err)[0]["code"] == NAME_UNRESOLVED


def test_cli_check_exit_codes():
    assert _cli(["check"], "1 2 + .")[0] == 0
    assert _cli(["check"], "zz")[0] == 2
    assert _cli(["check"], "[ 1 2")[0] == 1            # a parse error is not a static finding


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
