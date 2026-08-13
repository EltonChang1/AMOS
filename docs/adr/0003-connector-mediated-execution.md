# ADR 0003: Mediate every enterprise source read through a connector

- Status: Proposed
- Date: 2026-08-12
- Owners: Data and connector lead; security owner
- Decision scope: Data access and source-system trust boundary

## Context

Allowing the API, model, or a generic SQL worker to open warehouse connections
would bypass source-native identity, AMOS policy, capability limits, source
version observation, and auditable failure classification.

## Decision

Every enterprise source operation must pass through the registered tenant- and
solution-pack-specific connector. Connectors implement discovery, observation,
bounded reads, revalidation, change events, cursor recovery, and health. They
preserve source-native controls and independently validate short-lived AMOS
capabilities before invoking prepared, read-only operations.

The model receives schemas and verified result objects but never database
credentials or a direct company-system client. Core runtime and workers must
not create an independent connection around the connector path.

## Consequences

- One connector selected from signed customer demand sits on the MVP critical
  path.
- Each connector requires real-service conformance evidence and an operator
  runbook; a local adapter cannot confer production status.
- Revocation, freshness, schema, cost, quota, and source-failure states become
  typed product behavior rather than model interpretation.

## Validation gates

- Credential scanning proves secrets cannot reach model, API response,
  artifact, logs, or an unisolated worker.
- Conformance tests cover permissions, drift, limits, cancellation, outage,
  cursor recovery, rotation, revocation, duplicate events, and deletion.
- Execution and pre-publication revalidation fail closed when source state or
  authority changes.
