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
    STACK_OVERFLOW, NOT_CALLABLE, UNSUPPORTED, TEST_FAIL, INTEGRITY_FAIL, SIG_INVALID,
)
from zown.lexer import lex, T_INT, T_STR, T_OP, T_BIND, T_CAP, INT_MAX, INT_MIN
from zown.vm import VM, Big, Block, Cap, Res, WidthTag, Vec


def run(src, cpus=None):
    """Run source, returning (printed_output, vm). `cpus` pins the lane topology."""
    buf = io.StringIO()
    vm = VM(file="<test>", out=buf)
    if cpus is not None:
        vm.lanes.cpus_override = cpus
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


def test_lex_lane_ops():
    vals = [t.value for t in lex("~m ~n | ^ ||") if t.kind == T_OP]
    assert vals == ["~m", "~n", "|", "^", "||"]


def test_lex_tilde_needs_a_name():
    for src in ("~", "~1"):
        try:
            lex(src)
            assert False, src
        except ZownError as e:
            assert e.code == REPAIR_SYNTAX


def test_lex_lone_paren_is_still_an_error():
    try:
        lex("(")
        assert False, "expected a lex error"
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


def test_fuzzer_never_panics_and_has_no_raw_memory_fault():
    """Random programs must return a value or a .zerr, never a host crash.

    There is no raw pointer (§21 is design-only), so an out-of-bounds or
    use-after-free state is not representable. The fuzzer checks that stays true.
    """
    import random
    rng = random.Random(21)
    atoms = ["0", "1", "2", "3", "5", "8", "$a$", "$b$", "s", "n", "b",
             "+", "-", "*", "/", "%", "_", "==", "<", "ct", "rp", "=", ",",
             ".", "dp", "ln", "ty", "cl"]
    for _ in range(80):
        parts = [rng.choice(atoms) for _ in range(rng.randint(1, 10))]
        # A block that does not loop, so the fuzzer cannot hang.
        if rng.random() < 0.3:
            parts.append("[ 1 2 + ] @")
        src = " ".join(parts)
        try:
            run(src)
        except ZownError:
            pass
        from zown.checker import check_src
        try:
            check_src(src)
        except ZownError:
            pass


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
    os.environ.pop("ZOWN_SIGN", None)
    d = tempfile.mkdtemp()
    znp = os.path.join(d, "app.zn")
    open(znp, "w", encoding="utf-8").write(src)
    if prior is not None:
        open(manifest_path(znp), "w", encoding="utf-8").write(json.dumps(prior))
    return generate(znp)


def test_manifest_v2_scaffolds_fields():
    m = _gen_manifest("[ `s rq `r rq $hi$ . ]:get  5:n")
    assert m["language"] == "Zown v0.2"
    # content is the source hash. sig stays null until ZOWN_SIGN is set.
    assert set(m["module"]) >= {"content", "author", "sig", "ver", "deps"}
    from zown.blake3 import blake3_hex
    src = "[ `s rq `r rq $hi$ . ]:get  5:n"
    assert m["module"]["content"] == "zown:" + blake3_hex(src.encode())
    assert m["module"]["sig"] is None and m["module"]["log"] == ""
    assert m["module"]["ver"] == "0.0.0"
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
        "module": {"author": "zown:node:abc", "ver": "1.2.0", "sig": "ed25519:kept"},
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
    assert m["module"]["sig"] == "ed25519:kept"      # no ZOWN_SIGN, so the authored sig stays
    assert m["module"]["content"].startswith("zown:")


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


# --- fast lanes (M10, SPEC §22) ------------------------------------------------
_LANES = """
1 lc
[ 7 | ] ~m
[ 9 | ] ~n
[ ] ^
3 rn
sg .
pm .
pn .
"""


def test_lanes_demo_trace():
    # One pinned slot. Tick 1 is m, n, then the only job. Ticks 2 and 3 have no job.
    out, vm = run(_LANES, cpus=8)
    assert out == "mncmnmn\n7\n9\n"
    assert vm.stack == [1]  # lc leaves the grant; the pipes were pulled


def test_lanes_not_starved_under_load():
    src = """
1 lc ,
[ 1 | ] ~m
[ 2 | ] ~n
[ ] ^ [ ] ^ [ ] ^ [ ] ^ [ ] ^
3 rn
sg .
pm .
pn .
"""
    out, _ = run(src, cpus=2)
    assert out == "mncmncmnc\n1\n2\n"


def test_lanes_work_stealing():
    src = "0 lc , [ 1 ] ^ [ 2 ] ^ 2 rn ws ."
    out, vm = run(src, cpus=4)
    assert out == "1\n"
    assert vm.stack == [1, 2]
    out, vm = run(src, cpus=1)
    assert out == "0\n"
    assert vm.stack == [1, 2]


def test_lanes_grant_and_cpus():
    out, _ = run("4 lc .", cpus=2)  # room is 1, so the grant clamps
    assert out == "1\n"
    out, _ = run("hw .", cpus=5)
    assert out == "5\n"
    os.environ["ZOWN_CPUS"] = "3"
    try:
        out, _ = run("hw .")
        assert out == "3\n"
    finally:
        del os.environ["ZOWN_CPUS"]


def test_lanes_faults():
    _fails("pm", BOUNDS)
    _fails("1 |", TYPE_MISMATCH)
    _fails("[ ] ~z", UNSUPPORTED)
    _fails("1 ^", NOT_CALLABLE)
    _fails("1 _ lc", BOUNDS)
    _fails("1 _ rn", BOUNDS)
    _fails("1.5 lc", TYPE_MISMATCH)
    _fails("1 lc [ 1 rn ] ~m 1 rn", TYPE_MISMATCH)
    out, _ = run("1 lc , [ 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | ] ~m 1 rn pm .")
    assert out == "1\n"
    _fails("1 lc [ 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | 1 | ] ~m 1 rn", BOUNDS)


def test_cli_tune():
    import contextlib
    from zown.cli import main
    os.environ["ZOWN_CPUS"] = "4"
    try:
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = main(["tune", "1"])
        assert code == 0 and err.getvalue() == ""
        assert out.getvalue() == "cpus 4\ngranted 1\nworkers 3\npinned\n"
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = main(["tune", "0"])
        assert code == 0
        assert out.getvalue() == "cpus 4\ngranted 0\nworkers 4\nfibers\n"
        out, err = io.StringIO(), io.StringIO()
        with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
            code = main(["tune", "-1"])
        assert code == 1 and ">= 0" in err.getvalue()
    finally:
        del os.environ["ZOWN_CPUS"]


# --- std words, tests, formatter (M11) -----------------------------------------
def test_std_words_match_the_old_numbers():
    _, vm = run("7 b 4 mx")
    assert isinstance(vm.stack[-1], Big) and vm.stack[-1].v == 7
    _, vm = run("7.0 7 mx")
    assert vm.stack == [7.0]
    out, _ = run("1 2 3 cl dp .")
    assert out == "0\n"
    assert "mx" in VM().words and "mn" in VM().words and "cl" in VM().words


def test_std_word_can_be_shadowed():
    _, vm = run("5 :mx mx")
    assert vm.stack == [5]


def test_test_words():
    run("[ 1 1 + 2 == ] ts")
    run("0 [ 0 ] pp")  # zero repetitions do not run the body
    _fails("[ 0 ] ts", TEST_FAIL, "ts")
    _fails("[ ] ts", BOUNDS, "ts")
    _fails("1 [ 0 ] pp", TEST_FAIL, "pp")
    _fails("1 _ [ 1 ] pp", BOUNDS, "pp")


def test_define_word():
    out, vm = run("[ = ] $dd$ wd 3 dd .")
    assert out == "3\n" and vm.stack == [3]
    _fails("1 $dd$ wd", NOT_CALLABLE, "wd")


def test_formatter_is_idempotent_and_keeps_comments():
    from zown.fmt import format_source
    src = "# keep\n1:x\n\n[x x+]\n"
    once = format_source(src)
    assert once == format_source(once)
    assert once.splitlines() == ["# keep", "1 :x", "[ x x + ]"]
    hello = open(os.path.join(os.path.dirname(__file__), "..", "examples", "hello.zn"), encoding="utf-8").read()
    assert format_source(format_source(hello)) == format_source(hello)


def test_cli_test_and_fmt(tmp_path=None):
    import contextlib
    from zown.cli import main
    from zown.tester import run_tests
    root = os.path.join(os.path.dirname(__file__), "zn")
    buf = io.StringIO()
    assert run_tests(root, write=buf) == 0
    assert "failed" in buf.getvalue() and "0 failed" in buf.getvalue()
    # fmt -w round trip
    d = tempfile.mkdtemp()
    path = os.path.join(d, "p.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("1:x x.\n")
    out, err = io.StringIO(), io.StringIO()
    with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
        code = main(["fmt", "-w", path])
    assert code == 0 and err.getvalue() == ""
    text = open(path, encoding="utf-8").read()
    assert text == "1 :x x .\n"
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        assert main(["fmt", path]) == 0
    assert out.getvalue() == text


# --- mesh (M12) ----------------------------------------------------------------
def test_blake3_official_vectors():
    from zown.blake3 import blake3
    def pattern(n):
        return bytes(i % 251 for i in range(n))
    vectors = {
        0: "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262",
        1: "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213",
        1024: "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7",
        1025: "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444",
    }
    for n, want in vectors.items():
        assert blake3(pattern(n)).hex() == want
    assert blake3(b"hello").hex() == "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"


def test_bundle_publish_and_fetch():
    out, _ = run("$hello$ ~p = . ~f .")
    assert out == (
        "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f\n"
        "hello\n"
    )
    _fails("$zz$ ~f", TYPE_MISMATCH, "~f")
    _fails("1 ~p", TYPE_MISMATCH, "~p")


def test_node_id_follows_zown_node():
    os.environ["ZOWN_NODE"] = "hello"
    try:
        out, vm = run("ni . ua .")
    finally:
        del os.environ["ZOWN_NODE"]
    nid, addr = out.splitlines()
    assert nid == "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
    assert addr.startswith("fd") and addr.count(":") == 7
    assert vm.node_id.hex() == nid


def test_two_peers_exchange_a_bundle():
    import socket
    from zown.mesh import exchange
    sock = socket.socket(socket.AF_INET6, socket.SOCK_DGRAM)
    sock.bind(("::1", 0))
    port = sock.getsockname()[1]
    sock.close()
    digest, text, addrs = exchange("hello bundle", port)
    assert text == "hello bundle"
    assert digest == __import__("zown.blake3", fromlist=["blake3"]).blake3(b"hello bundle").hex()
    left, right = addrs.split()
    assert left.startswith("fd") and right.startswith("fd") and left != right


def _stream_vm(src, args, mesh):
    import io
    os.environ["ZOWN_MESH"] = mesh
    buf = io.StringIO()
    vm = VM(file="<stream>", out=buf, args=args)
    try:
        vm.run_src(src)
        return buf.getvalue(), None
    except ZownError as e:
        return buf.getvalue(), e


def test_two_peers_exchange_frames_on_loopback():
    import socket
    import threading
    mesh = tempfile.mkdtemp()
    probe = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    probe.bind(("::1", 0))
    port = str(probe.getsockname()[1])
    probe.close()
    server = """`r [ `t [
  $echo$ $echo$ 0 ag n ~l :s
  s ~r .
  $pong$ ~s
] gr ] gr
"""
    client = """`t [ `r [
  $echo$ $echo$ ~c :s
  s $ping$ ~s
  ~r .
] gr ] gr
"""
    got = {}
    def run(name, src, args):
        text, err = _stream_vm(src, args, mesh)
        got[name] = (text, None if err is None else f"{err.code} {err.op}")
    worker = threading.Thread(target=run, args=("srv", server, [port]))
    worker.start()
    run("cli", client, [])
    worker.join(4)
    assert got["srv"] == ("ping\n", None), got
    assert got["cli"] == ("pong\n", None), got


def test_relay_forwards_one_frame_each_way():
    import socket
    import threading
    import time
    mesh = tempfile.mkdtemp()
    probe = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
    probe.bind(("::1", 0))
    port = str(probe.getsockname()[1])
    probe.close()
    relay = "`r [ `t [ 0 ag n ~w ] gr ] gr\n"
    left = """`t [ `r [
  $::1$ 0 ag n ~o :s
  s $ping$ ~s
  ~r .
] gr ] gr
"""
    right = """`t [ `r [
  $::1$ 0 ag n ~o :s
  s ~r .
  $pong$ ~s
] gr ] gr
"""
    got = {}
    def run(name, src):
        text, err = _stream_vm(src, [port], mesh)
        got[name] = (text, None if err is None else f"{err.code} {err.op} {err.msg}")
    threads = [
        threading.Thread(target=run, args=("rel", relay)),
        threading.Thread(target=run, args=("a", left)),
        threading.Thread(target=run, args=("b", right)),
    ]
    threads[0].start()
    time.sleep(0.05)
    threads[1].start()
    time.sleep(0.05)
    threads[2].start()
    for thread in threads:
        thread.join(4)
    assert got["a"] == ("pong\n", None), got
    assert got["b"] == ("ping\n", None), got
    assert got["rel"][1] is None, got


def test_mapped_snapshot_reopens_in_a_fresh_vm():
    folder = tempfile.mkdtemp()
    path = os.path.join(folder, "g.txt")
    write = f"""$a$ gv
$b$ gv
$a$ $knows$ $b$ ge
${folder}$ [
  ${path}$ mm
] gd
gs .
"""
    first, vm = run(write)
    read = f"""${folder}$ [
  ${path}$ ml
] gd
gs .
$a$ » $knows$ → « .
"""
    second, _ = run(read)
    assert first == vm.graph.snapshot() + "\n"
    assert second == first + "b\n"


def test_merge_keeps_both_parents_either_order():
    src = open(os.path.join("conformance", "cases_v2", "merge.zn"), encoding="utf-8").read()
    want = open(os.path.join("conformance", "cases_v2", "merge.out"), encoding="utf-8").read()
    got, _ = run(src)
    assert got == want
    halves = [block for block in got.split("\n\n") if block]
    assert len(halves) == 2 and halves[0] == halves[1]


# --- layout (M13) --------------------------------------------------------------
def test_layout_dump_and_low_contrast():
    out, _ = run("""255 255 255 co :fg
255 255 0 co :ye
ye 0 0 [ $No$ 4 fg tx ] cm ly
.
""")
    assert out == "cm 0 0 8 4\n  tx 0 0 8 4 low No\n"
    _fails("256 0 0 co", BOUNDS, "co")
    _fails("0 0 0 co :c c 0 0 [ 1 ] cm", TYPE_MISMATCH, "cm")


def test_draw_paints_the_box_and_the_text():
    from zown.cli import main
    from zown.ui import UiNode, raster
    src = """255 255 255 co :fg
30 30 30 co :bg
bg 4 2 [ $Hi$ 8 fg tx ] cm
"""
    _, vm = run(src)
    node = vm.stack[0]
    assert isinstance(node, UiNode)
    raw = raster(node)
    assert raw.startswith(b"P6\n20 12\n255\n")
    body = raw.split(b"\n", 3)[3]
    assert body[0:3] == bytes((30, 30, 30))
    # H's top-left bit is on. The next column of that row is a hole, so the
    # parent gray shows through the cell.
    assert body[(2 * 20 + 2) * 3:(2 * 20 + 2) * 3 + 3] == bytes((255, 255, 255))
    assert body[(2 * 20 + 4) * 3:(2 * 20 + 4) * 3 + 3] == bytes((30, 30, 30))
    # The stem of i is ink. The column to its left is the parent.
    assert body[(2 * 20 + 14) * 3:(2 * 20 + 14) * 3 + 3] == bytes((255, 255, 255))
    assert body[(2 * 20 + 10) * 3:(2 * 20 + 10) * 3 + 3] == bytes((30, 30, 30))
    d = tempfile.mkdtemp()
    path = os.path.join(d, "ui.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)

    class _Out:
        def __init__(self):
            self.buffer = io.BytesIO()

        def write(self, _s):
            return None

        def flush(self):
            return None

    out = _Out()
    old = sys.stdout
    sys.stdout = out
    try:
        code = main(["draw", path])
    finally:
        sys.stdout = old
    assert code == 0
    assert out.buffer.getvalue().startswith(b"P6\n20 12\n255\n")


def test_button_handler_animation_and_gpu_readback():
    from zown.buf import Buf
    from zown.ui import Color, UiNode, raster

    out, _ = run(open(os.path.join("conformance", "cases_v2", "click.zn"), encoding="utf-8").read())
    assert out == "hit\nhit\nhit\nhit\n"
    _fails("1 0 0 pk", TYPE_MISMATCH, "pk")

    out, _ = run("""255 255 255 co :fg
$Hi$ 4 fg tx =
ly .
2 an
ly .
$Hi$ 4 fg tx =
2 an
ly .
ly .
""")
    assert out == (
        "tx 0 0 8 4 ok Hi\n"
        "tx 0 0 10 4 ok Hi\n"
        "tx 0 0 10 4 ok Hi\n"
        "tx 0 0 10 4 ok Hi\n"
    )
    _fails("$A$ 1 0 0 0 co tx 1 _ an", BOUNDS, "an")
    _fails(f"$A$ 1 0 0 0 co tx {INT_MAX} an", OVERFLOW, "an")

    _, vm = run("2 2 [ 255 0 0 co ] gp")
    buf = vm.stack[0]
    assert isinstance(buf, Buf)
    picture = raster(UiNode("cm", "", Color(255, 0, 0), 0, 0, 2, 2))
    assert bytes(buf.alloc.data) == picture.split(b"\n", 3)[3]
    e = _fails("1 1 [ 1 ] gp", TYPE_MISMATCH, "gp")
    assert e.stack == []
    _fails("0 1 [ 0 0 0 co ] gp", BOUNDS, "gp")


# --- graph store (M14) ---------------------------------------------------------
def test_graph_query_snapshot_and_errors():
    from zown.blake3 import blake3_hex
    out, vm = run("""
$a$ gv
$b$ gv
$c$ gv
$a$ $knows$ $b$ ge
$b$ $knows$ $c$ ge
$a$ $knows$ $b$ ge
$a$ » $knows$ → « .
$a$ » $knows$ → $knows$ → « .
$a$ » $missing$ → « .
gh .
""")
    snap = "v a\nv b\nv c\ne a knows b\ne b knows c\n"
    head = blake3_hex(snap.encode("utf-8"))
    assert out == f"b\nc\n\n{head}\n"
    assert vm.graph.snapshot() == snap
    assert vm.graph.head() == head
    empty, _ = run("gh .")
    assert empty == "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262\n"
    _, kept = run("$a$ gv sb :bundle $b$ gv bundle so , gs")
    assert kept.stack[-1] == "v a\n"
    e = _fails("$a b$ gv", TYPE_MISMATCH, "gv")
    assert e.kind == "run"
    bad = _fails(
        "$zown-sync 1\\nblake3 0000000000000000000000000000000000000000000000000000000000000000\\n$ so",
        INTEGRITY_FAIL, "so",
    )
    assert bad.kind == "sec"
    _fails("$not a bundle$ so", TYPE_MISMATCH, "so")
    _fails("1 »", TYPE_MISMATCH, "»")


def test_keep_fsync_roundtrip_and_tamper():
    from zown.cli import main
    d = tempfile.mkdtemp()
    src_path = os.path.join(d, "g.zn")
    bundle = os.path.join(d, "g.sync")
    src = "$a$ gv\n$b$ gv\n$a$ $to$ $b$ ge\n$a$ » $to$ → « .\n"
    with open(src_path, "w", encoding="utf-8") as fh:
        fh.write(src)
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = io.StringIO(), io.StringIO()
    try:
        code = main(["keep", "export", src_path, "-o", bundle])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0
    raw = open(bundle, "rb").read()
    assert raw.startswith(b"zown-sync 1\n")
    text = raw.decode("utf-8")
    # A new VM, the same bytes, the same query.
    opened, fresh = run(
        f"${text.replace(chr(10), '\\n')}$ so ,\n$a$ » $to$ → « .\n"
    )
    assert opened == "b\n"
    out, err = io.StringIO(), io.StringIO()
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["keep", "verify", bundle])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0
    lines = out.getvalue().splitlines()
    assert lines[0] == "ok"
    assert lines[1] == fresh.graph.head()
    flipped = bytearray(raw)
    flipped[flipped.index(ord("a"))] = ord("c")
    with open(bundle, "wb") as fh:
        fh.write(flipped)
    err = io.StringIO()
    sys.stdout, sys.stderr = io.StringIO(), err
    try:
        code = main(["--zerr", "keep", "verify", bundle])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 1
    packet = json.loads(err.getvalue())
    assert packet["code"] == "INTEGRITY_FAIL"
    assert packet["kind"] == "sec"
    assert packet["op"] == "so"


# --- control plane (M15) -------------------------------------------------------
def test_intent_drift_meet_and_fault():
    out, vm = run("""
$hello$ h3 ch
$1$ vr
$req$ td
$fetch$ sp
$read$ sp
se
se
$nodes$ $2$ wn
$nodes$ $1$ ob
df .
$nodes$ $2$ ob
df .
$DIV_ZERO$ $guard$ rm
[ 1 0 / ] ty ,
lg .
""")
    lines = out.splitlines()
    assert lines[0] == "nodes 2 1"
    assert lines[1] == "ok"
    log = "\n".join(lines[2:])
    assert " open fetch" in log
    assert " open read" in log
    assert log.index(" open read") < log.index(" close read") < log.index(" close fetch")
    assert " drift nodes 2 1" in log
    assert " meet nodes" in log
    assert " fault DIV_ZERO / guard" in log
    assert vm.tele.drift() == "ok"
    _fails("se", TYPE_MISMATCH, "se")
    _fails("$zz$ ch", TYPE_MISMATCH, "ch")
    _fails("$a\\nb$ nt", TYPE_MISMATCH, "nt")
    # A want that is set twice keeps its place. The new value is what df sees.
    out, _ = run("$a$ $1$ wn $b$ $1$ wn $a$ $2$ wn $a$ $2$ ob $b$ $0$ ob df .")
    assert out == "b 1 0\n"


def test_watch_prints_the_log_after_a_fault():
    from zown.cli import main
    d = tempfile.mkdtemp()
    path = os.path.join(d, "w.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write("$hello$ h3 ch\n1 0 /\n")
    out, err = io.StringIO(), io.StringIO()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["watch", path])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 1
    assert "DIV_ZERO" in err.getvalue()
    assert out.getvalue().strip().endswith("fault DIV_ZERO / -")
    assert out.getvalue().split()[1] == (
        "ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f"
    )


# --- self-hosting frontend (M16) ----------------------------------------------
def test_zown_lexer_matches_the_host():
    import glob
    from zown.lexer import token_dump
    from zown.stage import quote, zown_dump
    lex_src = open(os.path.join("zown", "std", "lex.zn"), encoding="utf-8").read()
    buf = io.StringIO()
    vm = VM(out=buf)
    vm.run_src(quote(lex_src) + " :z z pr")
    assert buf.getvalue() == lex_src
    files = []
    for pat in ("conformance/cases/*.zn", "conformance/cases_v2/*.zn",
                "conformance/errors/*.zn", "conformance/errors_v2/*.zn",
                "conformance/check/*.zn", "examples/*.zn", "zown/std/*.zn"):
        files.extend(glob.glob(pat))
    saw_self = False
    for path in files:
        text = open(path, encoding="utf-8").read()
        try:
            host = token_dump(text)
        except ZownError:
            continue
        assert zown_dump(text) == host, path
        if path.endswith("lex.zn"):
            saw_self = True
    assert saw_self


# --- parser (M19) --------------------------------------------------------------
def test_zown_parser_matches_the_host():
    import glob
    from zown.cli import main
    from zown.parser import ast_dump
    from zown.stage import stage_parse, zown_ast
    assert ast_dump("[[1]]\n") == "blk\n  blk\n    int 1\n  end\nend\n"
    assert ast_dump("007 .\n") == "int 007\nop .\n"
    assert ast_dump("1.50 .\n") == "float 1.50\nop .\n"
    samples = [
        "",
        "# only a comment\n",
        "1 2 +\n",
        "007 .\n",
        "1.50 .\n",
        "$$ .\n",
        "$a\\n$ .\n",
        "$say \\$$ .\n",
        "$a\nb$ .\n",
        "[ 1 2 + ] @\n",
        "[]\n",
        "[[1]]\n",
        "5 :x 'x .\n",
        "`t .\n",
        "$a$ » $knows$ → « .\n",
    ]
    for src in samples:
        assert zown_ast(src) == ast_dump(src), repr(src)
    files = []
    for pat in ("conformance/cases/*.zn", "conformance/cases_v2/*.zn",
                "conformance/errors/*.zn", "conformance/errors_v2/*.zn",
                "conformance/check/*.zn", "examples/*.zn", "zown/std/*.zn"):
        files.extend(glob.glob(pat))
    saw_self = False
    for path in files:
        text = open(path, encoding="utf-8").read()
        try:
            host = ast_dump(text, path)
        except ZownError:
            continue
        assert zown_ast(text) == host, path
        if path.endswith("parse.zn"):
            saw_self = True
    assert saw_self
    out, err = io.StringIO(), io.StringIO()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["stage", "parse", os.path.join("zown", "std", "parse.zn")])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0, err.getvalue()
    assert out.getvalue() == ast_dump(
        open(os.path.join("zown", "std", "parse.zn"), encoding="utf-8").read(),
        os.path.join("zown", "std", "parse.zn"),
    )
    bad = os.path.join(tempfile.mkdtemp(), "bad.zn")
    with open(bad, "w", encoding="utf-8") as fh:
        fh.write("]\n")
    try:
        stage_parse(bad)
        raise AssertionError("a stray bracket must stay a host parse error")
    except ZownError as e:
        assert e.code == REPAIR_SYNTAX


# --- crypto (M21) --------------------------------------------------------------
def test_crypto_matches_the_rfcs():
    from zown.aead import open_box, seal
    from zown.edwards import public_key, sign, verify, x25519
    key = bytes.fromhex("808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f")
    nonce = bytes.fromhex("070000004041424344454647")
    aad = bytes.fromhex("50515253c0c1c2c3c4c5c6c7")
    plain = bytes.fromhex(
        "4c616469657320616e642047656e746c656d656e206f662074686520636c617373"
        "206f66202739393a204966204920636f756c64206f6666657220796f75206f6e6c79"
        "206f6e652074697020666f7220746865206675747572652c2073756e73637265656e"
        "20776f756c642062652069742e"
    )
    boxed = seal(key, nonce, plain, aad)
    assert boxed[-16:].hex() == "1ae10b594f09e26a7e902ecbd0600691"
    assert open_box(key, nonce, boxed, aad) == plain
    seed = bytes.fromhex("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
    pub = bytes.fromhex("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
    sig = bytes.fromhex(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155"
        "5fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b"
    )
    assert public_key(seed) == pub
    assert sign(seed, b"") == sig
    assert verify(pub, b"", sig)
    assert not verify(pub, b"", bytes([sig[0] ^ 1]) + sig[1:])
    scalar = bytes.fromhex("77076d0a7318a57d3c16c17251b26645df4c2f87ebc0992ab177fba51db92c2a")
    point = bytes.fromhex("de9edb7d7b7dc1b4d35b61c2ece435373f8343c85b78674dadfc7e146f882b4f")
    shared = bytes.fromhex("4a5d9d5ba4ce2de1728e3bf480350f25e07e21c947d19e3376f09b3c1e161742")
    assert x25519(scalar, point) == shared
    _fails("0 bf 0 bf 0 bf 0 bf ae", TYPE_MISMATCH, "ae")
    _fails("0 bf 0 bf 0 bf ev", TYPE_MISMATCH, "ev")


# --- host I/O (M20) ------------------------------------------------------------
def test_host_io_grant_clock_and_args():
    import stat
    d = tempfile.mkdtemp()
    box = os.path.join(d, "box")
    os.mkdir(box)
    open(os.path.join(box, "a.txt"), "wb").write(b"Z")
    os.symlink("/etc/hostname", os.path.join(box, "out"))
    other = os.path.join(d, "box2")
    os.mkdir(other)
    open(os.path.join(other, "a.txt"), "wb").write(b"no")
    # A borrow can be the bytes fw writes. The owner stays exclusive.
    out, _ = run(f"""
${box}$ [
  1 bf
  0 90 bp
  = ${box}/c.txt$ fw
  ,
  0 bg .
] gd
""")
    assert out == "90\n"
    assert open(os.path.join(box, "c.txt"), "rb").read() == b"Z"
    _fails(f"${box}/a.txt$ fr", CAP_DENIED, "fr")
    _fails(f"${box}$ [ ${box}/out$ fr ] gd", CAP_DENIED, "fr")
    _fails(f"${box}$ [ ${other}/a.txt$ fr ] gd", CAP_DENIED, "fr")
    _fails(f"${box}$ [ ${box}/../box2/a.txt$ fr ] gd", CAP_DENIED, "fr")
    _fails(f"${box}$ [ ${box}/missing$ fr ] gd", BOUNDS, "fr")
    _fails(f"${box}$ [ ${box}$ fr ] gd", TYPE_MISMATCH, "fr")
    _fails("tm", CAP_DENIED, "tm")
    _fails("$PATH$ en", CAP_DENIED, "en")
    _fails("`v [ $ZOWN_NOT_SET_zz$ en ] gr", BOUNDS, "en")
    _fails("0 ag", BOUNDS, "ag")
    buf = io.StringIO()
    vm = VM(out=buf, args=["one", "two"])
    vm.run_src("ac . 0 ag . 1 ag .")
    assert buf.getvalue() == "2\none\ntwo\n"
    old_tick = os.environ.get("ZOWN_TICK")
    old_wall = os.environ.get("ZOWN_WALL")
    os.environ["ZOWN_TICK"] = "7"
    os.environ["ZOWN_WALL"] = "9"
    try:
        out, _ = run("`c [ tm . . ] gr")
    finally:
        if old_tick is None:
            os.environ.pop("ZOWN_TICK", None)
        else:
            os.environ["ZOWN_TICK"] = old_tick
        if old_wall is None:
            os.environ.pop("ZOWN_WALL", None)
        else:
            os.environ["ZOWN_WALL"] = old_wall
    assert out == "9\n7\n"
    # A file past the buffer budget is BOUNDS, not a partial buf.
    huge = os.path.join(box, "huge")
    with open(huge, "wb") as fh:
        fh.write(b"x" * (1048576 + 1))
    _fails(f"${box}$ [ ${huge}$ fr ] gd", BOUNDS, "fr")
    assert stat.S_ISREG(os.stat(huge).st_mode)


# --- buffers (M18) -------------------------------------------------------------
def test_buf_owner_borrow_and_free():
    out, _ = run("""
4 bf
= ln .
0 65 bp
0 bg .
= :r
1 66 bp
r 1 bg .
""")
    assert out == "4\n65\n66\n"
    _fails("4 bf = 0 1 bp", TYPE_MISMATCH, "bp")
    _fails("4 bf 4 bg", BOUNDS, "bg")
    _fails("4 bf = :r , r 0 bg", BOUNDS, "bg")
    _fails("1 _ bf", TYPE_MISMATCH, "bf")
    _fails("1048577 bf", BOUNDS, "bf")
    out, _ = run("4 bf :e 'e 0 7 bp 0 bg .\n")
    assert out == "7\n"


# --- signed bundle and transparency log (M24) ---------------------------------
def test_signed_bundle_matches_its_golden():
    for name in ("signed", "log"):
        src = open(os.path.join("conformance", "cases_v2", name + ".zn"), encoding="utf-8").read()
        want = open(os.path.join("conformance", "cases_v2", name + ".out"), encoding="utf-8").read()
        got, _vm = run(src)
        assert got == want, name


def test_bad_signature_leaves_the_graph():
    buf = io.StringIO()
    vm = VM(file="<signed>", out=buf)
    try:
        vm.run_src(open(os.path.join("conformance", "errors_v2", "signed_bad.zn"), encoding="utf-8").read())
        assert False, "a flipped signature opened"
    except ZownError as e:
        assert e.code == SIG_INVALID and e.kind == "sec" and e.op == "so"
    assert vm.graph.snapshot() == "v a\n"


def test_manifest_sig_is_a_real_signature():
    from zown.blake3 import blake3_hex
    from zown.edwards import public_key, verify
    from zown.manifest import generate
    src = "$a$ gv\n"
    seed = bytes(range(32))
    d = tempfile.mkdtemp()
    path = os.path.join(d, "app.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)
    os.environ["ZOWN_SIGN"] = seed.hex()
    try:
        signed = generate(path)
    finally:
        os.environ.pop("ZOWN_SIGN", None)
    assert signed["module"]["content"] == "zown:" + blake3_hex(src.encode())
    raw = bytes.fromhex(signed["module"]["sig"].split(":", 1)[1])
    assert verify(public_key(seed), src.encode(), raw)
    os.environ["ZOWN_SIGN"] = "nope"
    try:
        try:
            generate(path)
            assert False, "a bad ZOWN_SIGN was accepted"
        except ZownError as e:
            assert e.code == TYPE_MISMATCH and e.op == "manifest"
    finally:
        os.environ.pop("ZOWN_SIGN", None)


# --- stage-2 compiler (M23) ----------------------------------------------------
def test_zown_lower_matches_block_shape():
    from zown.cli import main
    from zown.stage import zown_ir
    hello = open(os.path.join("examples", "hello.zn"), encoding="utf-8").read()
    ir = zown_ir(hello, os.path.join("examples", "hello.zn"))
    assert ir == (
        "; zown ir  (2 blocks)\n"
        "main = b1\n"
        "\n"
        "b0:\n"
        "  push.s   \"Hello, World!\"\n"
        "  op       .\n"
        "\n"
        "b1:\n"
        "  push.blk b0\n"
        "  bind     h\n"
        "  load     h\n"
        "  invoke\n"
        "\n"
    )
    bare = zown_ir("$Zown$ .\n")
    assert "push.s   \"Zown\"" in bare
    assert bare.startswith("; zown ir  (1 blocks)\nmain = b0\n")
    own = open(os.path.join("zown", "std", "lower.zn"), encoding="utf-8").read()
    fixed = zown_ir(own, os.path.join("zown", "std", "lower.zn"))
    assert fixed.startswith("; zown ir  (")
    assert "\nmain = b" in fixed
    assert 'push.s   "lw"' in fixed
    out, err = io.StringIO(), io.StringIO()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["stage", "lower", os.path.join("examples", "hello.zn")])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0, err.getvalue()
    assert out.getvalue() == ir


def test_zown_emit_boot_sector():
    from zown.stage import stage_emit, zown_image
    img = zown_image(open(os.path.join("examples", "bare.zn"), encoding="utf-8").read())
    assert len(img) == 512
    assert img[-2:] == b"\x55\xaa"
    assert img[89:95] == b"Zown\n\x00"
    row = zown_image("$A$ .\n$B$ pr\n")
    assert row[89:93] == b"A\nB\x00"
    same = stage_emit(os.path.join("examples", "bare.zn"))
    assert same == img


# --- tooling (M30) -------------------------------------------------------------
def _frames(blob: bytes) -> list:
    msgs = []
    i = 0
    while i < len(blob):
        end = blob.find(b"\r\n\r\n", i)
        assert end >= 0, blob[i:]
        header = blob[i:end].decode("ascii")
        n = int(header.split(":", 1)[1].strip())
        body = blob[end + 4:end + 4 + n]
        msgs.append(json.loads(body.decode("utf-8")))
        i = end + 4 + n
    return msgs


def _rpc(obj: dict) -> bytes:
    data = json.dumps(obj).encode("utf-8")
    return f"Content-Length: {len(data)}\r\n\r\n".encode("ascii") + data


def _check_body(path: str) -> tuple[int, str, str]:
    """stderr of `zown check` split into findings and the summary line."""
    import subprocess
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    proc = subprocess.run(
        [sys.executable, os.path.join(root, "bin", "zown"), "check", path],
        capture_output=True, text=True,
    )
    err = proc.stderr
    if err.endswith("\n"):
        err = err[:-1]
    lines = err.split("\n") if err else []
    summary = lines[-1] if lines else ""
    body = "\n".join(lines[:-1])
    return proc.returncode, body, summary


def test_lsp_diagnostics_match_zown_check():
    from zown.lsp import diagnostics
    d = tempfile.mkdtemp()
    path = os.path.join(d, "bad.zn")
    src = "zz\n"
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)
    code, body, summary = _check_body(path)
    assert code == 2
    assert summary == f"1 issue(s) in {path}"
    items = diagnostics(src, path)
    assert "\n".join(item["message"] for item in items) == body
    assert items[0]["code"] == NAME_UNRESOLVED
    assert items[0]["severity"] == 1
    clean = os.path.join(d, "ok.zn")
    good = "1 :n\nn .\n"
    with open(clean, "w", encoding="utf-8") as fh:
        fh.write(good)
    code, body, summary = _check_body(clean)
    assert code == 0 and body == ""
    assert summary == f"ok: {clean} — no issues"
    assert diagnostics(good, clean) == []


def test_lsp_definition_and_format():
    from zown.fmt import format_source
    from zown.lsp import definition, formatting
    src = "1 :n\nn .\n"
    found = definition(src, 1, 0)
    assert found == {
        "start": {"line": 0, "character": 2},
        "end": {"line": 0, "character": 4},
    }
    assert src[2:4] == ":n"
    assert definition(src, 0, 2) == found
    shadowed = "1 :n\n2 :n\nn .\n"
    later = definition(shadowed, 2, 0)
    assert later["start"] == {"line": 1, "character": 2}
    early = "n .\n1 :n\n"
    assert definition(early, 0, 0)["start"] == {"line": 1, "character": 2}
    assert definition("1 :n\nln .\n", 1, 0) is None
    messy = "1  +  2"
    edits = formatting(messy)
    assert len(edits) == 1
    assert edits[0]["newText"] == format_source(messy) == "1 + 2\n"
    assert edits[0]["range"]["start"] == {"line": 0, "character": 0}
    assert edits[0]["range"]["end"] == {"line": 0, "character": len(messy)}


def test_lsp_stdio_roundtrip():
    from zown.lsp import Server, serve
    uri = "file:///t.zn"
    text = "1 :n\nn .\n"
    inp = io.BytesIO(b"".join([
        _rpc({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        _rpc({"jsonrpc": "2.0", "method": "initialized", "params": {}}),
        _rpc({"jsonrpc": "2.0", "method": "textDocument/didOpen", "params": {
            "textDocument": {"uri": uri, "text": text},
        }}),
        _rpc({"jsonrpc": "2.0", "id": 2, "method": "textDocument/diagnostic", "params": {
            "textDocument": {"uri": uri},
        }}),
        _rpc({"jsonrpc": "2.0", "id": 3, "method": "textDocument/definition", "params": {
            "textDocument": {"uri": uri},
            "position": {"line": 1, "character": 0},
        }}),
        _rpc({"jsonrpc": "2.0", "id": 4, "method": "textDocument/formatting", "params": {
            "textDocument": {"uri": uri},
        }}),
        _rpc({"jsonrpc": "2.0", "id": 5, "method": "shutdown"}),
        _rpc({"jsonrpc": "2.0", "method": "exit"}),
    ]))
    out = io.BytesIO()
    assert serve(inp, out) == 0
    msgs = _frames(out.getvalue())
    assert msgs[0]["id"] == 1
    caps = msgs[0]["result"]["capabilities"]
    assert caps["textDocumentSync"] == 1
    assert caps["definitionProvider"] is True
    assert caps["documentFormattingProvider"] is True
    assert "codeActionProvider" not in caps
    opened = next(m for m in msgs if m.get("method") == "textDocument/publishDiagnostics")
    assert opened["params"]["diagnostics"] == []
    pulled = next(m for m in msgs if m.get("id") == 2)
    assert pulled["result"] == {"kind": "full", "items": []}
    defined = next(m for m in msgs if m.get("id") == 3)
    assert defined["result"]["uri"] == uri
    assert defined["result"]["range"]["start"] == {"line": 0, "character": 2}
    formatted = next(m for m in msgs if m.get("id") == 4)
    assert formatted["result"][0]["newText"] == text
    assert next(m for m in msgs if m.get("id") == 5)["result"] is None
    assert serve(io.BytesIO(_rpc({"jsonrpc": "2.0", "method": "exit"})), io.BytesIO()) == 1
    assert serve(io.BytesIO(b""), io.BytesIO()) == 1
    broken = Server()
    broken.docs["u"] = "$nope"
    reply = broken.handle({
        "jsonrpc": "2.0", "id": 9, "method": "textDocument/formatting",
        "params": {"textDocument": {"uri": "u"}},
    })
    assert reply[0]["error"]["message"].startswith("zerr[")


def test_lsp_did_change_republishes():
    from zown.lsp import Server
    server = Server()
    uri = "file:///t.zn"
    opened = server.handle({
        "method": "textDocument/didOpen",
        "params": {"textDocument": {"uri": uri, "text": "1 .\n"}},
    })
    assert opened[0]["params"]["diagnostics"] == []
    changed = server.handle({
        "method": "textDocument/didChange",
        "params": {
            "textDocument": {"uri": uri},
            "contentChanges": [{"text": "zz\n"}],
        },
    })
    diags = changed[0]["params"]["diagnostics"]
    assert len(diags) == 1 and diags[0]["code"] == NAME_UNRESOLVED
    assert "zz" in diags[0]["message"]


def test_doc_prints_stored_fields_and_a_null_sig():
    from zown.blake3 import blake3_hex
    from zown.cli import main
    from zown.doc import render
    src = "[ 1 ] :n\n"
    d = tempfile.mkdtemp()
    path = os.path.join(d, "app.zn")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(src)
    os.environ.pop("ZOWN_SIGN", None)
    text = render(path)
    digest = "zown:" + blake3_hex(src.encode("utf-8"))
    assert f"content: {digest}" in text
    assert "sig: null" in text
    assert "ed25519:" not in text
    assert "symbol: n" in text
    assert not os.path.exists(path + ".json")
    out, err = io.StringIO(), io.StringIO()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["doc", path])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0, err.getvalue()
    assert out.getvalue() == text


def test_pkg_resolves_a_local_content_hash():
    from zown.blake3 import blake3_hex
    from zown.cli import main
    from zown.pkg import resolve
    d = tempfile.mkdtemp()
    os.makedirs(os.path.join(d, "a"))
    os.makedirs(os.path.join(d, "b"))
    blob = b"hello\n"
    for rel in ("a/hello.txt", "b/hello.txt"):
        with open(os.path.join(d, rel), "wb") as fh:
            fh.write(blob)
    digest = "zown:" + blake3_hex(blob)
    missing = "zown:" + ("0" * 64)
    pkg = os.path.join(d, "app.zpkg")
    with open(pkg, "w", encoding="utf-8") as fh:
        json.dump({"name": "app", "deps": [digest]}, fh)
    assert resolve(pkg) == [(digest, "a/hello.txt")]
    out, err = io.StringIO(), io.StringIO()
    old_out, old_err = sys.stdout, sys.stderr
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["pkg", pkg])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 0, err.getvalue()
    assert out.getvalue() == f"{digest} a/hello.txt\n"
    with open(pkg, "w", encoding="utf-8") as fh:
        json.dump({"name": "app", "deps": [missing]}, fh)
    try:
        resolve(pkg)
        assert False, "a missing hash resolved"
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "pkg"
    out, err = io.StringIO(), io.StringIO()
    sys.stdout, sys.stderr = out, err
    try:
        code = main(["pkg", pkg])
    finally:
        sys.stdout, sys.stderr = old_out, old_err
    assert code == 1
    assert "BOUNDS" in err.getvalue()
    with open(pkg, "w", encoding="utf-8") as fh:
        fh.write("{")
    try:
        resolve(pkg)
        assert False, "broken JSON was accepted"
    except ZownError as e:
        assert e.code == TYPE_MISMATCH and e.op == "pkg"


def test_bare_boot_skips_only_when_qemu_is_absent():
    import conformance.bare_boot as boot
    buf = io.StringIO()
    old_which = boot.shutil.which
    old_out = sys.stdout
    boot.shutil.which = lambda _name: None
    sys.stdout = buf
    try:
        code = boot.main()
    finally:
        boot.shutil.which = old_which
        sys.stdout = old_out
    assert code == 0, buf.getvalue()
    assert "skip" in buf.getvalue()
    assert "qemu-system-i386 not found" in buf.getvalue()


def test_h3_hashes_buf_bytes():
    out, _vm = run("4 bf 0 1 bp 1 2 bp 2 3 bp 3 4 bp h3 .")
    from zown.blake3 import blake3_hex
    assert out.strip() == blake3_hex(bytes([1, 2, 3, 4]))
    out, _vm = run("$hello$ h3 .")
    assert out.strip() == blake3_hex(b"hello")


_STILL = bytes.fromhex(
    "12000a06180cffd80080325915000000500e38c8bc443dfd8e548a388c486185"
    "f1e6c8710f49084b592b9181ce0481b245e9d503ee5d9f28bbe12a2226f2532a"
    "78f1b0c62396b0cb04ce1c4098452f50183178a8e6fa4ae2c3fdd302e4b83fff"
    "395615b990"
)


def _buf_words(data: bytes) -> str:
    lines = [f"{len(data)} bf"]
    for i, b in enumerate(data):
        if b:
            lines.append(f"{i} {b} bp")
    return "\n".join(lines)


_OPUS = bytes.fromhex("907f144ef4125748bef98d0cddc8e97892c9d1a5")


def test_opus_packet_hashes_pcm():
    from zown.blake3 import blake3_hex
    from zown.opus import decode
    pcm = decode(_OPUS)
    assert len(pcm) == 960
    assert blake3_hex(pcm) == "7489f4f97e2179d2c4851627de498a6d5ae28b39f2280de15163a1d1c35c96e5"
    out, _vm = run(_buf_words(_OPUS) + "\nop h3 .")
    assert out.strip() == blake3_hex(pcm)
    try:
        run("0 bf op .")
        assert False, "empty Opus ran"
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "op"
    try:
        run("1 bf 0 0 bp op .")
        assert False, "other TOC ran"
    except ZownError as e:
        assert e.code == UNSUPPORTED and e.op == "op"
    try:
        run("1 bf 0 144 bp op .")
        assert False, "short Opus ran"
    except ZownError as e:
        assert e.code == INTEGRITY_FAIL and e.op == "op"


def test_av1_still_hashes_i420():
    from zown.av1 import decode
    from zown.blake3 import blake3_hex
    pic = decode(_STILL)
    assert len(pic) == 384
    assert blake3_hex(pic) == "5ee6fcf9d2da2062d3e72591a7ad2d1aa1623b3609d613025cf10edbb5d05462"
    out, _vm = run(_buf_words(_STILL) + "\nav h3 .")
    assert out.strip() == blake3_hex(pic)
    try:
        run("0 bf av .")
        assert False, "empty AV1 ran"
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "av"
    try:
        run(_buf_words(_STILL[:-10]) + "\nav .")
        assert False, "truncated AV1 ran"
    except ZownError as e:
        assert e.code == INTEGRITY_FAIL and e.op == "av"


def test_av1_lossless_round_trip():
    from zown.av1 import decode
    from zown.blake3 import blake3_hex
    pic = decode(_STILL)
    out, _vm = run(_buf_words(pic) + "\nve av h3 .")
    assert out.strip() == blake3_hex(pic)
    try:
        run("0 bf ve .")
        assert False, "empty encode ran"
    except ZownError as e:
        assert e.code == BOUNDS and e.op == "ve"
    try:
        run("16 bf ve .")
        assert False, "short frame encoded"
    except ZownError as e:
        assert e.code == UNSUPPORTED and e.op == "ve"


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
