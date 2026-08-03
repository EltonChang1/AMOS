# Security Policy

AMOS is a governed control layer for enterprise AI analysis. Security reports
are treated as the highest-priority class of issue.

## Reporting a vulnerability

Please do not open a public issue for suspected vulnerabilities. Instead:

- Use GitHub's private vulnerability reporting ("Report a vulnerability" under
  the repository's Security tab), or
- Email the maintainer listed in the repository's commit history.

Include the affected version or commit, reproduction steps, and the impact you
believe the issue has. You should receive an acknowledgment within 72 hours.
Please allow up to 90 days for a coordinated fix and disclosure.

## Scope

Reports are especially valuable for defects in the security boundaries the
project explicitly claims:

- Authentication and authorization: bearer identity handling, permission-first
  memory retrieval, policy visibility, owner/role checks on API and UI routes.
- Capability tokens: signing, constant-time verification, invocation binding,
  and expiry of short-lived execution capabilities.
- SQL admission: the frozen read-only SQL subset, blocked-column enforcement,
  required metric filters, time-bound checks, and declared repairs.
- Model boundary: sanitized model payloads (no raw warehouse rows, no
  restricted memory), privacy profiles, and egress allow-lists.
- Evidence integrity: content hashes, idempotency keys, fencing tokens,
  append-only review and audit records.

## Out of scope

- The bundled `--demo` mode's static identities (`analyst_001`, `admin`, ...)
  are intentionally insecure local fixtures; deployments must supply their own
  `IdentityProvider`. Reports that only demonstrate that the demo identities
  are usable in demo mode are not vulnerabilities.
- Denial of service through locally configured resource limits.

## Supported versions

The project is pre-1.0. Only the latest commit on the default branch receives
security fixes.
