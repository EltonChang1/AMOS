#!/bin/sh
set -eu

DEPLOY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$DEPLOY_DIR"

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Required command is unavailable: $1" >&2
    exit 1
  fi
}

require_command docker
require_command openssl
docker compose version >/dev/null

resume=false
case "${1:-}" in
  "") ;;
  --resume) resume=true ;;
  *)
    echo "Usage: $0 [--resume]" >&2
    exit 2
    ;;
esac
if [ "$#" -gt 1 ]; then
  echo "Usage: $0 [--resume]" >&2
  exit 2
fi

umask 077
mkdir -p config secrets backups

if [ "$resume" = true ]; then
  for required_file in \
    config/server.json \
    secrets/capability-key \
    secrets/identities.json; do
    if [ ! -f "$required_file" ]; then
      echo "Cannot resume: required installation file is missing: $required_file" >&2
      exit 1
    fi
  done
  echo "Resuming with existing configuration and credentials; no secret will be replaced."
else
  if [ -e config/server.json ] || [ -e secrets/capability-key ] || [ -e secrets/identities.json ]; then
    echo "Installation files already exist. Refusing to replace configuration or credentials; use --resume after reviewing them." >&2
    exit 1
  fi

  PUBLIC_BASE_URL=${AMOS_PUBLIC_BASE_URL:-https://amos.local}
  case "$PUBLIC_BASE_URL" in
    https://*) ;;
    *)
      echo "AMOS_PUBLIC_BASE_URL must be an HTTPS URL." >&2
      exit 1
      ;;
  esac

  CAPABILITY_KEY=$(openssl rand -hex 32)
  ANALYST_TOKEN=$(openssl rand -hex 32)
  REVIEWER_TOKEN=$(openssl rand -hex 32)
  ADMIN_TOKEN=$(openssl rand -hex 32)

  token_hash() {
    printf '%s' "$1" | openssl dgst -sha256 -r | awk '{print $1}'
  }

  ANALYST_HASH=$(token_hash "$ANALYST_TOKEN")
  REVIEWER_HASH=$(token_hash "$REVIEWER_TOKEN")
  ADMIN_HASH=$(token_hash "$ADMIN_TOKEN")

  printf '%s\n' "$CAPABILITY_KEY" > secrets/capability-key

  sed "s|https://amos.customer.example|$PUBLIC_BASE_URL|" \
    config/server.example.json > config/server.json

  cat > secrets/identities.json <<EOF
{
  "schema_version": 1,
  "identities": [
    {
      "token_sha256": "$ANALYST_HASH",
      "identity": {
        "tenant_id": "tenant_demo",
        "subject_id": "customer_analyst",
        "roles": ["analyst"],
        "groups": [],
        "permissions": ["analytics", "payments"],
        "policy_attributes": {},
        "policy_epoch": 1
      }
    },
    {
      "token_sha256": "$REVIEWER_HASH",
      "identity": {
        "tenant_id": "tenant_demo",
        "subject_id": "customer_reviewer",
        "roles": ["reviewer"],
        "groups": [],
        "permissions": ["analytics", "payments"],
        "policy_attributes": {},
        "policy_epoch": 1
      }
    },
    {
      "token_sha256": "$ADMIN_HASH",
      "identity": {
        "tenant_id": "tenant_demo",
        "subject_id": "customer_admin",
        "roles": ["admin", "owner", "reviewer"],
        "groups": [],
        "permissions": ["analytics", "payments", "sre", "admin"],
        "policy_attributes": {},
        "policy_epoch": 1
      }
    }
  ]
}
EOF

  cat > secrets/initial-tokens.txt <<EOF
AMOS customer-evaluation credentials
Generated: $(date -u '+%Y-%m-%dT%H:%M:%SZ')
Public URL: $PUBLIC_BASE_URL

Analyst bearer token: $ANALYST_TOKEN
Reviewer bearer token: $REVIEWER_TOKEN
Administrator bearer token: $ADMIN_TOKEN

Store these in the customer's approved secret manager, distribute each token
only to its intended operator, then securely delete this file.
EOF

  chmod 644 config/server.json
  # Compose implements local file-backed secrets as bind mounts on native Linux.
  # The enclosing directory remains operator-only while the mounted files must be
  # readable by the unprivileged container UID.
  chmod 700 secrets
  chmod 604 secrets/capability-key secrets/identities.json
  chmod 600 secrets/initial-tokens.txt
fi

if [ ! -e .env ]; then
  cp .env.example .env
  chmod 600 .env
fi

echo "Building the pinned AMOS image..."
docker compose build --pull

echo "Validating configuration and mounted secrets..."
docker compose run --rm --no-deps --entrypoint /usr/local/bin/amosctl amos \
  validate --config /etc/amos/server.json

if docker compose run --rm --no-deps --entrypoint /usr/local/bin/amosctl amos \
  preflight --config /etc/amos/server.json --require-initialized >/dev/null 2>&1; then
  echo "Existing initialized data volume passed preflight; bootstrap skipped."
else
  echo "Initializing the explicit payment reference fixture..."
  docker compose run --rm --no-deps --entrypoint /usr/local/bin/amosctl amos \
    bootstrap-reference --config /etc/amos/server.json
fi

echo "Starting AMOS and the governed analyst toolbox..."
docker compose up -d amos

attempt=0
until docker compose exec -T amos /usr/local/bin/amosctl health --host 127.0.0.1 --port 8000 >/dev/null 2>&1; do
  attempt=$((attempt + 1))
  if [ "$attempt" -ge 30 ]; then
    docker compose logs --tail=100 amos >&2
    docker compose logs --tail=100 toolbox >&2
    echo "AMOS did not become healthy." >&2
    exit 1
  fi
  sleep 1
done

echo "AMOS is healthy on its configured endpoint."
if [ -f secrets/initial-tokens.txt ]; then
  echo "Initial credentials are stored in ${DEPLOY_DIR}/secrets/initial-tokens.txt."
else
  echo "No plaintext initial-token file is present; use the credentials retained in the approved secret manager."
fi
echo "This package runs the local reference adapters for customer evaluation; it is not the production pilot topology."
