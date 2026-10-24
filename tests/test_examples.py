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
