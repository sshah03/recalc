# Brindlewood, Inc. — Backup and Recovery Policy (v1.5, effective 2025-10-01)

1. The production database is backed up daily by snapshot, and snapshots are retained for 35 days.
2. Each calendar quarter, a snapshot is restored to an isolated account and verified. Each run is logged by the restore-test runner.
3. If a restore test fails, the failure is logged as an incident and the test is repeated. **A failed test followed by a passing retest within 7 days meets the quarterly requirement** and is not a control exception.
