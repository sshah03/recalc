import csv
import io

from .money import fmt


def csv_text(lines):
    """lines: [(description, amount)]. CSV with a header row."""
    buf = io.StringIO()
    w = csv.writer(buf, lineterminator="\n")
    w.writerow(["description", "amount"])
    for desc, amount in lines:
        w.writerow([desc, fmt(amount)])
    return buf.getvalue()
