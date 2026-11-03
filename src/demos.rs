//! Demo boards and their sample data, embedded in the binary for `recalc demo`.

use anyhow::{bail, Context, Result};
use std::path::Path;

pub struct Demo {
    pub name: &'static str,
    pub blurb: &'static str,
    /// (relative path, contents). `board.recalctemplate` is applied after writing.
    pub files: &'static [(&'static str, &'static [u8])],
}

macro_rules! demo_file {
    ($path:literal) => {
        ($path, include_bytes!(concat!("../demos/", $path)))
    };
}

pub const DEMOS: &[Demo] = &[
    Demo {
        name: "legal",
        blurb: "due diligence document grid: six contracts, typed columns, a file-group row, conditional risk columns",
        files: &[
            demo_file!("legal/board.recalctemplate"),
            demo_file!("legal/contracts/acme-master-services.md"),
            demo_file!("legal/contracts/acme-msa-amendment-1.md"),
            demo_file!("legal/contracts/globex-services-agreement.md"),
            demo_file!("legal/contracts/initech-mutual-nda.md"),
            demo_file!("legal/contracts/umbrella-saas-subscription.md"),
            demo_file!("legal/contracts/harborview-office-lease.md"),
            demo_file!("legal/contracts/okafor-employment-agreement.md"),
        ],
    },
    Demo {
        name: "tax",
        blurb: "document intake: five forms read into numbers, reconciled against the client's summary",
        files: &[
            demo_file!("tax/board.recalctemplate"),
            demo_file!("tax/client-summary.txt"),
            demo_file!("tax/documents/w2-hollis-manufacturing.txt"),
            demo_file!("tax/documents/1099-int-first-lakeshore-bank.txt"),
            demo_file!("tax/documents/1099-nec-brightpath-consulting.txt"),
            demo_file!("tax/documents/1099-div-vanguard-brokerage.txt"),
            demo_file!("tax/documents/1099-b-summit-brokerage-summary.txt"),
        ],
    },
    Demo {
        name: "bench",
        blurb: "benchmarks that keep score: watched cases, a history with sparklines, a regression gate",
        files: &[demo_file!("bench/board.recalctemplate"), demo_file!("bench/bench.py")],
    },
    Demo {
        name: "models",
        blurb: "the same tasks to Claude and Codex side by side, graded, with a judge for free text",
        files: &[demo_file!("models/board.recalctemplate")],
    },
    Demo {
        name: "research",
        blurb: "literature matrix over five abstracts, plus a synthesis cell fed the whole table",
        files: &[
            demo_file!("research/board.recalctemplate"),
            demo_file!("research/papers/01-speculative-decoding.md"),
            demo_file!("research/papers/02-kv-cache-quantization.md"),
            demo_file!("research/papers/03-continuous-batching.md"),
            demo_file!("research/papers/04-early-exit-layers.md"),
            demo_file!("research/papers/05-prompt-caching.md"),
        ],
    },
    Demo {
        name: "practice",
        blurb: "client grid: one question across eight clients, a re-runnable missing-documents check, request letters as actions",
        files: &[
            demo_file!("practice/board.recalctemplate"),
            demo_file!("practice/clients/alvarez-reyes/1099-int-lakeshore.txt"),
            demo_file!("practice/clients/alvarez-reyes/2024-form-1040-summary.txt"),
            demo_file!("practice/clients/alvarez-reyes/profile.md"),
            demo_file!("practice/clients/alvarez-reyes/w2-brightwater-dental.txt"),
            demo_file!("practice/clients/brennan/1099-div-pinecrest.txt"),
            demo_file!("practice/clients/brennan/2024-form-1040-summary.txt"),
            demo_file!("practice/clients/brennan/estimated-payments-2025.txt"),
            demo_file!("practice/clients/brennan/profile.md"),
            demo_file!("practice/clients/brennan/w2-corliss-engineering.txt"),
            demo_file!("practice/clients/chen-whitfield/1099-div-lantern-brokerage-corrected.txt"),
            demo_file!("practice/clients/chen-whitfield/1099-div-lantern-brokerage.txt"),
            demo_file!("practice/clients/chen-whitfield/2024-form-1040-summary.txt"),
            demo_file!("practice/clients/chen-whitfield/profile.md"),
            demo_file!("practice/clients/chen-whitfield/w2-halcyon-library.txt"),
            demo_file!("practice/clients/dunmore/1099-int-meridian-savings.txt"),
            demo_file!("practice/clients/dunmore/1099-nec-corvid-labs.txt"),
            demo_file!("practice/clients/dunmore/2024-form-1040-summary.txt"),
            demo_file!("practice/clients/dunmore/estimated-payments.txt"),
            demo_file!("practice/clients/dunmore/profile.md"),
            demo_file!("practice/clients/eastgate-fabrication/1099-int-granite-valley.txt"),
            demo_file!("practice/clients/eastgate-fabrication/1099-k-paywell.txt"),
            demo_file!("practice/clients/eastgate-fabrication/2024-form-1120s-summary.txt"),
            demo_file!("practice/clients/eastgate-fabrication/profile.md"),
            demo_file!("practice/clients/harbor-lane/1098-northgate-mortgage.txt"),
            demo_file!("practice/clients/harbor-lane/1099-int-granite-valley.txt"),
            demo_file!("practice/clients/harbor-lane/2024-form-1065-summary.txt"),
            demo_file!("practice/clients/harbor-lane/profile.md"),
            demo_file!("practice/clients/ivers-trust/1099-div-pinecrest.txt"),
            demo_file!("practice/clients/ivers-trust/1099-int-lakeshore.txt"),
            demo_file!("practice/clients/ivers-trust/2024-form-1041-summary.txt"),
            demo_file!("practice/clients/ivers-trust/estimated-payments-2025.txt"),
            demo_file!("practice/clients/ivers-trust/profile.md"),
            demo_file!("practice/clients/kowalczyk/1098-northgate-mortgage.txt"),
            demo_file!("practice/clients/kowalczyk/1099-int-meridian-savings.txt"),
            demo_file!("practice/clients/kowalczyk/2024-form-1040-summary.txt"),
            demo_file!("practice/clients/kowalczyk/estimated-payments.txt"),
            demo_file!("practice/clients/kowalczyk/profile.md"),
            demo_file!("practice/clients/kowalczyk/w2-ostrander-transit.txt"),
            demo_file!("practice/tools/draft_letter.py"),
            demo_file!("practice/tools/missing_docs.py"),
        ],
    },
    Demo {
        name: "entities",
        blurb: "entity grid: a client's S corporation, partnership, and trust, K-1s checked against her 1040",
        files: &[
            demo_file!("entities/board.recalctemplate"),
            demo_file!("entities/fairhaven-storage/1065-2025-return-summary.txt"),
            demo_file!("entities/fairhaven-storage/k1-1065-rosalind-kettering-amended.txt"),
            demo_file!("entities/fairhaven-storage/k1-1065-rosalind-kettering.txt"),
            demo_file!("entities/kettering-millwork/1120s-2025-return-summary.txt"),
            demo_file!("entities/kettering-millwork/k1-1120s-rosalind-kettering.txt"),
            demo_file!("entities/kettering-trust/1041-2025-return-summary.txt"),
            demo_file!("entities/kettering-trust/k1-1041-rosalind-kettering.txt"),
            demo_file!("entities/rosalind-kettering/1040-2025-draft-summary.txt"),
            demo_file!("entities/tools/schedule_e.py"),
        ],
    },
    Demo {
        name: "cve-triage",
        blurb: "dependency triage: eight scanner advisories checked against the code that ships, with version and call-site checks beside each agent answer",
        files: &[
            demo_file!("cve-triage/board.recalctemplate"),
            demo_file!("cve-triage/checks/advisory.py"),
            demo_file!("cve-triage/checks/reach.py"),
            demo_file!("cve-triage/issues/LLSA-2026-001.md"),
            demo_file!("cve-triage/issues/LLSA-2026-002.md"),
            demo_file!("cve-triage/issues/LLSA-2026-003.md"),
            demo_file!("cve-triage/issues/LLSA-2026-004.md"),
            demo_file!("cve-triage/issues/LLSA-2026-005.md"),
            demo_file!("cve-triage/issues/LLSA-2026-006.md"),
            demo_file!("cve-triage/issues/LLSA-2026-007.md"),
            demo_file!("cve-triage/issues/LLSA-2026-008.md"),
            demo_file!("cve-triage/repo/.gitignore"),
            demo_file!("cve-triage/repo/README.md"),
            demo_file!("cve-triage/repo/ledgerline/__init__.py"),
            demo_file!("cve-triage/repo/ledgerline/_deps.py"),
            demo_file!("cve-triage/repo/ledgerline/api.py"),
            demo_file!("cve-triage/repo/ledgerline/auth.py"),
            demo_file!("cve-triage/repo/ledgerline/config.py"),
            demo_file!("cve-triage/repo/ledgerline/http.py"),
            demo_file!("cve-triage/repo/ledgerline/importers/__init__.py"),
            demo_file!("cve-triage/repo/ledgerline/importers/bundle.py"),
            demo_file!("cve-triage/repo/ledgerline/ingest.py"),
            demo_file!("cve-triage/repo/ledgerline/ledger.py"),
            demo_file!("cve-triage/repo/ledgerline/storage.py"),
            demo_file!("cve-triage/repo/requirements-dev.txt"),
            demo_file!("cve-triage/repo/requirements.lock"),
            demo_file!("cve-triage/repo/tests/__init__.py"),
            demo_file!("cve-triage/repo/tests/fixtures/tenant.cfg"),
            demo_file!("cve-triage/repo/tests/helpers.py"),
            demo_file!("cve-triage/repo/tests/test_api.py"),
            demo_file!("cve-triage/repo/tests/test_auth.py"),
            demo_file!("cve-triage/repo/tests/test_bundle.py"),
            demo_file!("cve-triage/repo/tests/test_config.py"),
            demo_file!("cve-triage/repo/tests/test_http.py"),
            demo_file!("cve-triage/repo/tests/test_import.py"),
            demo_file!("cve-triage/repo/tests/test_ingest.py"),
            demo_file!("cve-triage/repo/tests/test_ledger.py"),
            demo_file!("cve-triage/repo/tests/test_storage.py"),
            demo_file!("cve-triage/repo/tools/lint.py"),
            demo_file!("cve-triage/repo/vendor/csvtide-1.8.0/csvtide/__init__.py"),
            demo_file!("cve-triage/repo/vendor/csvtide-2.0.0/CHANGELOG.md"),
            demo_file!("cve-triage/repo/vendor/csvtide-2.0.0/csvtide/__init__.py"),
            demo_file!("cve-triage/repo/vendor/fmtcolor-4.0.2/fmtcolor/__init__.py"),
            demo_file!("cve-triage/repo/vendor/httpcore-lite-2.2.0/httpcore_lite/__init__.py"),
            demo_file!("cve-triage/repo/vendor/jwtiny-1.3.2/jwtiny/__init__.py"),
            demo_file!("cve-triage/repo/vendor/pylintish-2.0.1/pylintish/__init__.py"),
            demo_file!("cve-triage/repo/vendor/tarsafe-0.7.9/tarsafe/__init__.py"),
            demo_file!("cve-triage/repo/vendor/tarsafe-0.8.1/CHANGELOG.md"),
            demo_file!("cve-triage/repo/vendor/tarsafe-0.8.1/tarsafe/__init__.py"),
            demo_file!("cve-triage/repo/vendor/urlkit-1.9.4/urlkit/__init__.py"),
            demo_file!("cve-triage/repo/vendor/yamlish-3.1.4/yamlish/__init__.py"),
            demo_file!("cve-triage/repo/vendor/yamlish-3.2.0/CHANGELOG.md"),
            demo_file!("cve-triage/repo/vendor/yamlish-3.2.0/yamlish/__init__.py"),
        ],
    },
    Demo {
        name: "soc2-evidence",
        blurb: "SOC 2 evidence: ten controls re-performed over six months of exports by commands an auditor can re-run, with agents for the judgement calls and a second agent re-performing each test",
        files: &[
            demo_file!("soc2-evidence/board.recalctemplate"),
            demo_file!("soc2-evidence/checks/controls.py"),
            demo_file!("soc2-evidence/documents/controls.csv"),
            demo_file!("soc2-evidence/evidence/access-reviews/2026-Q1.md"),
            demo_file!("soc2-evidence/evidence/access-reviews/2026-Q2.md"),
            demo_file!("soc2-evidence/evidence/backup-restore-tests.log"),
            demo_file!("soc2-evidence/evidence/change-tickets.csv"),
            demo_file!("soc2-evidence/evidence/github-merged-prs.json"),
            demo_file!("soc2-evidence/evidence/hris-hires.csv"),
            demo_file!("soc2-evidence/evidence/hris-terminations.csv"),
            demo_file!("soc2-evidence/evidence/ir-tabletop-2026-05.md"),
            demo_file!("soc2-evidence/evidence/okta-users.csv"),
            demo_file!("soc2-evidence/evidence/risk-register.csv"),
            demo_file!("soc2-evidence/evidence/training-completions.csv"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-01.json"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-02.json"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-03.json"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-04.json"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-05.json"),
            demo_file!("soc2-evidence/evidence/vuln-scans/2026-06.json"),
            demo_file!("soc2-evidence/policies/access-control.md"),
            demo_file!("soc2-evidence/policies/backup-and-recovery.md"),
            demo_file!("soc2-evidence/policies/change-management.md"),
            demo_file!("soc2-evidence/policies/incident-response.md"),
            demo_file!("soc2-evidence/policies/security-training.md"),
            demo_file!("soc2-evidence/policies/vendor-management.md"),
            demo_file!("soc2-evidence/policies/vulnerability-management.md"),
        ],
    },
    Demo {
        name: "sec-questionnaire",
        blurb: "security questionnaire: a customer's twelve questions answered from our policies and reports, each configuration claim tested against the Terraform, with a reviewer and a ready-to-send gate",
        files: &[
            demo_file!("sec-questionnaire/board.recalctemplate"),
            demo_file!("sec-questionnaire/checks/config.py"),
            demo_file!("sec-questionnaire/documents/questionnaire.csv"),
            demo_file!("sec-questionnaire/infra/backup.tf"),
            demo_file!("sec-questionnaire/infra/github-org-settings.json"),
            demo_file!("sec-questionnaire/infra/idp-export.json"),
            demo_file!("sec-questionnaire/infra/main.tf"),
            demo_file!("sec-questionnaire/infra/rds.tf"),
            demo_file!("sec-questionnaire/infra/s3.tf"),
            demo_file!("sec-questionnaire/policies/hr-security.md"),
            demo_file!("sec-questionnaire/policies/information-security-policy-v1.md"),
            demo_file!("sec-questionnaire/policies/information-security-policy-v2.md"),
            demo_file!("sec-questionnaire/policies/subprocessors.md"),
            demo_file!("sec-questionnaire/policies/vendor-management.md"),
            demo_file!("sec-questionnaire/reports/pentest-2025-07-letter.md"),
            demo_file!("sec-questionnaire/reports/soc2-bridge-letter-2026-07.md"),
            demo_file!("sec-questionnaire/reports/soc2-type2-2025-excerpt.md"),
        ],
    },
    Demo {
        name: "claims-ledger",
        blurb: "claims ledger: four agents working one repository in parallel, each 'done' held to its acceptance test as the code keeps changing, so a claim another agent's edit broke turns STALE",
        files: &[
            demo_file!("claims-ledger/board.recalctemplate"),
            demo_file!("claims-ledger/repo/README.md"),
            demo_file!("claims-ledger/repo/tally/__init__.py"),
            demo_file!("claims-ledger/repo/tally/dates.py"),
            demo_file!("claims-ledger/repo/tally/export.py"),
            demo_file!("claims-ledger/repo/tally/invoice.py"),
            demo_file!("claims-ledger/repo/tally/money.py"),
            demo_file!("claims-ledger/repo/tests/__init__.py"),
            demo_file!("claims-ledger/repo/tests/test_dates.py"),
            demo_file!("claims-ledger/repo/tests/test_export.py"),
            demo_file!("claims-ledger/repo/tests/test_invoice.py"),
            demo_file!("claims-ledger/repo/tests/test_parse.py"),
        ],
    },
    Demo {
        name: "done-means-green",
        blurb: "done means green: Claude Code sessions on four tasks, each kept from finishing by a Stop hook until its acceptance test passes",
        files: &[
            demo_file!("done-means-green/board.recalctemplate"),
            demo_file!("done-means-green/repo/.claude/settings.json"),
            demo_file!("done-means-green/repo/.codex/hooks.json"),
            demo_file!("done-means-green/repo/README.md"),
            demo_file!("done-means-green/repo/tally/__init__.py"),
            demo_file!("done-means-green/repo/tally/dates.py"),
            demo_file!("done-means-green/repo/tally/export.py"),
            demo_file!("done-means-green/repo/tally/invoice.py"),
            demo_file!("done-means-green/repo/tally/money.py"),
            demo_file!("done-means-green/repo/tests/__init__.py"),
            demo_file!("done-means-green/repo/tests/test_dates.py"),
            demo_file!("done-means-green/repo/tests/test_export.py"),
            demo_file!("done-means-green/repo/tests/test_invoice.py"),
            demo_file!("done-means-green/repo/tests/test_parse.py"),
        ],
    },
    Demo {
        name: "flaky-hunt",
        blurb: "flaky-test hunt: four suspects run 50 times before and after, an agent naming each cause, fix sessions held until the test passes every run, and a rule-of-three bound on what's left",
        files: &[
            demo_file!("flaky-hunt/board.recalctemplate"),
            demo_file!("flaky-hunt/repo/.claude/settings.json"),
            demo_file!("flaky-hunt/repo/.codex/hooks.json"),
            demo_file!("flaky-hunt/repo/README.md"),
            demo_file!("flaky-hunt/repo/shipit/__init__.py"),
            demo_file!("flaky-hunt/repo/shipit/backoff.py"),
            demo_file!("flaky-hunt/repo/shipit/review.py"),
            demo_file!("flaky-hunt/repo/shipit/tags.py"),
            demo_file!("flaky-hunt/repo/shipit/totals.py"),
            demo_file!("flaky-hunt/repo/tests/__init__.py"),
            demo_file!("flaky-hunt/repo/tests/test_backoff.py"),
            demo_file!("flaky-hunt/repo/tests/test_review.py"),
            demo_file!("flaky-hunt/repo/tests/test_tags.py"),
            demo_file!("flaky-hunt/repo/tests/test_totals.py"),
        ],
    },
    Demo {
        name: "review-lanes",
        blurb: "review lanes: open pull requests as rows kept in sync with the PR list, facts beside them, a security, tests and design lane per PR, and each lane's precision from your verify and flag marks",
        files: &[
            demo_file!("review-lanes/board.recalctemplate"),
            demo_file!("review-lanes/repo/tools/gh"),
            demo_file!("review-lanes/repo/prs/list.json"),
            demo_file!("review-lanes/repo/prs/201.diff"),
            demo_file!("review-lanes/repo/prs/202.diff"),
            demo_file!("review-lanes/repo/prs/203.diff"),
            demo_file!("review-lanes/repo/prs/204.diff"),
            demo_file!("review-lanes/repo/prs/205.diff"),
            demo_file!("review-lanes/repo/prs/206.diff"),
        ],
    },
    Demo {
        name: "release-gate",
        blurb: "release gate: scripted checks that re-run on their own, a person's sign-off, agent-drafted notes a person verifies, and a tag action that refuses until everything passes",
        files: &[
            demo_file!("release-gate/board.recalctemplate"),
            demo_file!("release-gate/repo/README.md"),
            demo_file!("release-gate/repo/pyproject.toml"),
            demo_file!("release-gate/repo/src/acme/__init__.py"),
            demo_file!("release-gate/repo/CHANGELOG.md"),
            demo_file!("release-gate/repo/merged.txt"),
            demo_file!("release-gate/repo/migrations/006_login_failures.up.sql"),
            demo_file!("release-gate/repo/migrations/006_login_failures.down.sql"),
            demo_file!("release-gate/repo/migrations/007_users_name_index.up.sql"),
            demo_file!("release-gate/repo/scripts/versions.sh"),
            demo_file!("release-gate/repo/scripts/check-down.sh"),
            demo_file!("release-gate/repo/tools/gh"),
            demo_file!("release-gate/repo/tools/tag"),
        ],
    },
    Demo {
        name: "beads-board",
        blurb: "beads board: a beads issue list kept in sync as rows, each bead's test beside it, ready when the bead it follows holds rather than when it's closed, and a close that waits for the test and your sign-off",
        files: &[
            demo_file!("beads-board/board.recalctemplate"),
            demo_file!("beads-board/repo/.beads/standin.json"),
            demo_file!("beads-board/repo/.claude/settings.json"),
            demo_file!("beads-board/repo/.codex/hooks.json"),
            demo_file!("beads-board/repo/README.md"),
            demo_file!("beads-board/repo/tally/__init__.py"),
            demo_file!("beads-board/repo/tally/dates.py"),
            demo_file!("beads-board/repo/tally/export.py"),
            demo_file!("beads-board/repo/tally/invoice.py"),
            demo_file!("beads-board/repo/tally/money.py"),
            demo_file!("beads-board/repo/tests/__init__.py"),
            demo_file!("beads-board/repo/tests/test_dates.py"),
            demo_file!("beads-board/repo/tests/test_export.py"),
            demo_file!("beads-board/repo/tests/test_invoice.py"),
            demo_file!("beads-board/repo/tests/test_parse.py"),
            demo_file!("beads-board/repo/tools/bd"),
        ],
    },
    Demo {
        name: "scorp",
        blurb: "S-corp returns: five fictional clients' 2026 books classified onto the IRS's draft Form 1120-S, formulas building and checking each return, one sheet per client, and an action that fills the draft PDF",
        files: &[
            demo_file!("scorp/board.recalctemplate"),
            demo_file!("scorp/clients/harbor/client-email.md"),
            demo_file!("scorp/clients/harbor/distributions-2026.md"),
            demo_file!("scorp/clients/harbor/engagement.md"),
            demo_file!("scorp/clients/harbor/fixed-assets-2026.md"),
            demo_file!("scorp/clients/harbor/return-2025-summary.md"),
            demo_file!("scorp/clients/harbor/trial-balance-2026.csv"),
            demo_file!("scorp/clients/harbor/w3-2026.md"),
            demo_file!("scorp/clients/kestrel/client-email.md"),
            demo_file!("scorp/clients/kestrel/distributions-2026.md"),
            demo_file!("scorp/clients/kestrel/engagement.md"),
            demo_file!("scorp/clients/kestrel/fixed-assets-2026.md"),
            demo_file!("scorp/clients/kestrel/return-2025-summary.md"),
            demo_file!("scorp/clients/kestrel/trial-balance-2026.csv"),
            demo_file!("scorp/clients/kestrel/w3-2026.md"),
            demo_file!("scorp/clients/lumen/client-email.md"),
            demo_file!("scorp/clients/lumen/distributions-2026.md"),
            demo_file!("scorp/clients/lumen/engagement.md"),
            demo_file!("scorp/clients/lumen/fixed-assets-2026.md"),
            demo_file!("scorp/clients/lumen/return-2025-summary.md"),
            demo_file!("scorp/clients/lumen/trial-balance-2026.csv"),
            demo_file!("scorp/clients/lumen/w3-2026.md"),
            demo_file!("scorp/clients/northside/client-email.md"),
            demo_file!("scorp/clients/northside/distributions-2026.md"),
            demo_file!("scorp/clients/northside/engagement.md"),
            demo_file!("scorp/clients/northside/fixed-assets-2026.md"),
            demo_file!("scorp/clients/northside/return-2025-summary.md"),
            demo_file!("scorp/clients/northside/trial-balance-2026.csv"),
            demo_file!("scorp/clients/northside/w3-2026.md"),
            demo_file!("scorp/clients/pinecrest/client-email.md"),
            demo_file!("scorp/clients/pinecrest/distributions-2026.md"),
            demo_file!("scorp/clients/pinecrest/engagement.md"),
            demo_file!("scorp/clients/pinecrest/fixed-assets-2026.md"),
            demo_file!("scorp/clients/pinecrest/return-2025-summary.md"),
            demo_file!("scorp/clients/pinecrest/trial-balance-2026.csv"),
            demo_file!("scorp/clients/pinecrest/w3-2026.md"),
            demo_file!("scorp/guide/mapping.md"),
            demo_file!("scorp/irs/f1120s--dft.pdf"),
            demo_file!("scorp/scripts/fill_1120s.py"),
            demo_file!("scorp/scripts/form_map.py"),
            demo_file!("scorp/scripts/py.sh"),
            demo_file!("scorp/scripts/setup.sh"),
        ],
    },
];

pub fn find(name: &str) -> Option<&'static Demo> {
    DEMOS.iter().find(|d| d.name == name)
}

/// Write a demo's files into an empty `dir` and return the board text to apply.
pub fn unpack(demo: &Demo, dir: &Path) -> Result<&'static str> {
    // A fresh board's directory holds only `.recalc/`, which counts as empty.
    let occupied = dir.exists()
        && dir
            .read_dir()?
            .filter_map(|e| e.ok())
            .any(|e| e.file_name() != ".recalc");
    if occupied {
        bail!(
            "{} already exists and isn't empty; pick another --into",
            dir.display()
        );
    }
    let prefix = format!("{}/", demo.name);
    let mut board = None;
    for (path, contents) in demo.files {
        let rel = path
            .strip_prefix(&prefix)
            .expect("demo files live under their demo's folder");
        // repo/ unpacks at the board's root, where the board's commands run.
        let rel = rel.strip_prefix("repo/").unwrap_or(rel);
        let target = dir.join(rel);
        std::fs::create_dir_all(target.parent().expect("has a parent"))?;
        std::fs::write(&target, contents)
            .with_context(|| format!("writing {}", target.display()))?;
        if rel == "board.recalctemplate" {
            board = Some(std::str::from_utf8(contents).context("the demo's board is UTF-8")?);
        }
    }
    board.context("demo has no board.recalctemplate")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::Engine;
    use crate::model::{Actor, CellKind, CellState};

    /// Every demo applies with no cell in Error and its agent cells Stale.
    #[tokio::test]
    async fn every_demo_unpacks_and_applies_cleanly() {
        for demo in DEMOS {
            let dir = tempfile::tempdir().unwrap();
            let target = dir.path().join(demo.name);
            let board = unpack(demo, &target).unwrap();
            let mut engine = Engine::open(&target).unwrap();
            engine
                .set_timeout(60_000) // the bench cases run python
                ;
            engine.apply_template(board, Actor::Cli).unwrap();
            engine.drain().await.unwrap();
            let mut agents = 0;
            for cell in engine.all_cells() {
                assert!(
                    !matches!(cell.state, CellState::Error(_)),
                    "{}: {} is in error: {:?}",
                    demo.name,
                    cell.name,
                    cell.state
                );
                if cell.kind == CellKind::AgentCall {
                    agents += 1;
                    assert_eq!(
                        cell.state,
                        CellState::Stale,
                        "{}: agent cells wait to be run",
                        demo.name
                    );
                }
            }
            assert!(agents > 0, "{}: a demo has agent cells", demo.name);
            // Unpacking twice into the same place is refused, not overwritten.
            assert!(unpack(demo, &target).is_err());
        }
    }

    #[tokio::test]
    async fn bench_demo_runs_its_cases() {
        let dir = tempfile::tempdir().unwrap();
        let board = unpack(find("bench").unwrap(), dir.path()).unwrap();
        let mut engine = Engine::open(dir.path()).unwrap();
        engine.set_timeout(60_000);
        engine.apply_template(board, Actor::Cli).unwrap();
        engine.drain().await.unwrap();
        let total: f64 = engine.cell("total_ms").unwrap().value.parse().unwrap();
        assert!(total > 0.0);
        assert_eq!(engine.cell("runs").unwrap().value, "1");
        assert!(engine
            .cell("median_ms")
            .unwrap()
            .value
            .parse::<f64>()
            .is_ok());
        assert!(matches!(
            engine.cell("regressed").unwrap().value.as_str(),
            "true" | "false"
        ));
    }

    #[tokio::test]
    async fn every_demo_board_parses() {
        for demo in DEMOS {
            let dir = tempfile::tempdir().unwrap();
            let mut e = crate::engine::Engine::open(dir.path()).unwrap();
            let board = demo
                .files
                .iter()
                .find(|(p, _)| p.ends_with("board.recalctemplate"))
                .map(|(_, b)| std::str::from_utf8(b).unwrap())
                .unwrap();
            e.apply_template(board, crate::model::Actor::Cli).unwrap();
            assert_eq!(
                e.source_problems(),
                Vec::<(String, String)>::new(),
                "demo {}",
                demo.name
            );
        }
    }
}
