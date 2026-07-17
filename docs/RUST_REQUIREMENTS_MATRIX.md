# AMOS Rust implementation requirements

This matrix makes the two manuscripts executable. The research paper defines the memory-operating kernel; the build guide defines the first production slice, cut list, and Specifications A–F.

## Research-paper kernel

| Contract | Rust implementation |
|---|---|
| Bounded active context | `context::ContextCompiler` with role coverage and token/object budgets |
| Typed persistent memory | `domain::MemoryObject`, `store::Store`, `memory::MemoryService` |
| Permission-first retrieval | `policy::PolicyEngine` before scoring and again before execution/publication |
| Effective time, authority, supersession | `memory::retrieve`, `memory::reconcile`, `memory::supersede` |
| Core primitives | `retrieve`, `write`, `supersede`, `reconcile`, `compact`, `verify`, `cite`, `replay` |
| Publication invariants I1–I5 | Enforced across `context`, `verification`, `evidence`, and `runtime` commit paths |
| Pre-execution verification | `verification::Verifier` over typed plans and parsed read-only SQL |
| Claim-level provenance | `evidence::EvidenceService` and typed `DependencyEdge` records |
| Durable feedback | `evidence::review` appends review and governed feedback; history is immutable |
| Replay | `runtime::replay` validates retained inputs and result hashes |
| Explicit outcomes | `Outcome::{Pass, Warning, Repair, NeedsReview, Reject, Abort}` |

## Build-guide first-slice contracts

| Specification | Required Rust behavior |
|---|---|
| A – A-TXN | Generated state-transition guard, compare-and-swap sequence, terminal immutability, fenced jobs, idempotency and outbox records |
| B – persistence | Tenant-scoped repositories, immutable versions/evidence, indexed state, audit history, object finalization state |
| C – task/context | Immutable task definition, required-role coverage, policy-first selection, explicit omissions/conflicts, non-governing compaction |
| D – connectors | Typed `discover`, `observe`, `read`, `validate`, `subscribe`, and `health` interface plus conformance tests |
| E – workers | Signed, audience-bound, expiring capability claims; typed plans; read-only SQL/statistics/chart workers; bounded repair |
| F – claims/review | Structured claims, typed dependency edges, append-only corrections, independent validity dimensions, replay levels, bounded invalidation |
| Minimum `/v1` API | Task lifecycle, permission-first memory search/write, SQL preflight, artifacts, claims, reviews, replay, and revalidation in `api::router` |

## First production slice

The executable alpha is deliberately bounded to one tenant, one read-only SQL source, the payment-failure metric family, three query shapes, one structured chart, one report template, and one reviewer role. A successful run persists an admitted transaction, connector observations, context manifest, typed plan, verified executions, result/chart hashes, structured claims, dependency edges, replay package, audit trail, and explicit outcome. Approved review obligations advance through revalidation and the local publication lifecycle; corrections append governed feedback for later tasks without mutating the original artifact.

## Cut until earned

General Python workers, free-form claim extraction, vector indexes, autonomous external publication, multi-tenant SSO fleets, customer-managed keys, regional data planes, hybrid retrieval, and controlled autonomy stay out of the first slice. Production PostgreSQL/RLS, object storage, KMS/secret management, SSO, and customer warehouse connectors are deployment adapters behind existing ports, not alternate domain logic.

## External boundaries

The repository supplies a fully working local SQLite warehouse connector and durable SQLite control-plane adapter for reproducibility. The domain ports do not depend on a database, web framework, model SDK, or connector implementation, allowing PostgreSQL, object storage, SSO, and customer warehouse adapters to replace local adapters without changing the contracts. Production credentials and customer-specific source adapters cannot be embedded in the repository and remain deployment integrations, not missing domain functions.
