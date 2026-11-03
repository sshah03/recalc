#!/usr/bin/env python3
"""Which documents a client's profile expects that aren't in the folder.

    missing_docs.py clients/<slug>/      prints "none", or the missing ones joined by "; "
    missing_docs.py --count clients/     prints how many are missing across every client

Each line under "## Expected documents" in profile.md reads "- <form>: <from>".
A document counts as received when some other file in the folder mentions both
the form and who it's from (case and spacing ignored), so the check reads the
documents themselves, not their file names. It only reads; the board re-runs
it whenever a file under clients/ changes. Standard library only.
"""
import os
import re
import sys


def norm(text):
    return re.sub(r"\s+", " ", text).strip().lower()


def expected(folder):
    path = os.path.join(folder, "profile.md")
    with open(path, encoding="utf-8") as f:
        lines = f.read().splitlines()
    out, inside = [], False
    for line in lines:
        if line.startswith("## "):
            inside = line.strip().lower() == "## expected documents"
            continue
        if inside and line.startswith("- ") and ":" in line:
            form, source = line[2:].split(":", 1)
            out.append((form.strip(), source.strip()))
    return out


def missing(folder):
    texts = []
    for name in sorted(os.listdir(folder)):
        path = os.path.join(folder, name)
        if name == "profile.md" or not os.path.isfile(path):
            continue
        with open(path, encoding="utf-8", errors="replace") as f:
            texts.append(norm(f.read()))
    return [
        f"{form} from {source}"
        for form, source in expected(folder)
        if not any(norm(form) in t and norm(source) in t for t in texts)
    ]


def main(argv):
    if len(argv) == 3 and argv[1] == "--count":
        root = argv[2]
        total = sum(
            len(missing(os.path.join(root, d)))
            for d in sorted(os.listdir(root))
            if os.path.isfile(os.path.join(root, d, "profile.md"))
        )
        print(total)
    elif len(argv) == 2:
        gone = missing(argv[1])
        print("; ".join(gone) if gone else "none")
    else:
        sys.exit(__doc__)


if __name__ == "__main__":
    main(sys.argv)
