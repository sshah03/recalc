# SOC 2 Type II report (excerpt) — Tamsin Labs, Inc.

Report on Tamsin Labs' description of its analytics platform and on the suitability of the design and operating effectiveness of its controls relevant to security and availability, for the period **1 January 2025 to 31 December 2025**.

Service auditor: **Pellham & Voss LLP**, Certified Public Accountants. Report dated 18 February 2026.

## Section 1: Independent service auditor's opinion (excerpt)

In our opinion, in all material respects, the description presents Tamsin Labs' system as designed and implemented throughout the period, and the controls stated in the description were suitably designed and operated effectively throughout the period to provide reasonable assurance that the service commitments and system requirements were achieved, based on the applicable trust services criteria.

## Section 4: Tests of controls and results (excerpt)

| Control | Control description | Test performed | Result |
|---|---|---|---|
| CC1.4.1 | Workforce members complete security awareness training on hire and annually. | Inspected LMS completion records for a sample of 25 workforce members. | No exceptions noted. |
| CC6.1.3 | MFA is enforced for workforce access to production. | Inspected IdP sign-on policy configuration. | No exceptions noted. |
| CC6.2.2 | Production user access is reviewed quarterly and the review is completed within 30 days of quarter end. | Inspected the four quarterly review records. | **Exception noted.** The Q3 2025 access review was completed and signed on 12 November 2025, 43 days after quarter end. |
| CC8.1.1 | Changes to production require an approved pull request. | Inspected branch protection settings and a sample of 40 merged pull requests. | No exceptions noted. |
| A1.2.4 | Production databases are backed up daily. | Inspected backup configuration and job history for a sample of 20 days. | No exceptions noted. |

### Management's response to the CC6.2.2 exception

The Q3 2025 review was delayed by the migration of our identity provider. All access was reviewed when the review was completed and no inappropriate access was found. The Q4 2025 review was completed on time.
