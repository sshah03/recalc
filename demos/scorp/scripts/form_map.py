"""The 2026 draft Form 1120-S, line by line, read from the IRS's own PDF.

    form_map.py irs/f1120s--dft.pdf   ->  JSON rows: line, label, field

Each fillable field's tooltip names its line ("12. Taxes and licenses."); page 1's
lines keep their numbers, Schedule K's get a K in front (K16d). Line 1a's
tooltip is the page's caution note, so it's named here.
"""
import json
import re
import sys

try:
    from pypdf import PdfReader
except ImportError:
    # Before Setup has installed pypdf: no rows yet, and say why.
    print("[]")
    print("pypdf isn't installed yet; run Setup on the Clients sheet", file=sys.stderr)
    sys.exit(0)

reader = PdfReader(sys.argv[1])
rows = []
for page_no, page in enumerate(reader.pages):
    for annot in page.get("/Annots") or []:
        a = annot.get_object()
        field, tip = str(a.get("/T", "")), str(a.get("/TU", ""))
        if not field.startswith(("f1_", "f3_", "f4_0")):
            continue
        if field == "f1_17[0]":
            line, label = "1a", "Gross receipts or sales"
        else:
            # The line is the first "12a." after any section heading.
            m = re.search(r"(?:^|\. )(\d{1,2}[a-z]?)\. (.+?)(?:\.\s*$|\. \()", tip)
            if not m:
                continue
            line, label = m.group(1), m.group(2).strip()
        if field.startswith("f1_") and not re.fullmatch(r"\d{1,2}[a-z]?", line):
            continue
        if field.startswith(("f3_", "f4_")):
            line = "K" + line
        if any(r["line"] == line for r in rows):
            continue
        rows.append({"line": line, "label": label[:80], "field": field.split("[")[0]})
print(json.dumps(rows))
