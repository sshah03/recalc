# tally

A small invoicing library: parse amounts, render invoices, export CSV for
the accounting system, and work out due dates. Python 3.10+, standard
library only.

| Path | What it is |
|---|---|
| `tally/money.py` | amounts: parsing and formatting, shared by everything below |
| `tally/invoice.py` | renders an invoice for a person to read |
| `tally/export.py` | CSV rows for the accounting system, which reads plain numbers |
| `tally/dates.py` | due dates |
| `tests/` | one acceptance test file per task; don't edit them |

```
python3 -m unittest discover -s tests -t .
```
