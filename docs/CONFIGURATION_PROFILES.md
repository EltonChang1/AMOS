# AMOS configuration profiles

Status: **proposed baseline**

These profiles define environment classes, not deployment-specific values.
Secrets must be referenced by secret-manager identity and must never be stored
in this file, source control, logs, support bundles, or model-visible context.

## Invariants in every profile

- Configuration is validated before storage, network, model, connector, worker,
  or publication initialization.
- Tenant, identity, policy epoch, source, solution pack, and destination are
  explicit; missing production context is a hard failure.
- Model, connector, signing, and publication credentials are separate.
- Durable mutations and external publication use idempotency keys.
- Logs are structured and redacted; prompts and result bodies are opt-in data,
  not default telemetry.
- `--demo`, static bearer identities, demo keys, seeded customer-like data, and
  local publication shortcuts are prohibited outside development.

## Profile matrix

| Dimension | Development | Staging | Pilot | Production |
| --- | --- | --- | --- | --- |
| Purpose | Local implementation and regression | Production-faithful integration and recovery rehearsal | One paid design-partner shadow workflow | Approved recurring customer operation after pilot gate |
| Data | Synthetic fixtures only | Sanitized or generated production-shaped data unless customer agreement permits otherwise | Customer-approved non-production or production read-only data | Customer-approved production read-only data |
| Identity | Explicit demo identities allowed | Enterprise identity test tenant; no static identities | Customer enterprise identity | Customer enterprise identity with supported lifecycle and emergency procedure |
| Control store | SQLite allowed | PostgreSQL with forced RLS | PostgreSQL with forced RLS, HA/PITR | Qualified PostgreSQL topology with HA/PITR and tested capacity |
| Objects/queue | Local filesystem and in-process dispatcher allowed | Versioned object store and durable queue | Customer-approved versioned store and durable queue | Qualified store/queue with lifecycle, monitoring, and recovery |
| Model | Stub or configured local model | Release-candidate provider adapter | Customer-approved model inside allowed boundary | Approved, version-pinned model with change control |
| Connector | SQLite fixture | Real connector against non-production service | Certified connector against agreed source | Certified connector and supported source version |
| Workers | In-process adapters allowed | Release isolation and egress policy | Isolated container/VM workers | Qualified isolated pools with capacity and patch process |
| Secrets | Developer-local secret store; never committed | Staging secret manager | Customer-approved secret manager and rotation | Production secret manager, rotation, access audit, and recovery |
| Publication | Local hash-addressed output | Test destination only | One agreed shadow destination; review required | One approved destination with acknowledgment and retry controls |
| Outbound traffic | Developer controlled | Explicit allowlist | Customer-approved allowlist; no default model/telemetry egress | Deny by default; approved destinations only |
| Observability | Local traces/logs | Shared staging backend | Customer-visible health plus pilot alerts | SLO dashboards, paging, retention, and audit export |
| Backup/restore | Disposable | Scheduled rehearsal | Declared pilot RPO/RTO and restore drill | Contracted objectives and recurring recovery drills |
| Support | Best effort | Engineering-owned | Named on-call and escalation path | Published support policy and incident communication |

## Required configuration groups

Every non-development configuration must name and version:

1. deployment identity, environment, tenant, region, and data boundary;
2. control-store, object-store, queue, and backup policies;
3. identity issuer, audience, claims mapping, session policy, and break-glass
   procedure;
4. solution pack, metric/schema definitions, source connector, and allowed
   query shapes;
5. model provider, endpoint, model identity, limits, and egress policy;
6. worker image digest, capability issuer/audience, sandbox, resource, and
   network limits;
7. artifact templates, reviewer role, retention, and publication destination;
8. logging, metrics, traces, alert routing, and redaction policy; and
9. release version, migration version, rollback compatibility, and support
   owner.

## Promotion gates

- Development to staging: CI passes; configuration lint rejects demo leakage;
  proposed ADRs affecting the deployment are accepted.
- Staging to pilot: real connector conformance, enterprise identity, isolated
  worker, security, migration, backup/restore, and publication retry suites
  pass in a clean environment.
- Pilot to production: four scored shadow cycles pass, critical incidents are
  closed, the customer approves the operating boundary, and an explicit
  production/renewal decision is recorded.

Environment promotion copies no secrets or data. It promotes signed versions
of software, solution packs, templates, schemas, and policy; each environment
resolves its own authorized secret and service references.
