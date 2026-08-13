# AMOS MVP risk register

Status: **initial prioritization; review weekly**

Likelihood and impact use a 1–5 scale. Score is likelihood multiplied by
impact. Named owners remain `TBD` until the founder kickoff; role ownership is
still accountable in the interim.

| Priority | Risk | L | I | Score | Role owner | Named owner | Early signal | Current mitigation / next action |
| ---: | --- | ---: | ---: | ---: | --- | --- | --- | --- |
| 1 | No paid partner grants workflow and source access | 4 | 5 | 20 | Product and design-partner lead | TBD | Qualified interviews do not yield a named buyer, weekly workflow, data owner, reviewer, budget, and decision date | Run Phase 1 in parallel; stop broad engineering after the plan's 30-interview gate rather than substituting fixtures for demand. |
| 2 | Customer workflow expands into bespoke consulting | 4 | 4 | 16 | Product and design-partner lead | TBD | Requests add sources, departments, arbitrary analyses, or outputs without removing scope | Fixed-scope SOW; one source/workflow/reviewer/destination; price and approve material changes separately; maintain deployment journal. |
| 3 | Metric or population semantics produce a wrong material number | 3 | 5 | 15 | Data and connector lead | TBD | Definitions are ambiguous, changed outside AMOS, or lack sealed reference outputs | Versioned owner-approved metric definitions, explicit time/population parameters, independent recomputation, 100% published-number accuracy gate. |
| 4 | Cross-tenant, cross-role, or source-native permission leak | 3 | 5 | 15 | Security owner | TBD | Missing tenant context, connector delegation mismatch, RLS bypass, or restricted evidence in artifacts/logs | Composite tenant keys, forced RLS, repository predicates, connector identity propagation, capability binding, adversarial isolation tests, zero-leak release gate. |
| 5 | Model plans or narratives fail the selected workflow | 3 | 4 | 12 | Agent and runtime lead | TBD | High malformed-plan, unsupported-claim, repair, timeout, or reviewer-correction rates | Provider-neutral structured interface, bounded plans/tools, frozen customer tasks, independent verification, model/prompt evaluation and explicit fallback. |
| 6 | Production connector misbehaves under drift, revocation, limits, or outages | 3 | 4 | 12 | Data and connector lead | TBD | Schema/freshness changes go undetected; retries amplify load; revocation is not visible | Select only after signed demand; capability-mediated path; real-service conformance suite; redacted health; operator runbook. |
| 7 | Customer-contained deployment cannot be installed or recovered predictably | 3 | 4 | 12 | Artifacts and platform lead | TBD | Manual steps, configuration drift, failed clean installs, incomplete backups, or undeclared dependencies | Signed compose/VM package, preflight, environment profiles, infrastructure automation, clean-install and coordinated restore drills. |
| 8 | Review experience is too slow to displace analyst effort | 3 | 4 | 12 | Product and design-partner lead | TBD | Review minutes or correction time match/exceed current workflow | Claim-level support UI, assumptions/freshness beside narrative, customer baseline, weekly scorecard, reject unsupported prose before review. |
| 9 | Duplicate or incorrect publication occurs during retry/recovery | 2 | 5 | 10 | Artifacts and platform lead | TBD | Lost acknowledgments, repeated destination objects, hash mismatch, or publish after policy change | Hash-bound idempotency key, pre-publication revalidation, destination acknowledgment record, lost-ack and crash recovery tests. |
| 10 | Secrets or customer data escape through model, logs, artifacts, or workers | 2 | 5 | 10 | Security owner | TBD | Ambient credentials, prompt/result logging, unrestricted egress, or sensitive columns in output | Customer-approved boundary, separate secret identities, deny-by-default egress, isolated workers, classification/redaction, credential and log scanning. |
| 11 | PostgreSQL migration changes current A-TXN correctness | 2 | 5 | 10 | Artifacts and platform lead | TBD | Lost CAS conflicts, stale fences commit, RLS differs by role, or migrations cannot restart | Port behind existing store contract; run concurrency/failure suite against PostgreSQL; online migration and rollback rehearsal; canary isolation checks. |
| 12 | Four-founder team cannot support delivery and incidents concurrently | 3 | 3 | 9 | Product and design-partner lead | TBD | Critical work becomes ownerless, response targets slip, or pilot interruptions consume roadmap | Named code/operational owners, scope cap, daily critical path, support rotation, runbooks, and explicit stop/narrow triggers. |

## Weekly review protocol

For every risk with score 10 or higher:

1. confirm likelihood, impact, owner, and evidence changed since last review;
2. record whether the early signal fired;
3. assign one dated mitigation action or accept the exposure explicitly;
4. escalate any critical security, permission, data-loss, metric-correctness, or
   duplicate-publication defect to the release gate; and
5. add newly discovered risks without renumbering existing risk identities in
   linked issues once the tracker is established.

This register does not replace customer-specific data-flow, threat-model, or
deployment risk reviews.
