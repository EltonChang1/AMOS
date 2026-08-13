# ADR 0007: Package the MVP for customer-contained deployment

- Status: Proposed
- Date: 2026-08-12
- Owners: Artifacts and platform lead; security owner
- Decision scope: Installation, upgrade, and customer trust boundary

## Context

The first design partner must run AMOS inside a customer-approved boundary.
Kubernetes-first packaging would add unnecessary operational burden, while an
undocumented set of developer commands would not be supportable or recoverable.

## Decision

Ship a signed, versioned container-compose or virtual-machine appliance for the
paid pilot. The package includes the control plane, isolated workers, model
adapter, database migrations, object and queue configuration, health checks,
preflight, solution pack, observability hooks, backup/restore procedures, and
upgrade/rollback automation. Add Kubernetes packaging only when a customer
requires it or measured scaling justifies it.

Development, staging, pilot, and production are separate profiles. Demo
identity, keys, data, and publication paths are prohibited outside development.
No customer data, prompt, result, or telemetry leaves the approved boundary
unless an administrator explicitly configures the destination.

## Consequences

- Installation and recovery are product features with acceptance tests.
- Supported hardware, external dependencies, privileges, ports, retention, and
  disconnected behavior must be explicit.
- Customer-specific work belongs in configuration and solution packs, not a
  forked core runtime.

## Validation gates

- A fresh customer-controlled environment installs and runs without source
  checkout or developer intervention.
- Upgrade, rollback, backup, restore, key rotation, and uninstall rehearsals
  preserve or intentionally delete data as documented.
- Package signatures, SBOM/provenance, vulnerability scans, and configuration
  linting pass.
- A deployment journal distinguishes reusable product work from bespoke work.
