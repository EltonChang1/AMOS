# ADR 0004: Treat models as untrusted providers behind a neutral port

- Status: Proposed
- Date: 2026-08-12
- Owners: Agent and runtime lead; security owner
- Decision scope: Model integration and authority boundary

## Context

The MVP needs model planning and interpretation, but provider choice will vary
with customer hardware, policy, and model quality. A model must not become the
authority for access, calculation, evidence, or publication.

## Decision

Define a provider-neutral `ModelProvider` port for health, structured
generation, cancellation, timeout, token/latency accounting, and model identity.
Support one local or customer-approved adapter first. The model proposes typed
plans, bounded tool calls, narratives, and artifact plans. AMOS authorizes,
executes, recomputes, verifies, records, review-gates, and publishes.

Only context filtered by tenant, identity, state, effective time, type, and
sensitivity may cross the port. Provider output is untrusted input and must
validate against versioned schemas. No provider receives source or publication
credentials.

## Consequences

- Model and prompt upgrades do not change durable transaction or evidence
  contracts.
- Provider errors, malformed output, timeouts, and refusals require explicit
  terminal behavior and evaluation.
- A customer-local profile can operate without outbound model or telemetry
  traffic.

## Validation gates

- The same frozen evaluation cases run through at least two provider adapters
  without core-runtime changes.
- Malformed, injected, over-budget, and unauthorized proposals fail closed.
- Provider identity and parameters are recorded for replay and audit.
- Model replacement cannot bypass verification or review obligations.
