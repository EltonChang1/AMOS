# AMOS architecture decision records

Architecture decision records document changes to trust boundaries, durable
contracts, deployment topology, or MVP scope. `Proposed` records require founder
review before implementation relies on them; accepted records are immutable
apart from status and links to superseding decisions.

| ADR | Decision | Status |
| --- | --- | --- |
| [0001](0001-modular-monolith.md) | Modular monolith control plane | Proposed |
| [0002](0002-postgresql-authoritative-store.md) | PostgreSQL authoritative store | Proposed |
| [0003](0003-connector-mediated-execution.md) | Connector-mediated source execution | Proposed |
| [0004](0004-model-provider-boundary.md) | Provider-neutral, untrusted model boundary | Proposed |
| [0005](0005-verified-result-artifact-ir.md) | Verified-result artifact intermediate representation | Proposed |
| [0006](0006-isolated-capability-workers.md) | Isolated capability-limited workers | Proposed |
| [0007](0007-pilot-deployment-packaging.md) | Customer-contained pilot packaging | Proposed |

## Record format

Each record includes context, decision, consequences, and validation gates. A
new record supersedes an accepted decision; editing history out of an accepted
record is not permitted.
