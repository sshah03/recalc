"""Test a configuration claim against the files in infra/, so a questionnaire
answer that says "yes, encrypted" can't stand next to a bucket that isn't.

    python3 checks/config.py "Encryption at rest"
    python3 checks/config.py MFA
    python3 checks/config.py "Branch protection"
    python3 checks/config.py Backups
    python3 checks/config.py None            # n/a

Prints pass, or fail: <the resources that fall short>. It reads only the
Terraform and exports under infra/, the way an auditor would.
"""
import glob
import json
import re
import sys

claim = " ".join(sys.argv[1:]).strip().lower()
tf = "\n".join(open(p).read() for p in sorted(glob.glob("infra/*.tf")))


def resources(kind):
    """(name, body) of every `resource "<kind>" "<name>" { ... }` block."""
    out = []
    for m in re.finditer(rf'resource "{kind}" "(\w+)" \{{', tf):
        depth, i = 1, m.end()
        while depth:
            depth += {"{": 1, "}": -1}.get(tf[i], 0)
            i += 1
        out.append((m.group(1), tf[m.end():i - 1]))
    return out


def report(failing):
    print("fail: " + ", ".join(failing) if failing else "pass")


if claim in ("", "none", "n/a"):
    print("n/a")
elif claim.startswith("encryption"):
    failing = [f"aws_db_instance.{n}" for n, body in resources("aws_db_instance") if not re.search(r"storage_encrypted\s*=\s*true", body)]
    encrypted = {re.search(r"aws_s3_bucket\.(\w+)\.", body).group(1) for _, body in resources("aws_s3_bucket_server_side_encryption_configuration")}
    failing += [f"aws_s3_bucket.{n}" for n, _ in resources("aws_s3_bucket") if n not in encrypted]
    report(failing)
elif claim == "mfa":
    idp = json.load(open("infra/idp-export.json"))
    report([f"{', '.join(p['applies_to_groups'])} (sign-on policy '{p['name']}')" for p in idp["sign_on_policies"] if p["status"] == "ACTIVE" and not p["rule"].get("mfa_required")])
elif claim.startswith("branch"):
    gh = json.load(open("infra/github-org-settings.json"))
    failing = []
    for r in gh["repositories"]:
        if not r.get("production"):
            continue
        bp = r.get("branch_protection") or {}
        reviews = (bp.get("required_pull_request_reviews") or {}).get("required_approving_review_count", 0)
        if bp.get("branch") != r["default_branch"] or reviews < 1 or not bp.get("enforce_admins"):
            failing.append(r["name"])
    report(failing)
elif claim.startswith("backup"):
    daily = [n for n, body in resources("aws_backup_plan") if re.search(r'schedule\s*=\s*"cron\(\S+ \S+ \* \* \? \*\)"', body)]
    report([] if daily else ["no daily aws_backup_plan schedule"])
else:
    print(f"fail: no check for the claim {claim!r}")
