#!/usr/bin/env python3
"""Draft the missing-documents request letter for one client.

    draft_letter.py clients/<slug>/ "<missing documents>"

Writes letters/<slug>.md (a draft for the assignee to review and send; it
sends nothing) and prints its path. The second argument is the board's
missing-documents cell for the row; "none" drafts nothing. This writes a
file, so on the board it is a ! action a person runs. Standard library only.
"""
import datetime
import os
import re
import sys


def field(profile, label):
    m = re.search(rf"^- {label}: (.+)$", profile, re.M)
    return m.group(1).strip() if m else ""


def main(argv):
    if len(argv) != 3:
        sys.exit(__doc__)
    folder, missing = argv[1], argv[2].strip()
    slug = os.path.basename(os.path.normpath(folder))
    with open(os.path.join(folder, "profile.md"), encoding="utf-8") as f:
        profile = f.read()
    title = re.search(r"^# Client profile: (.+)$", profile, re.M)
    client = title.group(1).strip() if title else slug
    assignee = field(profile, "Assignee") or "your preparer"
    if missing.lower() in ("", "none"):
        print(f"nothing missing for {client}; no letter drafted")
        return
    items = [m.strip() for m in missing.split(";") if m.strip()]
    today = datetime.date.today().isoformat()
    lines = [
        f"# Documents still needed: {client}",
        "",
        f"Draft, {today}. Review before sending.",
        "",
        f"Dear {client},",
        "",
        "Thank you for sending your documents for the 2025 return. To finish it we still need:",
        "",
        *[f"- {item}" for item in items],
        "",
        (
            "This was on last year's return or on the list you gave us. Please upload it to the "
            "portal when it arrives, or tell us if it no longer applies, and we'll take it off the list."
            if len(items) == 1
            else "Each of these was on last year's return or on the list you gave us. Please upload "
            "them to the portal when they arrive, or tell us if one no longer applies, and we'll "
            "take it off the list."
        ),
        "",
        "Thank you,",
        "",
        assignee,
        "",
    ]
    os.makedirs("letters", exist_ok=True)
    out = os.path.join("letters", f"{slug}.md")
    with open(out, "w", encoding="utf-8") as f:
        f.write("\n".join(lines))
    print(f"wrote {out}")


if __name__ == "__main__":
    main(sys.argv)
