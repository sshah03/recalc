#!/usr/bin/env python3
"""A small, dependency-free benchmark suite. Prints milliseconds.

    python3 bench.py            # total across all cases
    python3 bench.py sort       # one case: sort | regex | json | strings

Each case does the same fixed amount of work, so a change in the number is
a change in the machine or in this file — which is what the board watches.
"""
import json
import random
import re
import sys
import time

N = 60_000


def case_sort():
    rng = random.Random(7)
    data = [rng.random() for _ in range(N * 4)]
    data.sort()
    return data[0]


def case_regex():
    text = " ".join(f"user{i}@example.com order-{i * 7} 2025-0{1 + i % 9}-1{i % 10}" for i in range(N // 6))
    return len(re.findall(r"[a-z]+\d+@example\.com|order-\d+|\d{4}-\d{2}-\d{2}", text))


def case_json():
    payload = [{"id": i, "tags": ["a", "b", str(i % 13)], "ok": i % 3 == 0} for i in range(N // 2)]
    return len(json.loads(json.dumps(payload)))


def case_strings():
    parts = [f"{i:08x}".upper().replace("0", "o") for i in range(N * 2)]
    return len("".join(parts))


CASES = {"sort": case_sort, "regex": case_regex, "json": case_json, "strings": case_strings}


def run(name):
    fn = CASES[name]
    best = None
    for _ in range(3):
        t0 = time.perf_counter()
        fn()
        ms = (time.perf_counter() - t0) * 1000
        best = ms if best is None else min(best, ms)
    return best


def main(argv):
    names = argv[1:] or list(CASES)
    for name in names:
        if name not in CASES:
            sys.exit(f"unknown case {name!r}; cases: {', '.join(CASES)}")
    total = sum(run(n) for n in names)
    print(f"{total:.1f}")


if __name__ == "__main__":
    main(sys.argv)
