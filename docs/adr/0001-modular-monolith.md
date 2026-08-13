# ADR 0001: Use a modular monolith for the MVP control plane

- Status: Proposed
- Date: 2026-08-12
- Owners: Agent and runtime lead; artifacts and platform lead
- Decision scope: Deployment topology and internal component boundaries

## Context

The MVP must deliver one governed workflow with atomic lifecycle state,
evidence, review, invalidation, replay, and publication. Premature network
boundaries would add distributed failure modes before independent scaling or
ownership needs have been measured.

## Decision

Build one stateless control-plane application with typed internal modules and
one authoritative transaction database. Domain code remains independent of web
frameworks, database drivers, model SDKs, and concrete connectors. Model
serving and execution workers may run out of process where their security or
hardware boundary requires it; they communicate only through versioned ports.

Extract another service only when measurements show that worker isolation,
connector scaling, context-index capacity, invalidation load, enterprise data
planes, or independent release ownership requires it.

## Consequences

- Atomic state changes and same-commit outbox writes remain straightforward.
- Customer installation, recovery, and upgrades have fewer moving pieces.
- Internal package ownership and dependency direction must be enforced so the
  monolith does not become an unstructured shared database application.
- A later extraction must preserve the same typed and idempotent contracts.

## Validation gates

- Architecture tests prevent domain code from importing adapters.
- Each durable table and effect has one owning module.
- A clean deployment can run the complete workflow without hidden services.
- Service extraction requires a measured trigger and a superseding ADR.
