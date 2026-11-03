# Brindlewood, Inc. — Change Management Policy (v2.4, effective 2025-10-01)

## 1. Code changes

1.1 Changes to payroll-core reach production only through pull requests merged to `main`.

1.2 Every pull request needs at least one approving review from someone other than its author before merge.

1.3 Administrator bypass of branch protection is prohibited, except for an emergency change. An emergency change must have an emergency change record (EC-number) opened within one business day of the merge and approved by the Head of Engineering within two business days. A bypass without such a record is an exception.

## 2. Infrastructure changes

2.1 Production infrastructure changes are raised as change tickets.

2.2 A **Normal** change must be approved before it is implemented (`approved_at` earlier than `implemented_at`).

2.3 A **Standard** change is a low-risk change listed in the pre-approved standard change catalog (SC-numbers). It is approved by being in the catalog and needs no individual approval. It is not an exception.

2.4 A cancelled ticket was never implemented and is out of scope.
