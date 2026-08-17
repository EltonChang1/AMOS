# Pre-access data-flow and security review

Status: **mandatory template; no credentials or customer data before approval**

Complete this review for the exact proposed pilot. “Planned” is not an
implemented control. Unresolved critical items block access; unknown answers do
not default to approval.

## 1. Review control

| Field | Value |
| --- | --- |
| Account/workflow ID | TBD |
| Architecture/configuration version | TBD |
| Review date and expiry | TBD |
| Customer data owner | TBD |
| Customer security/privacy approver | TBD |
| AMOS security owner | TBD |
| Legal/compliance review required | Yes / No / Unknown |
| Final decision | Approved / Approved with conditions / Blocked |

## 2. Intended use and human authority

- Exact business decision and audience:
- Allowed questions and scheduled trigger:
- Decisions AMOS may inform but not make:
- Reviewer role, authority, and service window:
- Claims that always require review:
- Prohibited uses and downstream actions:
- Stop/suspend authority for both parties:

## 3. Data inventory and classification

| Data/object | System of record | Classification | Allowed fields/rows | Prohibited fields/rows | Purpose | Retention | Owner |
| --- | --- | --- | --- | --- | --- | --- | --- |
| TBD | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

Explicitly evaluate personal data, credentials/secrets, payment-card data,
protected health information, financial records, employment data, biometrics,
children's data, export-controlled data, customer confidential data, regulated
decisions, contractual restrictions, and data licensed from third parties.

Prohibited data must be denied at the source and AMOS layers where feasible,
not removed only after model generation.

## 4. System and trust-boundary inventory

| Component | Owner/operator | Location/region | Identity used | Data received/stored | Network path | Logs/telemetry | Approved? |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Employee/browser | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| AMOS control plane | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Identity provider | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Model provider/service | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Source connector/warehouse | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| SQL/statistics/chart workers | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| PostgreSQL/object store/queue | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Monitoring/support systems | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Publication destination | TBD | TBD | TBD | TBD | TBD | TBD | TBD |
| Backup/restore systems | TBD | TBD | TBD | TBD | TBD | TBD | TBD |

Attach a reviewed data-flow diagram showing request, identity, context, plan,
capability, source query, result, model interpretation, evidence, artifact,
review, publication, telemetry, backup, support, and deletion flows. Mark every
boundary where operator, legal entity, region, identity, or encryption context
changes.

## 5. Required control checks

| Control | Required evidence | Status | Owner/action/date |
| --- | --- | --- | --- |
| Purpose and scope | SOW boundaries, prohibited uses, approved users/audience | Unknown | TBD |
| Enterprise identity | Issuer/audience validation, role mapping, lifecycle, revocation, break glass | Unknown | TBD |
| Tenant isolation | Composite tenant keys, forced RLS, repository checks, object/cache/queue scoping | Unknown | TBD |
| Source-native authorization | Named read-only identity/delegation, row/column/role controls, revocation | Unknown | TBD |
| Connector enforcement | All reads use capability-checked connector; no ambient bypass | Unknown | TBD |
| Query/tool restrictions | Prepared read-only operations, allowlists, cost/row/byte/time/concurrency limits | Unknown | TBD |
| Model boundary | Approved context classes, no credentials, provider retention/training/telemetry terms | Unknown | TBD |
| Worker isolation | Image digest, sandbox, no ambient secrets, deny-by-default egress, resource limits | Unknown | TBD |
| Secrets/keys | Approved manager, separate identities, rotation, access audit, recovery | Unknown | TBD |
| Encryption | In transit, at rest, backup, key owner/region/rotation | Unknown | TBD |
| Data minimization | Raw-row limits, field masking, aggregation, blocked columns, output controls | Unknown | TBD |
| Logging and audit | Events retained, prompt/result policy, redaction, access, integrity, customer export | Unknown | TBD |
| Evidence and review | Claim-level support, immutable originals, correction path, publication gate | Unknown | TBD |
| Publication | Approved destination, pre-publish revalidation, idempotency, acknowledgment, retry | Unknown | TBD |
| Retention/deletion | Per-object periods, legal hold, backups, logs, derived artifacts, confirmation | Unknown | TBD |
| Vulnerability management | Dependency/image scan, disclosure route, severity/response targets, patch owner | Unknown | TBD |
| Incident response | Contacts, classification, containment, evidence, notification, exercises | Unknown | TBD |
| Backup/recovery | Scope, RPO/RTO, restore evidence, key/config/source recovery | Unknown | TBD |
| Support access | Named support identities, approval, session logging, time bounds, no copied data | Unknown | TBD |
| Subprocessors | Complete list, purpose/location/terms or explicit none | Unknown | TBD |
| Offboarding | Revoke access, export, delete, verify, retain minimum lawful audit proof | Unknown | TBD |

## 6. Model and prompt-specific review

- Provider, model/version, endpoint, and operator:
- Customer data training/fine-tuning use: Prohibited / Approved with terms
- Prompt/result/provider-log retention and deletion:
- Outbound network and telemetry destinations:
- Context filtering by tenant, identity, status, effective time, type, and
  sensitivity:
- Prompt-injection treatment for documents, catalog text, and source values:
- Structured-output validation and malformed-output behavior:
- Hallucination/unsupported-claim controls and human review:
- Provider outage, model change, rollback, and replay behavior:

## 7. Abuse and failure cases

Record test/evidence for:

- cross-user and cross-tenant access;
- revoked identity or source permission;
- direct and indirect prompt injection;
- malicious or superseded governing definitions;
- sensitive-column inference or display;
- query cost explosion, timeout, cancellation, and worker exhaustion;
- stale schema, metric, freshness, policy, or source state;
- malformed model output and unsupported prose;
- lost job lease, stale fence, crash, and duplicate source event;
- lost publication acknowledgment and duplicate retry;
- retention, legal hold, deletion, and backup restoration; and
- source, model, AMOS, or destination outage classification.

## 8. Residual risks and customer acceptance

| Risk | Likelihood/impact | Existing control | Residual exposure | Accepting authority | Expiry/review trigger |
| --- | --- | --- | --- | --- | --- |
| TBD | TBD | TBD | TBD | TBD | TBD |

Never describe a customer acceptance as eliminating the risk or as a general
compliance certification.

## 9. Access prerequisites

All must be true before credentials or customer data are received:

- [ ] Executed agreement, SOW, and required data/security terms exist.
- [ ] Intended use, prohibited use, source, fields, users, reviewer, model,
      destination, retention, and region are approved.
- [ ] Named least-privilege identities and revocation owners exist.
- [ ] Credential exchange uses the approved secret manager; no email, chat,
      ticket, repository, prompt, or support document contains a secret.
- [ ] Connectivity and worker egress are deny-by-default and tested.
- [ ] Logging/redaction, incident notification, deletion, and offboarding are
      testable and owned.
- [ ] No unresolved critical security, privacy, permission, destructive-action,
      data-loss, or duplicate-publication issue remains.
- [ ] A sanitized fixture and sealed references can be produced without copying
      prohibited data into the repository.

## 10. Decision and conditions

- Decision:
- Approved data/access start date:
- Conditions and deadlines:
- Compensating controls:
- Review expiry or change triggers:
- Customer approver/date:
- AMOS security owner/date:

Re-review is mandatory when source, schema, metric, data class, identity,
provider, model, region, worker image, storage, destination, retention,
subprocessor, or use changes.

Reference framework: the
[NIST AI Risk Management Framework](https://www.nist.gov/itl/ai-risk-management-framework)'s
govern/map/measure/manage approach emphasizes documented use context, human
roles, risk tolerances, deployment-relevant evaluation, privacy, security, and
ongoing monitoring. This template is an AMOS operating aid, not evidence of
NIST endorsement or compliance.
