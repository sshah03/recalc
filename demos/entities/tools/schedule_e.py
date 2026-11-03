#!/usr/bin/env python3
"""What the owner's 1040 reports for one row of the entity grid.

    schedule_e.py <owner's 1040 summary> "<row's files, comma-separated>"

For an entity's row, finds the entity's EIN in the row's files and prints the
amount the 1040's Schedule E reports on the line with that EIN, or "not on
the 1040" if no line carries it. For the owner's own row (the 1040 itself is
in the list), prints the Schedule E pass-through total: line 32 plus line 37.
Amounts print as plain numbers. It only reads. Standard library only.
"""
import os
import re
import sys

EIN = re.compile(r"XX-XXX\d{4}")
AMOUNT = re.compile(r"-?\d[\d,]*\.\d\d")


def last_amount(line):
    found = AMOUNT.findall(line)
    return float(found[-1].replace(",", "")) if found else None


def show(n):
    print(int(n) if n == int(n) else f"{n:.2f}")


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    ret = argv[1]
    files = [p.strip() for p in argv[2].split(",") if p.strip()]
    with open(ret, encoding="utf-8") as f:
        lines = f.read().splitlines()
    if any(os.path.exists(p) and os.path.samefile(p, ret) for p in files):
        total = 0.0
        for line in lines:
            if re.match(r"\s*Line (32|37)\b", line):
                total += last_amount(line) or 0.0
        show(total)
        return
    eins = set()
    for p in files:
        with open(p, encoding="utf-8", errors="replace") as f:
            eins.update(EIN.findall(f.read()))
    for line in lines:
        if any(e in line for e in eins):
            amount = last_amount(line)
            if amount is not None:
                show(amount)
                return
    print("not on the 1040")


if __name__ == "__main__":
    main(sys.argv)
