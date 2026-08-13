# ADR 0006: Run tools in isolated, capability-limited workers

- Status: Proposed
- Date: 2026-08-12
- Owners: Agent and runtime lead; security owner
- Decision scope: Execution isolation and credential handling

## Context

SQL, statistics, and chart computation process customer-derived inputs and can
consume significant resources. In-process enforcement alone does not provide
the required filesystem, network, syscall, credential, or noisy-neighbor
boundary for a pilot deployment.

## Decision

Execute approved tools in separate container or VM workers. Each invocation
receives a short-lived signed capability bound to tenant, identity, A-TXN,
plan, step, tool, source, relation, operation, limits, policy epoch, expiry, and
fence. Workers have no ambient source credentials, writable host filesystem,
or unrestricted network access. They obtain narrowly scoped credentials through
an approved broker only after validating the capability.

Cancellation, wall/CPU time, memory, row, byte, concurrency, and output limits
are mandatory. Arbitrary Python is outside the MVP boundary.

## Consequences

- Worker images, signing keys, credential delivery, sandbox policy, and egress
  become security-critical release artifacts.
- Local in-process workers remain useful regression adapters but cannot prove
  production isolation.
- Lost leases and retries must be fenced and idempotent.

## Validation gates

- Rebinding or tampering with any capability field is rejected.
- Escape, egress, credential-exposure, exhaustion, cancellation, and stale-fence
  tests fail safely.
- Worker logs and outputs are bounded, classified, and redacted.
- Image provenance and dependency scanning pass the release gate.
