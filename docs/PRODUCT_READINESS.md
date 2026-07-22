# AMOS local product readiness

Status: **ready for the documented local production slice**

Evidence date: 2026-07-22

Release: `amos 0.2.0`

This ledger maps the local boundary in `README.md`, the two papers, and
`docs/RUST_REQUIREMENTS_MATRIX.md` to implementation and executable evidence.
It does not qualify the deployment-only integrations listed below.

## Completion summary

- P0 local gaps: none known.
- P1 local gaps: none known.
- P2 local gaps: none known.
- P3 local deferrals: final throughput, RSS, and noisy-neighbor qualification
  must be repeated on named release hardware; current local benchmark evidence
  is deterministic and threshold-gated but is not a production capacity claim.
- Generated demo databases, objects, proxy processes, and browser state were
  isolated outside the repository and removed after the walkthrough.

## Research-kernel contract

| Promise | Implementation | Executable evidence | Status |
|---|---|---|---|
| Bounded active context | `context::ContextCompiler`; exact tokenizer, required-role coverage, object/token budgets, deterministic ranking trace | `context::tests::compiler_enforces_consistency_exact_budget_and_ranking_trace`; `complete_vertical_slice_is_review_gated_and_replayable` | Complete |
| Typed persistent memory | `domain::MemoryObject`, `store::Store`, `memory::MemoryService`; immutable source identity, authority, time, permissions, status, provenance and supersession | `store::tests::source_version_identity_is_immutable`; memory policy and FTS tests; Memory Studio browser walkthrough | Complete |
| Permission-first retrieval | SQL/FTS candidate filters precede scoring; `PolicyEngine` rechecks reads, execution and publication | `memory::tests::retrieve_filters_permissions_before_results`; `retrieval_pushes_scope_filters_into_bounded_fts_candidates`; API cross-identity tests | Complete |
| Memory primitives | `retrieve`, `write`, `supersede`, `reconcile`, `compact`, verification, citation and replay services | module tests plus `/v1/memory`, `/v1/memory/search`, `/v1/memory/{id}/supersede`; Memory Studio search and governed-note form | Complete |
| Pre-execution verification | `verification::Verifier` over parsed single-query read-only SQL, schema, metric and freshness contracts | `verifier_rejects_unsafe_queries_and_permits_only_declared_repairs`; `/v1/verify/sql` API test | Complete |
| Claim-level provenance | typed claims, execution/verifier references and `DependencyEdge` records in atomic evidence commit | complete vertical slice, API claim inspection and artifact visibility tests | Complete |
| Durable feedback | atomic idempotent review/correction, governed feedback memory, audit, job and outbox | `review_mutations_are_idempotent_and_commit_one_feedback_job_and_event`; rollback/concurrency tests; Review Queue walkthrough | Complete |
| Replay | separate replay A-TXN, new fences/executions and persisted exact/equivalent/different comparisons | replay tests; API contract; browser produced three `Exact` comparisons without changing the original | Complete |
| Explicit outcomes | typed `Outcome::{Pass, Warning, Repair, NeedsReview, Reject, Abort}` and exhaustive lifecycle state | state-machine test and complete vertical slice | Complete |

## Specifications A-F

| Specification | Local implementation and evidence | Status |
|---|---|---|
| A — A-TXN, concurrency and publication | Atomic idempotent admission, CAS state/sequence transitions, terminal immutability, fencing, checkpoints, same-commit outbox, lease recovery, separated evidence/finalization/publication states. Covered by state-machine, concurrent admission/transition, fence, expired-lease, checkpoint, crash-at-every-checkpoint and publication lost-ack tests. | Complete |
| B — persistence invariants | Tenant-scoped SQLite repositories, immutable bodies/evidence, schema-v6 checksummed forward migrations, tamper/future-version rejection, audit, retention/erasure proof, FTS deletion and hash-addressed object promotion. Covered by store migration, source identity, erasure and publication tests. | Complete locally |
| C — tasks and context | Frozen effective task definition, approved risk/tool boundaries, consistency minima, required-first budget, filtered FTS candidates, ambiguity handling, omissions/conflicts and ranking trace. Covered by context corpus tests and vertical slice. | Complete |
| D — connector contract | Typed discover/observe/read/validate/subscribe/health interface; capability revalidation; durable deduplicated 250-item cursor stream and fail-closed unknown cursor. Covered by three connector conformance tests and Operations health/source-event surfaces. | Complete locally |
| E — workers, capabilities and verification | HMAC capabilities bind issuer, audience, tenant, subject, A-TXN, plan, step, source, relation, operation, limits, epoch and fence; query-only parsed SQL; deterministic stats/chart workers; cancellation and incremental time/row/byte limits. Covered by capability rebinding/tamper, SQL and worker limit tests. | Complete locally |
| F — claims, review, invalidation and replay | Referential/numeric/chart support, append-only approval/rejection/correction, independent validity dimensions, quota-bounded transitive invalidation and durable continuation/revalidation jobs, level-3 replay comparison evidence. Covered by evidence, invalidation continuation, source-change, review and replay tests. | Complete |

## Production-gap register

| Area | Local completion evidence | Deployment-only remainder |
|---|---|---|
| Tenancy and identity | Tenant predicates and composite ownership are enforced in repositories and policy; static demo provider fails closed; 401/403 and second-analyst tests pass | Enterprise OIDC/SAML, issuer/JWKS/session lifecycle and PostgreSQL forced RLS need real tenants and infrastructure |
| Storage concurrency and migrations | Eight-permit blocking lane; independent-connection CAS tests; checksummed forward migration ledger and restart-safe backfill | PostgreSQL pool/online DDL, backup, PITR and RLS rehearsal |
| Retrieval and context | Policy filters are pushed before a bounded 2,000-candidate top-K; exact budgets and ambiguity corpus pass | Production search-service and million-object noisy-neighbor qualification |
| Connector durability | Durable deduplicated cursors, health, capability-bound reads and restart tests | Customer credentials, rotation, quotas and vendor outage certification |
| SQL and capabilities | Parser-enforced frozen subset, blocked columns, metric/time-window checks, HMAC binding, query-only driver and cancellation/limits | KMS/HSM custody and isolated remote-worker identity/egress |
| Runtime recovery | State-driven recovery resumes fourteen automatic/post-review boundaries without duplicate evidence | Multi-process controller election and distributed placement |
| Verification | Numeric rates, concentration, statistics, schema/metric references and deterministic chart hash are independently recomputed | Additional domain verifier packs beyond payment health |
| Policy and invalidation | Independent dimensions, bounded reverse traversal, visited set, quotas, durable cursors and idempotent revalidation | External policy evaluator and organization activation workflow |
| Replay and publication | Separate replay A-TXN/comparisons; staged, fsynced, hash-checked and atomically promoted filesystem objects | S3/GCS lifecycle/residency and external destination acknowledgments |
| Outbox and jobs | Lease owner/fence checks, renewal, retry/backoff, dead letter, bounded workers and recovery | Broker adapter, production alert routing |
| API and browser | Complete enumerated OpenAPI 3.1 paths; 1 MiB limit, stable errors, bearer challenge, request/correlation IDs, CSP/frame/referrer/nosniff/no-store headers; four responsive server-rendered surfaces | Enterprise cookie sessions would require secure cookies and CSRF; local product remains bearer-only |
| Observability | Admin-only tenant-safe counters, latency buckets, connector health, audit, jobs and outbox UI/API | Exporter, trace backend, dashboards, SLO paging |
| Retention and privacy | Versioned retention/legal hold, due erasure, dependent-claim revocation/redaction, receipts, audit and outbox in one tenant-safe commit; UI/API controls | Regional placement, cloud-key destruction, legal export and external deletion confirmation |
| Performance | Release benchmark at 10,000 memory items: retrieval p50/p95/p99 41.556/42.567/43.597 ms; governed task 22.968/30.148/30.148 ms; replay 21.082/26.145/26.145 ms; every p95 below its gate | Named release-hardware throughput/RSS/noisy-neighbor envelope |

## `/v1`, CLI and product surfaces

- `tests/rust_api.rs::openapi_documents_every_versioned_route_and_public_security_boundary`
  enumerates every routed `/v1` path, requires operation IDs/responses, and
  proves only the OpenAPI document is public.
- Task, replay, review, job, retention and erasure commands use explicit
  idempotency keys. The CLI now requires `--idempotency-key` for `run` and its
  regression test proves a retry returns the same artifact with unchanged
  audit/outbox counts.
- Analysis Workspace exposes identity, admitted lifecycle/outcome, context
  budget, selected objects, plan/execution/evidence counts, typed claim
  validity, replay and policy-visible history.
- Memory Studio exposes policy-visible type/status/version/authority/effective
  time/source-version/provenance, permission-first search and a constrained
  non-governing user-note write.
- Review Queue exposes claims, support counts/hash and explicitly confirmed
  approve, reject and structured append-only correction controls.
- Operations Console exposes connector health, task/recovery metrics, jobs,
  outbox state, audit, source-event processing, retention/legal hold and due
  erasure controls. Analyst access returns 403.

## Clean-root walkthrough evidence

The evaluator workflow was run against a new temporary data root and a
configurable loopback port:

1. Running the bundled binary without `--demo` returned a validation failure
   before storage initialization; explicit demo seed/start succeeded.
2. `/health` and `/v1/openapi.json` returned 200 without credentials; missing
   and unknown credentials returned 401; a known analyst on admin metrics
   returned 403.
3. The browser attached the bearer header at the loopback origin. No identity
   was accepted from URLs or forms.
4. Analyst submission produced `NeedsReview`, 414 exact context tokens, six
   selected objects, three typed steps, three fenced executions, ten dependency
   edges, four typed claims and level-3 replay evidence.
5. Browser replay created a new A-TXN and three new executions; all comparisons
   were `Exact` and the original remained unchanged.
6. Review Queue displayed four claims and ten edges. Reviewer approval required
   an explicit confirmation and reason, appended a durable review, advanced the
   original lifecycle to `Published`, and set publication validity to
   `ValidAtPublication`.
7. Memory Studio displayed reviewer feedback as a new active
   `ReviewerApproved` version with artifact provenance; permission-first search
   returned only policy-visible memory.
8. Operations displayed healthy connector state, the revalidation job, ready
   outbox events including review/publication, and audit entries for evidence,
   replay, review and local publication.
9. Desktop and 390-by-844 browser checks retained semantic navigation and form
   labels. A discovered long-ID overflow was fixed; measured client and scroll
   width are both 390 px after the correction.
10. Security headers on authenticated HTML were observed: CSP, `DENY` framing,
    `nosniff`, no-referrer and no-store, plus request/correlation IDs.

Executable tests additionally cover the source/schema invalidation and durable
continuation/revalidation workflow, crash/restart recovery, stale fences and
epochs, lost acknowledgments, dead letters, legal hold/erasure, policy-hidden
claims and second-analyst isolation without relying on manual state changes.

## Release gate

The following commands pass from the current checkout:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --release
cargo build --release
cargo doc --no-deps
git diff --check
AMOS_BENCH_MEMORY_ITEMS=10000 cargo bench --bench control_paths
```

Both debug and release profiles run 64 tests (31 library, 15 API/CLI/UI and 18
end-to-end runtime tests), with zero failures or ignored tests. `--all-targets`
also executes the control-path benchmark. The release benchmark used 10,000
memory objects and passed every threshold.

## Intentional boundaries and non-goals

The local product does not claim PostgreSQL/RLS, enterprise identity, KMS/HSM,
cloud object lifecycle, customer-warehouse credentials, isolated remote
workers, brokers, telemetry backends or external publication destinations.
Those require real deployment infrastructure and conformance evidence.

General Python/notebook execution, arbitrary production writes, unrestricted
EDA, general multi-agent scheduling, free-form authoritative claim extraction,
unreviewed causal conclusions and autonomous external communication remain
intentional non-goals. They were not added to inflate local scope.

## Honest residual risk

SQLite and loopback filesystem behavior prove the local contracts, not the
failure envelope of a distributed deployment. The static bearer identities are
demo-only. Interactive browser use requires a dedicated local header rule; the
credential must never be placed in a URL. Capacity numbers describe this local
machine and build only. These boundaries are explicit and do not leave a known
P0, P1 or P2 gap inside the promised local slice.
