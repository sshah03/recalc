"""Write a client's checked figures into the IRS's 2026 draft Form 1120-S.

    fill_1120s.py "Client name" 1a=1284600 2=611250 ... K16d=100000

Fields come from the PDF's own tooltips (form_map.py), so a line goes where the
draft puts it. The output keeps the IRS's DRAFT - DO NOT FILE marks.
"""
import json
import os
import subprocess
import sys

try:
    from pypdf import PdfReader, PdfWriter
except ImportError:
    sys.exit("pypdf isn't installed yet; run Setup on the Clients sheet")

here = os.path.dirname(os.path.abspath(__file__))
pdf = os.path.join(here, "..", "irs", "f1120s--dft.pdf")
client, pairs = sys.argv[1], sys.argv[2:]
fields = {r["line"]: r["field"] for r in json.loads(
    subprocess.run([sys.executable, os.path.join(here, "form_map.py"), pdf], capture_output=True, text=True, check=True).stdout)}
values = {"f1_04": client}
for pair in pairs:
    line, _, amount = pair.partition("=")
    if line not in fields:
        sys.exit(f"the draft form has no line {line}")
    n = round(float(amount or 0))
    values[fields[line]] = f"{n:,}" if n else ""
writer = PdfWriter(clone_from=PdfReader(pdf))
for page in writer.pages:
    names = {str(a.get_object().get("/T", "")).split("[")[0] for a in page.get("/Annots") or []}
    mine = {f"{k}[0]": v for k, v in values.items() if k in names}
    if mine:
        writer.update_page_form_field_values(page, mine)
writer.set_need_appearances_writer(True)
# Drop the IRS's cover note: the form starts on its second page.
del writer.pages[0]
os.makedirs("returns", exist_ok=True)
out = os.path.join("returns", f"{client} - 2026 Form 1120-S (DRAFT, do not file).pdf")
with open(out, "wb") as f:
    writer.write(f)
print(out)
