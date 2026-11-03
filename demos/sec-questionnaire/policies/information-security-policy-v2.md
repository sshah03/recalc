# Tamsin Labs, Inc. — Information Security Policy

| Document control | |
|---|---|
| Version | 2.0 |
| Status | Approved, in force |
| Effective date | 2026-03-01 |
| Supersedes | Information Security Policy v1.3 (effective 2024-02-01) |
| Owner | Head of Security (Ines Varga) |
| Approved by | Chief Technology Officer (Rowan Achterberg), 2026-02-19 |
| Next review | 2027-03-01 |

## 1. Purpose and scope

This policy sets the minimum security requirements for Tamsin Labs, Inc. ("Tamsin") and applies to all employees, contractors and temporary staff (the "workforce"), and to every system that stores, processes or transmits customer data. Where a standard or runbook conflicts with this policy, this policy controls.

## 2. Data classification

Customer data is classified **Restricted**. It includes data customers upload to the Tamsin platform, analytics derived from it, and exports generated from it.

## 3. Encryption

3.1 All customer data is encrypted at rest using AES-256 or an equivalent algorithm, in every data store that holds it, including databases, object storage buckets, backups and exports. Encryption for each data store is declared explicitly in infrastructure code; a cloud provider's default setting is not relied on.

3.2 Customer data is encrypted in transit using TLS 1.2 or later.

3.3 Encryption keys are managed in the cloud provider's key management service and rotated annually.

## 4. Access control

4.1 Access is granted on the principle of least privilege and approved by the system owner.

4.2 Multi-factor authentication is required for all workforce access to production systems and corporate applications. There are no standing exceptions.

4.3 User access to production systems is reviewed quarterly by system owners. Each quarterly review is completed and signed within 30 days of quarter end.

4.4 Access is removed within one business day of a workforce member's termination.

## 5. Change management

5.1 All changes to production code and infrastructure are made through pull requests to a protected default branch.

5.2 Every pull request requires at least one approving review from someone other than the author before merge. Branch protection is enforced for administrators.

## 6. Business continuity and disaster recovery

6.1 The recovery time objective (RTO) for the production service is **4 hours**. The recovery point objective (RPO) is 1 hour.

6.2 Production databases are backed up daily and backups are retained for 35 days.

6.3 Restore procedures are documented in the operations runbook.

## 7. Vulnerability management

7.1 An independent third party performs a penetration test of the production application annually.

7.2 Critical findings are remediated within 15 days and high findings within 30 days.

## 8. Incident response

8.1 Tamsin maintains a documented incident response plan, owned by the Head of Security and tested annually.

8.2 When a security incident is confirmed to affect a customer's data, Tamsin notifies the affected customer without undue delay.

## 9. Security awareness

9.1 All workforce members complete security awareness training within 30 days of starting and at least annually after that. Completion is tracked in the learning management system.

## 10. Third parties

10.1 Vendors that store or process customer data are assessed before onboarding and reviewed annually under the Vendor Management Policy. Current sub-processors are listed in `subprocessors.md`.

## 11. Exceptions

Exceptions to this policy require written approval from the Head of Security, a documented compensating control and an expiry date no more than 12 months out.
