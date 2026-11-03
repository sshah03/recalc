# shipit

A small release helper: tags a change, picks its reviewer, retries a flaky
upload with backoff, and totals line items. Python 3.10+, standard library
only. CI fails about one run in three, and nobody can say which test is to
blame.

| Path | What it is |
|---|---|
| `shipit/tags.py` | renders a change's labels |
| `shipit/review.py` | picks a reviewer for a change |
| `shipit/backoff.py` | retries with a jittered delay |
| `shipit/totals.py` | totals line items |
| `tests/` | one test file per module |

```
python3 -m unittest discover -s tests -t .
```
