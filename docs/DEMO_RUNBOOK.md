# AMOS private, auditable analyst demo runbook

This runbook is the release and recording contract for the subscription-churn
vertical slice. The hosted configuration makes one qualified claim:
customer-controlled AMOS with governed access to an approved hosted Gemma API.
It does not claim that the hosted configuration is air-gapped.

## Configure

```bash
cp .env.example .env
# Set GEMINI_API_KEY in .env, or remove it and set GEMINI_API_KEY_FILE.
```

The expected non-secret configuration is:

```text
AMOS_MODEL_PROVIDER=gemma_api
AMOS_MODEL_NAME=gemma-4-26b-a4b-it
AMOS_MODEL_BASE_URL=https://generativelanguage.googleapis.com/v1beta
AMOS_MODEL_ROUTE_CLASS=approved_hosted_api
AMOS_PRIVACY_PROFILE=approved_api
AMOS_ALLOWED_EGRESS_HOSTS=generativelanguage.googleapis.com
AMOS_EXTERNAL_TELEMETRY=false
```

On macOS, prefer a local Keychain item over a plaintext `.env` credential:

```bash
# Prompts twice with hidden input; the credential is never a command argument.
security add-generic-password \
  -U -a amos-demo -s AMOS_GEMINI_API_KEY \
  -l 'AMOS local Gemini API key' \
  -j 'Local-only AMOS credential; never commit' \
  -w

# Inject only into the current shell or a single child process.
export GEMINI_API_KEY="$(
  security find-generic-password \
    -a amos-demo -s AMOS_GEMINI_API_KEY -w
)"
```

Run the release gates from that shell and call `unset GEMINI_API_KEY` when the
session ends. The Keychain item remains local and is not part of Git, Docker
build context, AMOS storage, or rendered demo output.

AMOS rejects a public model URL under `air_gapped`, rejects a hosted route
whose hostname is absent from the allowlist, and fails closed when the
credential is missing. The credential is accepted only from the environment or
a mounted file; it is never accepted as a CLI argument.

## Release gate

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
cargo test --all-targets --release
cargo build --release --locked
docker compose config
docker compose build
scripts/live-model-smoke.sh
scripts/demo-smoke.sh
scripts/rehearse-recording.sh
```

`live-model-smoke.sh` runs one real plan and narrative call, then prints only
the model name, invocation IDs, latency, token counts, and hashes.
`demo-smoke.sh` creates a fresh root and exercises the HTTP analysis, evidence,
review, exact replay, and source-successor flow. `rehearse-recording.sh` runs
that complete flow twice with separate clean roots.

For the recording root, persist the successful compatibility probe so
`/health` can report it:

```bash
AMOS_PROBE_ROOT=.demo-recording scripts/live-model-smoke.sh
```

## Host-native recording server

```bash
set -a
source .env
set +a

cargo run --release --locked -- \
  --demo \
  --root .demo-recording \
  serve \
  --seed-demo \
  --bind 127.0.0.1 \
  --port 8000
```

Open `http://127.0.0.1:8000/`. Use the `Local demo identity` switch instead of
editing bearer headers:

1. As Analyst, submit the prefilled SMB churn question.
2. Open a material claim and show exact SQL, aggregate output, checks, governed
   objects, versions, and hashes.
3. Switch to Reviewer, open the Review Queue, and approve all claims.
4. Switch to Administrator, reopen the analysis, and receive the updated
   snapshot.
5. Show `Stale after source change`, the revalidation jobs, outbox records,
   audit event, and preserved replay contract.

The identity switch uses a random server-side session token in an `HttpOnly`,
`SameSite=Strict` cookie. It exists only on `api::demo_router`; the production
router has neither demo session nor demo source-successor routes.

## Compose

```bash
docker compose up --build
```

Compose publishes only `127.0.0.1:8000`, runs as an unprivileged user with all
Linux capabilities dropped, uses a read-only root filesystem, and persists the
demo control/warehouse/object root in `amos-demo-data`. The explicit
`AMOS_DEMO_LOOPBACK_PUBLISH=true` bridge override is valid only with that
loopback-only host port mapping; host-native demo mode rejects non-loopback
binds.

## Recording checks

- Use a clean 1440×900 browser window at 100% zoom.
- Confirm the boundary bar says `approved hosted API`, telemetry `disabled`,
  and the one allowed hostname.
- Confirm the visible model identity is `gemma_api:gemma-4-26b-a4b-it`.
- Never show `.env`, terminal history, request headers, or container
  environment.
- Confirm neither privacy canary appears in the task response, analysis page,
  evidence page, server log, or persisted model input.
- Keep a second complete live run as B-roll, labeled with its own A-TXN and
  invocation IDs.
