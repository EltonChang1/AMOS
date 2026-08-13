# Phase 0 execution backlog

Status: **in progress**

Source: [MVP execution plan](MVP_EXECUTION_PLAN.md#phase-0--program-reset-and-repository-truth-days-15)

This is the ordered, repository-controlled backlog for Phase 0. A role owner
identifies the accountable workstream; the founders must replace every `TBD`
assignee with one named person during the kickoff. An item is complete only
when its acceptance condition is demonstrably true.

## Baseline recorded at kickoff

- Baseline commit: `b1c2602b305562f40c9ad117d7d3e13e2cef2ed8`
- Toolchain: Rust `1.97.0`, pinned in `rust-toolchain.toml`
- Local verification on 2026-08-12: formatting, warning-free Clippy and docs,
  64 debug tests, 64 release tests, optimized build, the 10,000-item bounded
  control-path benchmark, and the RustSec audit all passed.
- Known repository state: the proposed execution plan and `.demo-advisor/`
  were untracked when Phase 0 began. `.demo-advisor/` is outside this backlog
  and must not be included in an MVP baseline change without a separate review.

## Ordered work items

| ID | Work item | Role owner | Named assignee | Depends on | Acceptance condition | State |
| --- | --- | --- | --- | --- | --- | --- |
| P0-01 | Approve the execution plan and kickoff roles | Product and design-partner lead | TBD | None | The plan is reviewed, its status is changed from proposed to approved, and every workstream below has a named assignee. | Blocked on founder decision |
| P0-02 | Establish this ordered Phase 0 backlog | Product and design-partner lead | TBD | P0-01 | Every Phase 0 plan item has an owner, dependency, acceptance condition, and current state in one reviewed backlog. | Ready for review |
| P0-03 | Freeze the Rust control-contract baseline | Agent and runtime lead | TBD | P0-01 | The baseline commit and toolchain are recorded, existing regression behavior is unchanged, and changes to durable contracts require an ADR plus regression updates. | Ready for review |
| P0-04 | Enforce the Rust baseline in CI | Artifacts and platform lead | TBD | P0-03 | Pull requests and main-branch pushes gate formatting, warning-free Clippy, debug and release tests, optimized build, warning-free docs, dependency audit, and the bounded benchmark. | Ready for review |
| P0-05 | Confirm IP ownership and add the intended root license | Product and design-partner lead | TBD | P0-01 | All contributors/IP owners confirm the licensing decision; `Cargo.toml`, the root license file, and distribution metadata agree. | Blocked on founder/IP decision |
| P0-06 | Publish the security policy and private reporting channel | Security owner | TBD | P0-01 | `SECURITY.md` states supported versions, a working private reporting route, severity definitions, response targets, and customer-notification policy. | Not started |
| P0-07 | Classify Python-era scenarios and evaluation commands | Agent and runtime lead | TBD | P0-01 | Each legacy scenario/evaluator is either labeled archived evidence or restored and exercised by CI; no documentation implies a missing command still executes. | Ready for review |
| P0-08 | Separate research evidence from product-readiness claims | Product and design-partner lead | TBD | P0-07 | `PRODUCT_READINESS.md`, the README, and evaluation documentation consistently distinguish historical research results from current executable evidence. | Ready for review |
| P0-09 | Record the seven baseline architecture decisions | Agent and runtime lead | TBD | P0-01 | Reviewed ADRs cover the modular monolith, PostgreSQL, connector-mediated execution, model-provider boundary, artifact IR, worker isolation, and deployment packaging. | Proposed ADRs ready for review |
| P0-10 | Define environment configuration profiles | Artifacts and platform lead | TBD | P0-09 | Development, staging, pilot, and production profiles define required services, secret sources, fail-closed defaults, and prohibited demo settings. | Proposed profiles ready for review |
| P0-11 | Establish and prioritize the MVP risk register | Security owner | TBD | P0-01 | The register scores customer demand, model quality, metric correctness, connector behavior, permissions, deployment, support, and consulting-creep risks, with owners and mitigations. | Initial register ready for review |
| P0-12 | Constrain the payment workflow to a regression fixture | Data and connector lead | TBD | P0-03 | Product documentation and routing treat payment as a fixture; no new product requirement depends on payment-specific tables, metrics, dates, prose, or artifacts. | Not started |
| P0-13 | Audit the Phase 0 exit gate | Product and design-partner lead | TBD | P0-02 through P0-12 | Main is green; current and historical evidence are unambiguous; all MVP work has ownership, dependencies, and acceptance conditions; and the ordered backlog is reviewed. | Not started |

## Contract-change rule

Until Phase 0 closes, the recorded tests are the frozen executable contract.
Any intentional change to a trust boundary, durable data or API contract,
deployment model, or MVP scope must include an ADR and corresponding regression
updates in the same reviewed change.

## Decisions required at kickoff

1. Name the five accountable role owners and every work-item assignee.
2. Approve or amend `MVP_EXECUTION_PLAN.md`.
3. Confirm contributor/IP ownership and whether the intended distribution
   license is MIT, as currently declared by `Cargo.toml`.
4. Select a private security-reporting route before publishing `SECURITY.md`.

Repository check on 2026-08-12: `EltonChang1/AMOS` is public and GitHub private
vulnerability reporting is disabled. Enabling it changes external repository
security settings and requires an explicit owner decision.
