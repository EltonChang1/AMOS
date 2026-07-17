# AMOS: A Memory Operating Layer for Autonomous Data Analysis

AMOS is a Rust-native operating layer between analytical agents and governed data systems. It gives an agent a bounded, permission-safe view of persistent organizational memory; verifies proposed computation against current schemas, metrics, policies, and data state; executes through signed, capability-limited workers; and commits every material claim with provenance, review state, invalidation rules, and replay metadata.

This repository implements the complete first production slice defined by the accompanying papers: one tenant, one read-only warehouse connector, the payment-failure metric family, deterministic statistics and charting, governed documents, typed report claims, reviewer approval or correction, dependency invalidation, and computational replay.

- [Research paper](papers/AMOS_research_paper.pdf): the memory-operating abstraction, core primitives, reference scenario, and evaluation.
- [Build guide](papers/AMOS_design_proposal.pdf): the first production slice, cut list, build gates, and normative Specifications A–F.
- [Rust requirements matrix](docs/RUST_REQUIREMENTS_MATRIX.md): direct traceability from both papers to implementation modules.

## What the MVP provides

- **Governed memory:** typed, versioned objects with authority, effective time, permissions, supersession, provenance, and immutable source-version identity.
- **Permission-first context:** inaccessible, expired, revoked, and superseded objects are removed before ranking; required roles, conflicts, omissions, and budgets are recorded in a context manifest.
- **A-TXN runtime:** idempotent admission, explicit state transitions, compare-and-swap sequencing, atomic evidence commits, an outbox, fenced jobs, and durable audit events.
- **Safe execution:** parsed read-only SQL, schema and metric checks, blocked-column enforcement, bounded declared repairs, signed short-lived capabilities, and row/byte/time limits.
- **Evidence and review:** typed claims, dependency edges, independent validity dimensions, human approval/rejection/correction, immutable feedback history, and reviewer-approved local publication.
- **Continuous validity:** reverse dependency traversal, source-change invalidation, revalidation jobs, connector health, and level-3 computational replay with hash comparison.
- **Product surfaces:** a server-rendered Analysis Workspace, Memory Studio, Review Queue, and Operations Console—without a JavaScript runtime.

The application, CLI, API, UI rendering, persistence, connectors, workers, and tests are all written in Rust. SQLite supplies reproducible local warehouse and control-plane adapters; the domain contracts remain independent of SQLite, Axum, or any model SDK.

## Quick start

Install the stable Rust toolchain, then seed and run the local MVP:

```bash
cargo run -- seed
cargo run -- serve
```

Open [http://127.0.0.1:8000](http://127.0.0.1:8000):

- `/` — Analysis Workspace
- `/memory` — Memory Studio
- `/reviews` — Review Queue
- `/operations` — Operations Console

The demo API accepts local bearer identities `analyst_001`, `reviewer_001`, and `admin`. They exist only for local development and must be replaced by enterprise identity in deployment.

Run the reference analysis from the CLI:

```bash
cargo run -- run \
  --request "Why did payment failure rate increase over the last six hours, and should we update the executive dashboard?"
```

The run returns a context manifest, typed plan, verified executions, report, claims, dependencies, replay package, and an explicit `needs_review` outcome. Replay the resulting artifact with:

```bash
cargo run -- replay ARTIFACT_ID
```

Use a separate data root or port when needed:

```bash
cargo run -- --root /tmp/amos-demo serve --port 8080 --seed-demo
```

## HTTP contract

The versioned `/v1` API covers:

- task admission and lifecycle inspection;
- permission-first memory search, writes, and supersession;
- structured SQL preflight and referenced-version reporting;
- artifact, claim, evidence, and audit inspection;
- reviewer approval, rejection, correction, and governed feedback;
- replay and artifact dependency revalidation;
- connector health, durable jobs, and source-event processing.

Representative endpoints include `POST /v1/tasks`, `GET /v1/tasks/{id}`, `POST /v1/memory/search`, `POST /v1/verify/sql`, `GET /v1/artifacts/{id}`, `GET /v1/claims/{id}`, `POST /v1/reviews`, `POST /v1/replay/{id}`, and `POST /v1/artifacts/{id}/revalidate`.

## Verification

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo build --release
```

The Rust regression suite exercises the state machine, source-version immutability, connector conformance, capability tampering, permission filtering, SQL safety, metric predicates, schema evolution and repair, claim support, prompt injection, memory poisoning, idempotency, reviewer feedback retention, publication, invalidation, replay, API authorization, and the complete payment-failure vertical slice.

## Architecture

- `src/domain.rs` — paper-defined objects, outcomes, validity dimensions, and A-TXN states.
- `src/store.rs` — tenant-scoped SQLite persistence, CAS transitions, atomic evidence/publication commits, outbox, audit, and jobs.
- `src/memory.rs`, `src/context.rs`, `src/policy.rs` — governed memory, reconciliation, compaction, and permission-first context compilation.
- `src/connectors.rs`, `src/workers.rs` — typed connector interface and capability-bound SQL, statistics, and chart workers.
- `src/verification.rs` — SQL, schema, metric, freshness, repair, and claim-support verification.
- `src/evidence.rs`, `src/scheduler.rs` — citations, review feedback, invalidation, and fenced background work.
- `src/runtime.rs` — complete A-TXN vertical slice, publication, revalidation, and replay orchestration.
- `src/api.rs`, `src/main.rs` — Axum HTTP/UI surfaces and the command-line application.
- `tests/` — Rust unit, API, security, and end-to-end contracts.

## MVP boundary

The build guide explicitly defers general Python execution, arbitrary production writes, unrestricted notebook code, general multi-agent scheduling, unreviewed causal claims, and autonomous external communication; they are intentionally absent until named expansion gates pass. Production PostgreSQL/RLS, object storage, KMS or secret management, SSO, and customer-specific warehouse connectors are deployment adapters that require real infrastructure and credentials, not alternate application logic.

Frozen paper artifacts, evaluation JSON, and scenario fixtures remain in the repository as research evidence; no legacy Python or JavaScript application code remains.
