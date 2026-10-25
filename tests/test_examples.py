"""End-to-end check that the shipped example programs produce expected output."""

import io
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from zown.vm import VM

_EX = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "examples")

EXPECTED = {
    "hello.zn": "Hello, World!\n",
    "fib.zn": "0\n1\n1\n2\n3\n5\n8\n13\n21\n34\n",
    "fizzbuzz.zn": (
        "1\n2\nFizz\n4\nBuzz\nFizz\n7\n8\nFizz\nBuzz\n11\nFizz\n13\n14\nFizzBuzz\n"
    ),
    "lanes.zn": "mncmnmn\n7\n9\n",
    "bare.zn": "Zown\n",
    "buf.zn": "4\n65\n66\n",
    "ui.zn": "cm 0 0 20 24\n  tx 2 2 16 8 ok Hi\n  bt 2 14 16 8 ok Go\n",
    "graph.zn": (
        "b\nc\n\n"
        "cb8d08bdb86a701fd4e9f1d104d1748e82d14c78702aab2b87c407da8061de82\n"
    ),
    "intent.zn": (
        "nodes 2 1\nok\n"
        "1 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req open fetch\n"
        "2 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req close fetch\n"
        "3 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req want nodes 2\n"
        "4 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req obs nodes 1\n"
        "5 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req drift nodes 2 1\n"
        "6 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req obs nodes 2\n"
        "7 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req meet nodes\n"
        "8 ea8f163db38682925e4491c5e58d4bb3506ef8c14eb78a86e908c5624a67200f 1 req fault DIV_ZERO / guard\n"
    ),
}


def _run_file(path):
    with open(path, "r", encoding="utf-8") as fh:
        src = fh.read()
    buf = io.StringIO()
    VM(file=path, out=buf).run_src(src)
    return buf.getvalue()


def test_examples_match_expected():
    for name, want in EXPECTED.items():
        got = _run_file(os.path.join(_EX, name))
        assert got == want, f"{name}: {got!r} != {want!r}"


if __name__ == "__main__":
    test_examples_match_expected()
    print("examples ok")
