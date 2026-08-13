# ADR 0002: Use PostgreSQL as the authoritative production store

- Status: Proposed
- Date: 2026-08-12
- Owners: Artifacts and platform lead; security owner
- Decision scope: Durable state, tenancy, queues, audit, and recovery

## Context

SQLite proves local control contracts but does not establish multi-process
concurrency, forced tenant isolation, high availability, point-in-time
recovery, or production migration behavior.

## Decision

Use PostgreSQL for authoritative lifecycle state, metadata, evidence indexes,
audit, outbox, and job coordination. Keep large immutable artifact bodies in
versioned object storage and bind them to PostgreSQL metadata with hashes.

Every runtime table uses composite tenant keys and forced row-level security.
Runtime roles neither own tables nor hold `BYPASSRLS`; migration credentials are
separate and unavailable to the application. Repositories repeat tenant
predicates as a second fail-closed layer. Lifecycle transitions retain compare-
and-swap sequences, fencing, short transactions, and same-commit outbox writes.

## Consequences

- PostgreSQL becomes part of the pilot backup, restore, upgrade, and support
  surface.
- The SQLite adapter remains a demo and regression implementation, not
  production-readiness evidence.
- Object storage, database records, queues, and keys need one coordinated
  recovery procedure.

## Validation gates

- Cross-tenant tests fail at repository and RLS layers independently.
- Concurrent transitions, leases, and retries preserve current invariants.
- Forward and rollback-compatible migration rehearsals pass on realistic data.
- Backup/PITR and full-system restore meet the declared pilot RPO/RTO.
