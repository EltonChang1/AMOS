# AMOS implementation readiness

Status: **control-layer reference slice and signed pack contract implemented; full analyst product not ready**

Evidence date: 2026-08-21

Release: `amos 0.2.0`

> AMOS is an internally deployed analyst system that connects to company data
> and tools, answers business questions, performs verified analysis, and
> produces graphs, reports, and presentation slides.

`docs/PRODUCT_REQUIREMENTS.md` is the canonical product target. This ledger
reports what the current Rust implementation proves against that target. A
complete local control-layer fixture is not the same as a complete customer
product.

## Completion summary

- Current control-layer fixture: all documented local tests pass.
- Product-critical gaps: local Gemma 4 agent, pack-driven runtime routing,
  report and slide planning, deterministic PPTX/PDF/spreadsheet generation,
  production connectors, enterprise deployment, and customer validation.
- Current payment-specific task, schema, metric, verifier, permissions, and UI
  are a temporary fixture and must not define the product.
- Capacity deferral: final throughput, RSS, and noisy-neighbor qualification
  must be repeated on named release hardware; current local benchmark evidence
  is deterministic and threshold-gated but is not a production capacity claim.
- Generated demo databases, objects, proxy processes, and browser state were
  isolated outside the repository and removed after the walkthrough.
- Historical Python evaluation results and scenario fixtures remain archived,
  but the `amos.evaluation` command implementations are absent from this Rust
  checkout. Those commands are not current executable evidence.
- Signed solution-pack documents now define a strict domain-neutral
  configuration boundary and include separate bank-liquidity and payment
  regression fixtures. Successful validation does not mean the legacy runtime
  is pack-driven yet.

## Signed solution-pack contract

| Promise | Implementation | Executable evidence | Status |
|---|---|---|---|
| Strict versioned workflow contract | `solution_pack::SolutionPackManifest` covers identity/effective state/owner, questions, parameters, context, sources, metrics, banking metadata, plans, verification/review, claims, outputs, publication, retention, and evaluations with unknown fields denied | `solution_pack::tests::strict_json_rejects_unknown_manifest_fields`; checked-in fixture integration test | Complete |
| Asymmetric tenant trust | Ed25519 signatures over typed manifests; trusted publisher and pack both constrain tenants; semantic core compatibility and effective/approval state gate activation | `signed_approved_pack_verifies_for_authorized_tenant`; `unsigned_tampered_incompatible_and_unauthorized_packs_fail_closed` | Complete locally |
| Read-only and ambiguity boundary | Source write capability is rejected; identifiers/references are closed; question examples and active workflow intervals cannot conflict | `ambiguous_or_write_capable_contracts_are_rejected`; `registry_rejects_overlapping_workflow_activation` | Complete locally |
| Operator validation/signing | `amosctl solution-packs validate` reports hashes and signer identity; `sign` reads and zeroizes private key material from standard input and atomically promotes output | CLI signing/validation integration tests; configured finance CI fixture gate | Complete locally |
| Bank and regression fixtures | Signed aggregate synthetic bank-liquidity pack plus separately scoped payment regression pack and development-only trust root | `checked_in_solution_packs_have_valid_tenant_scoped_signatures` | Complete as contract fixtures; not customer validation |

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
| Analyst agent | Typed plans already record `model_identity`; runtime contracts are independent of a model SDK | Implement provider-neutral model routing and customer-local Gemma 4 inference with structured plan, narrative and slide-plan outputs |
| Domain neutrality | Strict signed solution-pack contracts now type and validate task, source, metric, policy, verifier, review, artifact, publication, retention, and evaluation definitions | Persist tenant activation/version history and load runtime planning, connector, verification, composition, replay, and routing behavior from the active pack; remove payment constants from core |
| Artifact generation | Deterministic SVG chart worker, typed artifacts, claim evidence and hash-addressed local publication | Produce editable PPTX, PDF/HTML reports, dashboards and spreadsheets from verified result objects |
| Tenancy and identity | Tenant predicates and composite ownership are enforced in repositories and policy; static demo provider fails closed; 401/403 and second-analyst tests pass | Enterprise OIDC/SAML, issuer/JWKS/session lifecycle and PostgreSQL forced RLS need real tenants and infrastructure |
| Storage concurrency and migrations | Eight-permit blocking lane; independent-connection CAS tests; checksummed forward migration ledger and restart-safe backfill | PostgreSQL pool/online DDL, backup, PITR and RLS rehearsal |
| Retrieval and context | Policy filters are pushed before a bounded 2,000-candidate top-K; exact budgets and ambiguity corpus pass | Production search-service and million-object noisy-neighbor qualification |
| Connector durability | Durable deduplicated cursors, health, capability-bound reads and restart tests | Customer credentials, rotation, quotas and vendor outage certification |
| SQL and capabilities | Parser-enforced frozen subset, blocked columns, metric/time-window checks, HMAC binding, query-only driver and cancellation/limits | KMS/HSM custody and isolated remote-worker identity/egress |
| Runtime recovery | State-driven recovery resumes fourteen automatic/post-review boundaries without duplicate evidence | Multi-process controller election and distributed placement |
| Verification | Numeric rates, concentration, statistics, schema/metric references and deterministic chart hash are independently recomputed in the reference fixture | Configuration-driven verifier packs for general business analysis and real customer tasks |
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
cargo test --all-targets --all-features
cargo test --all-targets --all-features --release
cargo build --all-features --release
RUSTDOCFLAGS='-D warnings' cargo doc --no-deps --all-features
cargo audit
git diff --check
AMOS_BENCH_MEMORY_ITEMS=10000 cargo bench --bench control_paths
```

Both debug and release profiles run 83 tests (44 library, 3 deployment, 15
API/CLI/UI, 18 end-to-end runtime, and 3 solution-pack signing/fixture tests), with
zero failures or ignored tests. `--all-targets` also executes the control-path
benchmark. The 2026-08-21 release benchmark used 10,000 memory objects and
passed every threshold; the final p95 included 87.929 ms retrieval, 39.359 ms
governed task, and 33.659 ms persisted replay on this machine.

The repository pins Rust 1.97.0 in `rust-toolchain.toml`, and
`.github/workflows/ci.yml` enforces the same gates for pull requests and pushes
to `main` or `finance`. The dependency audit scanned all 171 locked crate
dependencies with no reported vulnerability on 2026-08-21. A clean
Linux/aarch64 image build retained the non-root identity and validated both
packaged signed fixtures from `/usr/share/amos/solution-packs`.

## Historical evaluation boundary

The JSON and Markdown under `artifacts/evaluation/`, the three `scenarios/`
packs, `evaluation_protocols/`, and the external-study templates are frozen or
archived research evidence. They describe an earlier Python evaluation harness
and remain useful for provenance and future study design. They do not prove
that the current Rust product implements those evaluators, scenarios, or
headline experiments.

No Python source files, package metadata, or importable `amos.evaluation`
modules exist in this checkout. Accordingly:

- Python commands in the archived evaluation guides are historical procedures,
  not supported current commands;
- scenario manifests are marked `archived_non_executable_evidence` and retain
  their former commands only as `historical_eval_commands`;
- the frozen result files may be integrity-checked where sidecar hashes exist,
  but they cannot currently be regenerated from source in this repository; and
- a new independent study must first restore or replace its validators, put
  them under CI, and freeze a newly reviewed protocol.

## Intentional boundaries and non-goals

The current implementation does not claim PostgreSQL/RLS, enterprise identity, KMS/HSM,
cloud object lifecycle, customer-warehouse credentials, isolated remote
workers, brokers, telemetry backends or external publication destinations.
Those require real deployment infrastructure and conformance evidence.

General Python/notebook execution, arbitrary production writes, unrestricted
EDA, general multi-agent scheduling, free-form authoritative claim extraction,
unreviewed causal conclusions and autonomous external communication remain
intentional non-goals. They were not added to inflate local scope.

The following are required product features, not non-goals: a bundled local
analyst agent, open-ended business questions within configured permissions,
graphs, reports, editable presentation slides, dashboards, spreadsheets, and a
supported internal customer deployment.

## Honest residual risk

SQLite and loopback filesystem behavior prove the control-layer contracts, not the
failure envelope of a distributed deployment. The static bearer identities are
demo-only. Interactive browser use requires a dedicated local header rule; the
credential must never be placed in a URL. Capacity numbers describe this local
machine and build only. These boundaries leave no known critical gap inside
the legacy reference fixture, but the full product gaps above remain open.
