# AMOS product and Rust implementation requirements

> AMOS is an internally deployed analyst system that connects to company data
> and tools, answers business questions, performs verified analysis, and
> produces graphs, reports, and presentation slides.

This matrix makes the two manuscripts and `docs/PRODUCT_REQUIREMENTS.md`
executable. The product requirements define the complete analyst system. The
research paper defines the memory-operating kernel, and the design proposal
defines Specifications A-F for the control and analysis runtime.

## Target product components

| Component | Required Rust boundary |
|---|---|
| Local analyst agent | Provider-neutral `ModelProvider`; Gemma 4 adapter; structured plan, tool-call, narrative and slide-plan responses; complete model/version/configuration evidence |
| AMOS control layer | Permission-first context, task and policy enforcement, typed plans, capabilities, verification, evidence, review, replay and invalidation |
| Company connectors | Typed production adapters for warehouse, semantic, catalog, document and business-tool sources; bounded reads, health, change events and conformance tests |
| Analytical workers | Authoritative SQL, statistics and chart computation outside the model with resource and data-access limits |
| Artifact compiler | Deterministic chart, PPTX, PDF/HTML, dashboard and spreadsheet generation from verified result objects |
| Customer deployment | Internally deployable appliance with enterprise identity, secret custody, network isolation, observability, retention and support contracts |

## Research-paper kernel

| Contract | Rust implementation |
|---|---|
| Bounded active context | `context::ContextCompiler` with role coverage and token/object budgets |
| Typed persistent memory | `domain::MemoryObject`, `store::Store`, `memory::MemoryService` |
| Permission-first retrieval | `policy::PolicyEngine` before scoring and again before execution/publication |
| Effective time, authority, supersession | `memory::retrieve`, `memory::reconcile`, `memory::supersede` |
| Core primitives | `retrieve`, `write`, `supersede`, `reconcile`, `compact`, `verify`, `cite`, `replay` |
| Pre-execution verification | `verification::Verifier` over typed plans and parsed read-only SQL |
| Claim-level provenance | `evidence::EvidenceService` and typed `DependencyEdge` records |
| Durable feedback | `evidence::review` uses an idempotent atomic commit for the review, exact claim-state changes, governed feedback, audit, revalidation job, and outbox; history is immutable |
| Replay | `runtime::replay` creates a separate replay A-TXN, fenced executions, exact/equivalent/different comparisons, audit, and outbox state without changing the original |
| Explicit outcomes | `Outcome::{Pass, Warning, Repair, NeedsReview, Reject, Abort}` |

## Design-proposal production contracts

| Specification | Required Rust behavior |
|---|---|
| A - A-TXN | Generated state-transition guard, atomic admission and compare-and-swap sequencing, terminal immutability, fence-checked execution commits, expired-lease redelivery, idempotency, and same-commit outbox records |
| B - persistence | Tenant-scoped repositories, immutable versions/evidence, indexed state, checksummed migration ledger, audit history, retention/erasure proof, and hash-verified object promotion |
| C - task/context | Immutable effective task definition, consistency minima, required-first exact token budgets, permission-filtered FTS candidates, deterministic ranking trace, and explicit omissions/conflicts |
| D - connectors | Typed `discover`, `observe`, `read`, `validate`, `subscribe`, and `health` interface; capability revalidation; durable deduplicated cursor events; conformance tests |
| E - workers | Constant-time signed, audience-bound, expiring and fully invocation-bound capability claims; typed plans; frozen SQL subset; cancellation and incremental time/row/byte bounds; bounded repair |
| F - claims/review | Referential and numeric claim verification, chart-data binding, append-only idempotent corrections, independent validity dimensions, persisted replay comparisons, transitive quota-bounded invalidation continuations, and same-commit validity events |
| Minimum `/v1` API | Task lifecycle, permission-first memory search/write, SQL preflight, artifacts, claims, reviews, replay, and revalidation in `api::router` |

## Production gap traceability

| Gap-register area | Local completion evidence | Deployment-only remainder |
|---|---|---|
| Analyst agent | `TypedPlan` records a planner identity and the runtime is independent of a model SDK | Implement the `ModelProvider` contract, local Gemma 4 serving, structured outputs, failure handling and model evaluation |
| Domain configuration | Task, memory, policy and verifier concepts are typed | Remove payment-specific constants and load task, metric, schema, policy, verifier and artifact definitions from configuration |
| Artifact compiler | The chart worker produces deterministic hash-bound SVG and the runtime stores typed artifacts | Add report and slide planning plus editable PPTX, PDF/HTML, dashboard and spreadsheet renderers |
| Tenancy and identity | Every repository key and policy read is tenant scoped; source-event processing binds the authenticated tenant; missing, cross-tenant, stale-epoch, and hidden-claim tests fail closed | Enterprise OIDC/SAML issuer, JWKS, session, and PostgreSQL forced-RLS configuration require an identity tenant and database |
| Storage concurrency/migrations | Tokio-facing database work uses an eight-permit blocking lane; CAS/fence concurrency tests cover independent connections; schema v6 has a checksummed forward-only ledger, legacy backfill, and future/tampered-version rejection | PostgreSQL pool sizing, online DDL, rollback rehearsal, forced RLS, backup and point-in-time restore require a deployed cluster |
| Retrieval/context | SQLite FTS5 pushes tenant/status/type/time/all-label filters before ranking; bounded heap top-K, 2,000-candidate ceiling, exact tokenizer budget, consistency minima, ambiguity corpus, and ranking trace are tested | Million-object/noisy-neighbor qualification must run against release hardware and the selected production search service |
| Connector durability | Connector events are durable, unique by source deduplication key, cursor paged at 250, and survive connector restart; unknown cursors fail closed | Customer connector credentials, auth rotation, vendor quotas, and certified outage fixtures |
| SQL/capabilities | Frozen one-query AST subset, time-window/function/join/subquery rules, query-only SQLite, HMAC verification, full claim binding, cancellation, wall time, incremental rows and bytes | KMS/HSM key custody and container/VM worker identities and egress controls |
| Runtime recovery | `recover_task` is state driven and resumes persisted manifest/plan/execution/evidence checkpoints; the recovery test recreates the runtime at fourteen automatic and post-review boundaries | Multi-process controller election and distributed worker placement |
| Verification | Execution/verifier/memory references, numeric rates, concentration payloads, statistical constraints, and deterministic chart SVG/hash binding are independently recomputed | Configuration-driven verifier packs beyond the legacy reference fixture |
| Policy/invalidation | Policy epochs are checked on execution, commit, recovery, and reads; transitive reverse traversal has a visited set, quotas, durable cursors, idempotent revalidation jobs, and storm-bounded pages | External policy evaluator and organization-specific activation/simulation workflow |
| Replay/publication | Replay persists a new A-TXN and comparisons; filesystem objects stage, fsync, hash-check, atomically promote, and recover lost acknowledgments | S3/GCS residency/lifecycle and external destination acknowledgment/revocation adapters |
| Outbox/jobs | Leases, owner/fence checks, renewal, exponential retry, dead letter, bounded dispatch/worker batches, shutdown, expired-lease recovery, and delivery-error tests | Message-broker adapter and production alert routing |
| API/browser | Required mutation keys, 1 MiB body limit, opaque artifact cursors, request/correlation IDs, stable errors, bearer challenge, CSP, frame/referrer/content-type/cache headers, and a 3.1 contract endpoint | Cookie sessions are not used locally; an enterprise browser session adapter must add secure cookies and CSRF controls if bearer-only forms are replaced |
| Observability | Tenant-safe counters and latency buckets are exposed to administrators; tracing initialization and durable audit remain enabled | Metrics exporter, distributed trace backend, dashboards, SLO alerts, and incident paging |
| Retention/privacy | Versioned retention records, legal holds, due erasure, FTS deletion, dependent-claim invalidation/redaction, idempotent receipts, audit, and outbox proof are atomic and tested | Regional placement, cloud key destruction, legal export formats, and external deletion verification |
| Performance | `benches/control_paths.rs` generates reproducible retrieval, SQL-preflight, durable-commit, job-lease, invalidation, full-task, and replay workloads; each reports p50/p95/p99 and enforces a profile-specific p95 threshold | Final throughput/RSS/noisy-neighbor envelope must be recorded on named release hardware |

## Current reference fixture

The executable code is currently bounded to a configured tenant, one read-only
SQL source, a legacy payment-specific metric fixture, three query shapes, one
structured chart, one report template, and one reviewer role. This fixture
proves the control-layer lifecycle but is not the product definition.

The next implementation slice must replace payment-specific behavior with
configuration, integrate a local Gemma 4 analyst agent, accept a
domain-neutral business question, and produce verified graphs, a report, and an
editable slide deck.

## External boundaries

The repository supplies working SQLite warehouse/control adapters, durable
connector cursors, filesystem object promotion, static demo identity, and local
outbox destinations. Domain contracts remain independent of Axum and model
SDKs. The absence of model integration is now an open product requirement, not
an intended final boundary. Production PostgreSQL/RLS, enterprise identity,
KMS/HSM, cloud object lifecycle, customer warehouse, isolated-worker, broker,
telemetry-backend, and external-publication adapters require real
infrastructure or credentials; the table above identifies their executable
local conformance contracts.
