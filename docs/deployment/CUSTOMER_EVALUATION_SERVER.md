# Install the AMOS customer-evaluation server

Status: **installable evaluation application; not the production pilot topology**

This package runs the current governed payment reference workflow on a
customer-controlled server. It is appropriate for product evaluation with
synthetic/reference data. It must not receive customer credentials or customer
data until the workflow-specific data-flow/security review passes and the
missing production integrations are implemented and qualified.

## What is included

- the Rust `amos` HTTP/API/UI server;
- a separate non-root toolbox image with pinned Spark, R, pandas, Polars,
  DuckDB, scikit-learn, XLSX, PPTX, dbt-manifest, and notebook-inspection
  runtimes;
- the `amosctl` validation, preflight, initialization, status, token-hashing,
  and health commands;
- a Linux OCI image running as an unprivileged user;
- Docker Compose installation with read-only root filesystems, all Linux
  capabilities dropped, no-new-privileges, PID/CPU/memory limits, bounded logs,
  health checks, graceful SIGTERM shutdown, and a persistent data volume;
- per-install capability signing key and hashed bearer-token manifest;
- explicit initialization of the payment reference fixture; and
- diagnostics and stopped-service consistent backup scripts.

## Important boundary

The installed server still uses:

- SQLite for control state and the reference warehouse;
- static bearer identities whose token hashes are loaded from a mounted secret;
- the payment-specific reference workflow;
- an in-process source-connected SQLite SQL worker, one shared isolated
  evaluation toolbox, and local filesystem object storage; and
- no embedded/local analyst model or customer-branded report compiler.

The configuration requires
`acknowledge_local_reference_adapters: true` so an operator cannot accidentally
label this topology production. PostgreSQL with forced RLS, enterprise OIDC,
customer source identity, one certified production connector, per-risk worker pools,
a separate model server, object storage, durable queues, restore/rollback, and
security qualification remain release gates.

## Server requirements

- 64-bit Linux supported by the official Rust/Debian container images;
- Docker Engine with the Compose v2 plugin;
- OpenSSL for installation-time secret generation;
- at least 4 logical CPUs, 6 GiB memory, and 15 GiB free storage for evaluation;
- an HTTPS reverse proxy or customer ingress terminating TLS; and
- outbound registry access while building, unless the image is provided through
  the customer's approved registry.

The Compose service publishes only to `127.0.0.1:8000` by default. Keep that
default when a reverse proxy runs on the same server. Do not expose raw HTTP on
an untrusted network.

## Install

From a reviewed release checkout on the client server:

```bash
cd deploy/compose
export AMOS_PUBLIC_BASE_URL=https://amos.customer.example
./install.sh
```

The installer:

1. refuses to replace existing configuration or credentials;
2. generates a 256-bit capability key and independent analyst, reviewer, and
   administrator bearer tokens;
3. stores only SHA-256 token hashes in the server identity manifest;
4. builds the locked Rust application and analyst-toolbox OCI images;
5. validates mounted configuration and secrets;
6. explicitly initializes the reference fixture in the persistent volume;
7. starts the service and waits for health; and
8. writes the initial plaintext tokens to
   `deploy/compose/secrets/initial-tokens.txt` with mode `0600`.

Move each token to the customer's approved secret manager, distribute it only
to its intended operator, then securely delete `initial-tokens.txt`. Never put a
token in a URL, repository, ticket, prompt, log, or support bundle.

If installation is interrupted after credentials are generated, inspect the
generated configuration and service logs, correct the underlying problem, and
resume without rotating or replacing credentials:

```bash
./install.sh --resume
```

Resume validates the existing files, preserves every secret, skips bootstrap
only when the existing data volume passes initialized preflight, and waits for
the service to become healthy. A normal rerun still refuses existing state.

## Configure ingress and DNS

Terminate TLS at the customer's ingress and proxy to
`http://127.0.0.1:8000`. Preserve the `Host` header and request/correlation
headers. Apply the customer's request-size, access-log redaction, connection,
timeout, and IP/network policies.

`public_base_url` must be HTTPS even though the internal container listener is
HTTP. The current server does not consume forwarded identity headers; it
requires an AMOS bearer token. Do not configure a proxy to trust an
internet-supplied `Authorization` or identity header.

## Verify and use the API

Check the public liveness endpoint:

```bash
curl --fail --silent https://amos.customer.example/health
```

Run an authenticated reference task with the analyst token:

```bash
curl --fail --silent \
  -H "Authorization: Bearer $AMOS_ANALYST_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{
    "request": "Why did payment failures increase?",
    "idempotency_key": "customer-evaluation-001"
  }' \
  https://amos.customer.example/v1/tasks
```

The server-rendered UI uses the same bearer authentication. For an evaluation,
use a customer-managed browser profile or trusted same-origin gateway that
attaches the token without exposing it to URLs or page scripts. Enterprise
browser sessions, OIDC login, CSRF controls, logout, and session revocation are
not yet implemented; do not present the static-token UI as production SSO.

## Operations

From `deploy/compose`:

```bash
./diagnose.sh
docker compose logs --tail=200 amos toolbox
./backup.sh
docker compose restart amos
```

`diagnose.sh` validates configuration/secrets, checks initialized files,
reports the control schema version and local-adapter boundary, shows Compose
state, calls both health endpoints, and executes every toolbox contract through
a newly signed, fully bound smoke-test capability.

`backup.sh` stops AMOS, archives the entire persistent data volume, and restarts
the service if it was running. The resulting archive is mode `0600` under
`deploy/compose/backups/`. Treat it as customer confidential because it contains
the control database, reference warehouse, and artifacts. A supported restore,
upgrade/rollback, audit export, and deletion-verification workflow is not yet
implemented; do not use this package where those are contractual requirements.

## Configuration and secret files

| Host path | Container path | Purpose |
| --- | --- | --- |
| `config/server.json` | `/etc/amos/server.json` | Reviewed non-secret server configuration |
| `secrets/capability-key` | `/run/secrets/capability_key` | 256-bit capability signing key |
| `secrets/identities.json` | `/run/secrets/identities` | Hashed tokens and scoped identities |
| Docker volume `amos-data` | `/var/lib/amos` | Databases and object bodies |
| `backups/` | Host only | Operator-created stopped-service archives streamed from the data volume |

Generated configuration, secrets, and backups are excluded from Git. Compose
mounts configuration and secrets read-only. The `secrets/` directory is mode
`0700`; its file-backed Compose secrets are readable inside the container but
cannot be traversed by other host users. The plaintext initial-token file
remains mode `0600` and is never mounted into the service.

## Stop and offboard

Stop without deleting data:

```bash
docker compose down
```

Deleting the `amos-data` volume is irreversible and is intentionally not
automated. Before deletion, confirm the customer export, retention, legal-hold,
backup, credential-revocation, and deletion-evidence requirements. The current
evaluation package does not constitute verified production offboarding.
