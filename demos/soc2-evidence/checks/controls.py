"""Re-perform a control's test over the exports, so the counts an auditor
sees come from a command anyone can run again rather than from reading.

    python3 checks/controls.py population CC8.1-PR
    python3 checks/controls.py exceptions CC8.1-PR
    python3 checks/controls.py details    CC8.1-PR
    python3 checks/controls.py latest     CC8.1-PR
    python3 checks/controls.py owner      CC8.1-PR

The audit period is 2026-01-01 to 2026-06-30, by the UTC date of each
record. Exceptions are counted after the policies' own allowances (a
Standard change, a cancelled ticket, access removed before the termination
date, a retest within 7 days, a risk acceptance approved before the due
date); `details` names the exceptions and any allowance that was applied.
A control with nothing in evidence/ prints Not found.
"""
import csv
import datetime as dt
import glob
import json
import re
import sys

START, END = dt.date(2026, 1, 1), dt.date(2026, 6, 30)
QUARTERS = [("2026-Q1", dt.date(2026, 3, 31)), ("2026-Q2", dt.date(2026, 6, 30))]


def day(s):
    return dt.date.fromisoformat(s[:10]) if s else None


def in_period(d):
    return d is not None and START <= d <= END


def rows(path):
    with open(path, newline="") as f:
        return list(csv.DictReader(f))


def business_days(a, b):
    """Weekdays after a, up to and including b."""
    n, d = 0, a
    while d < b:
        d += dt.timedelta(days=1)
        n += d.weekday() < 5
    return n


def pr():
    prs = [p for p in json.load(open("evidence/github-merged-prs.json"))["pull_requests"] if in_period(day(p["merged_at"]))]
    bad = [p for p in prs if p["admin_bypass"] or not [a for a in p["approvers"] if a != p["author"]]]
    details = [f"PR {p['number']} '{p['title']}' merged {p['merged_at'][:10]} by {p['merged_by']}" + (" with an admin bypass" if p["admin_bypass"] else "") + (" and no independent approval" if not [a for a in p["approvers"] if a != p["author"]] else "") for p in bad]
    return len(prs), details, [], max(day(p["merged_at"]) for p in prs)


def ct():
    tickets = rows("evidence/change-tickets.csv")
    pop = [t for t in tickets if t["status"] != "Cancelled" and in_period(day(t["implemented_at"]))]
    allowed = [f"{t['ticket']} is a Standard change ({t['standard_change_id']})" for t in pop if t["type"] == "Standard" and t["standard_change_id"]]
    bad = [t for t in pop if t["type"] != "Standard" and not (t["approved_at"] and t["approved_at"] < t["implemented_at"])]
    return len(pop), [f"{t['ticket']} implemented {t['implemented_at'][:10]} without prior approval" for t in bad], allowed, max(day(t["implemented_at"]) for t in pop)


def td():
    terms = [t for t in rows("evidence/hris-terminations.csv") if in_period(day(t["termination_date"]))]
    users = {u["employee_id"]: u for u in rows("evidence/okta-users.csv")}
    bad, allowed, dates = [], [], []
    for t in terms:
        term = day(t["termination_date"])
        off = day(users.get(t["employee_id"], {}).get("deactivated_at"))
        if off is None:
            bad.append(f"{t['employee_id']} {t['name']} terminated {term}, still active")
            continue
        dates.append(off)
        if off < term:
            allowed.append(f"{t['employee_id']} {t['name']} deactivated {off}, before the {term} termination")
        elif business_days(term, off) > 1:
            bad.append(f"{t['employee_id']} {t['name']} terminated {term}, deactivated {off} ({business_days(term, off)} business days)")
    return len(terms), bad, allowed, max(dates)


def ar():
    bad, dates = [], []
    for q, end in QUARTERS:
        files = glob.glob(f"evidence/access-reviews/{q}.md")
        if not files:
            bad.append(f"{q} review missing")
            continue
        signed = day(re.search(r"Date signed: (\S+)", open(files[0]).read()).group(1))
        dates.append(signed)
        late = (signed - end).days
        if late > 30:
            bad.append(f"{q} signed {signed}, {late} days after quarter end (30 allowed)")
    return len(QUARTERS), bad, [], max(dates)


def mfa():
    active = [u for u in rows("evidence/okta-users.csv") if u["status"] == "ACTIVE"]
    bad = [f"{u['user_id']} {u['name']} has no MFA" for u in active if u["mfa_enrolled"] != "true"]
    return len(active), bad, [], max(day(u["created"]) for u in active)


def sat():
    hires = [h for h in rows("evidence/hris-hires.csv") if in_period(day(h["hire_date"]))]
    done = {c["employee_id"]: day(c["completed_on"]) for c in rows("evidence/training-completions.csv") if c["course"] == "Security awareness: new starter"}
    bad, dates = [], []
    for h in hires:
        hired, finished = day(h["hire_date"]), done.get(h["employee_id"])
        if finished is None:
            bad.append(f"{h['employee_id']} {h['name']} has no new-starter training")
            continue
        dates.append(finished)
        if (finished - hired).days > 30:
            bad.append(f"{h['employee_id']} {h['name']} completed on day {(finished - hired).days}")
    return len(hires), bad, [], max(dates)


SLA = {"critical": 15, "high": 30}


def vm():
    latest = {}
    for path in sorted(glob.glob("evidence/vuln-scans/*.json")):
        for f in json.load(open(path))["findings"]:
            if f["severity"] in SLA:
                latest[f["id"]] = f  # later scans carry the latest status
    found = [f for f in latest.values() if in_period(day(f["first_seen"])) or in_period(day(f["remediated_on"])) or (day(f["first_seen"]) < START and not f["remediated_on"])]
    risks = {r["related_finding"]: r for r in rows("evidence/risk-register.csv") if r["type"] == "Risk acceptance" and r["related_finding"]}
    bad, allowed = [], []
    for f in found:
        seen, fixed = day(f["first_seen"]), day(f["remediated_on"])
        due = seen + dt.timedelta(days=SLA[f["severity"]])
        if (fixed or END) <= due:
            continue
        r = risks.get(f["id"])
        if r and r["approved_on"] and day(r["approved_on"]) <= due and fixed and fixed <= day(r["expires_on"]):
            allowed.append(f"{f['id']} past its {due} due date, covered by {r['risk_id']} (approved {r['approved_on']}, expires {r['expires_on']})")
        else:
            bad.append(f"{f['id']} {f['severity']}, due {due}, " + (f"remediated {fixed}" if fixed else "still open"))
    return len(found), bad, allowed, max(day(f["remediated_on"] or f["first_seen"]) for f in found)


def br():
    tests = []
    for line in open("evidence/backup-restore-tests.log"):
        m = re.match(r"(\S+) RESTORE_TEST .*result=(PASS|FAIL)", line)
        if m:
            tests.append((day(m.group(1)), m.group(2)))
    bad, allowed, starts = [], [], [dt.date(2026, 1, 1), dt.date(2026, 4, 1)]
    for (q, end), start in zip(QUARTERS, starts):
        here = [t for t in tests if start <= t[0] <= end]
        if not any(r == "PASS" for _, r in here):
            bad.append(f"{q} has no passing restore test")
        for d, r in here:
            if r == "FAIL":
                retest = [p for p, pr_ in tests if pr_ == "PASS" and d < p <= d + dt.timedelta(days=7)]
                (allowed if retest else bad).append(f"{q}: restore test failed {d}, " + (f"retested and passed {retest[0]}" if retest else "no passing retest within 7 days"))
    return len(QUARTERS), bad, allowed, max(d for d, _ in tests)


def ir():
    held = []
    for path in glob.glob("evidence/ir-tabletop-*.md"):
        m = re.search(r"Date held: (\d{4}-\d{2}-\d{2})", open(path).read())
        if m and in_period(day(m.group(1))):
            held.append(day(m.group(1)))
    return 1, ([] if held else ["no tabletop exercise held in the period"]), [], max(held) if held else None


TESTS = {"CC8.1-PR": pr, "CC8.1-CT": ct, "CC6.3-TD": td, "CC6.2-AR": ar, "CC6.1-MFA": mfa, "CC1.4-SAT": sat, "CC7.1-VM": vm, "A1.2-BR": br, "CC7.4-IR": ir}

what, control = sys.argv[1], sys.argv[2]
if what == "owner":
    print(next(c["owner"] for c in rows("documents/controls.csv") if c["control_id"] == control))
    sys.exit()
if control not in TESTS:
    # CC9.2-VR: nothing under evidence/ shows a vendor review took place.
    print("Not found" if what != "details" else "No evidence of this control under evidence/")
    sys.exit()
population, exceptions, allowances, latest = TESTS[control]()
if what == "population":
    print(population)
elif what == "exceptions":
    print(len(exceptions))
elif what == "latest":
    print(latest or "Not found")
elif what == "details":
    lines = exceptions + [f"allowed: {a}" for a in allowances]
    print("; ".join(lines) if lines else "none")
