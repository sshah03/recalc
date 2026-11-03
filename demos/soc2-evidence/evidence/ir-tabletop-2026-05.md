# Incident response tabletop exercise — May 2026

Date held: 2026-05-14, 14:00–16:30 UTC
Facilitator: Mira Moravec, Head of Security
Participants: Rafe Moravec (Engineering), Greer Galloway (Infrastructure), Bram Balogun (IT), Hana Brandt (People), Lenno Lindqvist (Legal), Ada Achterberg (Customer Success)

## Scenario

A customer's payroll administrator reports that a direct-deposit change they did not make was applied to twelve employees. Investigation finds a session token for that administrator was reused from an unfamiliar network.

## What went well

- Severity was assigned within 10 minutes and the incident channel opened from the runbook.
- Engineering could list every direct-deposit change by session within 30 minutes from the audit log.

## Gaps and follow-ups

| # | Gap | Owner | Due |
|---|---|---|---|
| 1 | The customer-notification template does not cover payroll-specific data (bank details). | Lenno Lindqvist | 2026-06-30 |
| 2 | No documented step to freeze pending ACH batches for an affected customer. | Rafe Moravec | 2026-06-15 |

Follow-ups tracked as SEC-318 and SEC-319. Both were closed by 2026-06-26.

Result: the incident response plan was exercised as required by the Incident Response Policy.
