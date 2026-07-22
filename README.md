# AMOS: A Memory Operating Layer for Autonomous Data Analysis

AMOS is a Rust-native operating layer between analytical agents and governed data systems. It gives an agent a bounded, permission-safe view of persistent organizational memory; verifies proposed computation against current schemas, metrics, policies, and data state; executes through signed, capability-limited workers; and commits every material claim with provenance, review state, invalidation rules, and replay metadata.

This repository implements the complete local production slice defined by the accompanying papers: one configured tenant, one read-only warehouse connector, the payment-failure metric family, deterministic statistics and charting, governed documents, typed report claims, reviewer approval or correction, transitive dependency invalidation, durable replay evidence, crash recovery, and local object publication.

- [Research paper](papers/AMOS_research_paper.pdf): the memory-operating abstraction, core primitives, reference scenario, and evaluation.
- [Design proposal](papers/AMOS_design_proposal.pdf): the product architecture and normative Specifications A–F.
- [Rust requirements matrix](docs/RUST_REQUIREMENTS_MATRIX.md): direct traceability from both papers to implementation modules.

## What the MVP provides

- **Governed memory:** typed, versioned objects with authority, effective time, permissions, supersession, provenance, and immutable source-version identity.
- **Permission-first context:** indexed tenant/type/status/time/label filtering happens before bounded top-K ranking; consistency minima, exact lexical token accounting, role coverage, ambiguity, omissions, and a ranking trace are frozen in the context manifest.
- **A-TXN runtime:** atomic idempotent admission, explicit compare-and-swap transitions, same-commit outbox records, fence-checked execution commits, atomic evidence commits, leased jobs, and durable audit events. A state-driven controller resumes every automatic lifecycle boundary after process loss.
- **Safe execution:** a frozen parsed SQL subset, schema and metric checks, blocked-column enforcement, bounded declared repairs, constant-time verified short-lived capabilities with full invocation binding, driver cancellation, and incremental row/byte/time limits.
- **Evidence and review:** typed claims, dependency edges, queryable independent validity dimensions, idempotent human approval/rejection/correction, atomic append-only feedback commits, and reviewer-approved local publication.
- **Continuous validity:** quota-bounded transitive invalidation, durable continuation/revalidation workers, dimension-specific outbox events, durable connector event cursors, and level-3 replay with new fenced executions, comparison evidence, audit, and outbox state.
- **Operations and privacy:** a checksummed forward-only migration ledger, leased outbox dispatch with retry/dead-letter behavior, tenant-safe metrics, request/correlation IDs, security headers, legal holds, due erasure with claim revocation, and audit proof.
- **Publication:** hash-addressed filesystem staging and atomic promotion are idempotent across lost acknowledgments; destination-specific cloud adapters remain deployment integrations.
- **Product surfaces:** a server-rendered Analysis Workspace, Memory Studio, Review Queue, and Operations Console—without a JavaScript runtime.

The application, CLI, API, UI rendering, persistence, connectors, workers, and tests are all written in Rust. SQLite supplies reproducible local warehouse and control-plane adapters; the domain contracts remain independent of SQLite, Axum, or any model SDK.

## Quick start

Install the stable Rust toolchain, then seed and run the local MVP:

```bash
cargo run -- --demo seed
cargo run -- --demo serve
```

The bundled binary is fail-closed unless `--demo` (or `AMOS_DEMO=true`) is
explicitly set. The demo uses a named demo signing key and static local
identities; embedding applications must construct `RuntimeConfig::new` with
their own capability secret and pass an `IdentityProvider` to `api::router`.

Every API and UI route except `/health` and `/v1/openapi.json` requires an explicit bearer identity.
For example:

```bash
curl -H 'Authorization: Bearer analyst_001' http://127.0.0.1:8000/
curl -H 'Authorization: Bearer analyst_001' http://127.0.0.1:8000/v1/memory
```

The authenticated product surfaces are:

- `/` — Analysis Workspace
- `/memory` — Memory Studio
- `/reviews` — Review Queue
- `/operations` — Operations Console

For an interactive browser walkthrough, configure a dedicated local browser
profile or request-header rule to attach `Authorization: Bearer IDENTITY` to
`http://127.0.0.1:8000/*`. Keep the credential in the header: never place it in
a URL. Same-origin, server-rendered forms then preserve that header through the
analysis, memory-search/write, replay, review/correction, source-event, and
retention workflows. Switch the rule between `analyst_001`, `reviewer_001`, and
`admin` when demonstrating role boundaries. The application intentionally does
not create a weaker demo cookie or accept identity fields from forms.

The explicit demo mode accepts local bearer identities `analyst_001`,
`analyst_002`, `reviewer_001`, and `admin`. They exist only for local
development and must be replaced by an enterprise identity provider in an
embedding deployment. Missing, malformed, and unknown credentials return
`401 UNAUTHENTICATED`; authenticated identities that lack authority return
`403 PERMISSION_DENIED`.

Run the reference analysis from the CLI:

```bash
cargo run -- --demo run \
  --idempotency-key payment-health-001 \
  --request "Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?"
```

The run returns a context manifest, typed plan, verified executions, report, claims, dependencies, replay package, and an explicit `needs_review` outcome. Replay the resulting artifact with:

```bash
cargo run -- --demo replay ARTIFACT_ID --idempotency-key replay-001
```

Use a separate data root or port when needed:

```bash
cargo run -- --demo --root /tmp/amos-demo serve --port 8080 --seed-demo
```

## HTTP contract

The versioned `/v1` API has a 1 MiB request limit, explicit mutation keys, stable error envelopes, request/correlation headers, and fail-closed authentication. It covers:

- task admission and lifecycle inspection;
- permission-first memory search, writes, and supersession;
- structured SQL preflight and referenced-version reporting;
- artifact, claim, evidence, and audit inspection;
- reviewer approval, rejection, correction, and governed feedback;
- replay and artifact dependency revalidation;
- connector health, durable jobs, source-event processing, operations metrics, retention, legal hold, and erasure.

Representative endpoints include `POST /v1/tasks`, `GET /v1/tasks/{id}`, `GET /v1/artifacts/page`, `POST /v1/memory/search`, `POST /v1/verify/sql`, `GET /v1/artifacts/{id}`, `GET /v1/claims/{id}`, `POST /v1/reviews`, `POST /v1/replay/{id}`, `POST /v1/retention`, and `POST /v1/retention/memory/{id}/erase`. The machine-readable contract is at `/v1/openapi.json`.

Task, replay, review, job, retention, and erasure mutations require an
`idempotency_key`. Repeating the same command returns the original effect and
creates no duplicate durable side effects; reusing a key for different
content returns `409 IDEMPOTENCY_CONFLICT`.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --release
cargo build --release
cargo doc --no-deps
AMOS_BENCH_MEMORY_ITEMS=10000 cargo bench --bench control_paths
```

The Rust regression suite additionally exercises crash-at-every-checkpoint recovery, stale fences and policy epochs, migration tampering, cancellation/timeouts, incremental byte limits, outbox retry/dead-letter recovery, durable connector cursors, object-promotion lost acknowledgments, legal hold/erasure, opaque cursor pagination, request limits, and browser security headers.

The capacity executable reports and gates p50/p95/p99 for indexed retrieval,
SQL preflight, durable commits, job lease/complete, claim invalidation, a full
governed task, and persisted computational replay. Set
`AMOS_BENCH_CONTROL_ITERATIONS` to increase the default control-path sample
count.

## Architecture

- `src/domain.rs` — paper-defined objects, outcomes, validity dimensions, and A-TXN states.
- `src/store.rs` — tenant-scoped SQLite persistence, CAS transitions, atomic evidence/review/publication/validity commits, outbox, audit, and jobs.
- `src/memory.rs`, `src/context.rs`, `src/policy.rs` — governed memory, reconciliation, compaction, and permission-first context compilation.
- `src/connectors.rs`, `src/workers.rs` — typed connector interface and capability-bound SQL, statistics, and chart workers.
- `src/verification.rs` — SQL, schema, metric, freshness, repair, and claim-support verification.
- `src/evidence.rs`, `src/scheduler.rs` — citations, review feedback, invalidation, and fenced background work.
- `src/publication.rs`, `src/observability.rs` — hash-checked local object promotion and tenant-safe operational metrics.
- `src/runtime.rs` — complete A-TXN vertical slice, publication, revalidation, and replay orchestration.
- `src/api.rs`, `src/main.rs` — Axum HTTP/UI surfaces and the command-line application.
- `tests/` — Rust unit, API, security, and end-to-end contracts.

## MVP boundary

The design proposal explicitly defers general Python execution, arbitrary production writes, unrestricted notebook code, general multi-agent scheduling, unreviewed causal claims, and autonomous external communication; they are intentionally absent.

The local implementation supplies complete contracts and executable adapters for SQLite, filesystem object promotion, static demo identity, and local dispatch. The following release integrations genuinely require deployment infrastructure or credentials and are not represented as completed here: PostgreSQL forced RLS and backup/restore, enterprise OIDC/SAML validation, KMS/HSM-backed signing-key rotation, S3/GCS regional lifecycle controls, customer warehouse credentials, container/VM worker sandboxing and egress policy, and external publication destinations. Their conformance targets are the same tenant, capability, fence, hash, idempotency, acknowledgment, and audit contracts tested by the local adapters.

Frozen paper artifacts, evaluation JSON, and scenario fixtures remain in the repository as research evidence; no legacy Python or JavaScript application code remains.
