#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

if [[ -f .env ]]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

if [[ "${AMOS_MODEL_PROVIDER:-}" != "gemma_api" ]]; then
  echo "demo smoke requires AMOS_MODEL_PROVIDER=gemma_api" >&2
  exit 2
fi
if [[ -z "${GEMINI_API_KEY:-}" && -z "${GEMINI_API_KEY_FILE:-}" ]]; then
  echo "demo smoke requires GEMINI_API_KEY or GEMINI_API_KEY_FILE" >&2
  exit 2
fi
if ! command -v python3 >/dev/null 2>&1; then
  echo "demo smoke requires python3 for local JSON assertions" >&2
  exit 2
fi

smoke_root="${AMOS_SMOKE_ROOT:-$(mktemp -d "${TMPDIR:-/tmp}/amos-demo-smoke.XXXXXX")}"
smoke_port="${AMOS_SMOKE_PORT:-18080}"
smoke_url="http://127.0.0.1:${smoke_port}"
smoke_tmp="$(mktemp -d "${TMPDIR:-/tmp}/amos-demo-http.XXXXXX")"
server_pid=""

cleanup() {
  if [[ -n "$server_pid" ]]; then
    kill "$server_pid" 2>/dev/null || true
    wait "$server_pid" 2>/dev/null || true
  fi
  rm -rf "$smoke_tmp"
  if [[ -z "${AMOS_SMOKE_ROOT:-}" ]]; then
    rm -rf "$smoke_root"
  fi
}
trap cleanup EXIT

cargo build --quiet --release --locked
"$repo_root/target/release/amos" \
  --demo \
  --root "$smoke_root" \
  serve \
  --seed-demo \
  --bind 127.0.0.1 \
  --port "$smoke_port" \
  >"$smoke_tmp/server.log" 2>&1 &
server_pid="$!"

for _ in $(seq 1 120); do
  if curl --fail --silent "$smoke_url/health" >"$smoke_tmp/health.json"; then
    break
  fi
  if ! kill -0 "$server_pid" 2>/dev/null; then
    echo "AMOS smoke server exited during startup" >&2
    tail -40 "$smoke_tmp/server.log" >&2
    exit 1
  fi
  sleep 0.25
done
curl --fail --silent "$smoke_url/health" >/dev/null
python3 - "$smoke_tmp/health.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    health = json.load(handle)
assert health["status"] == "ok"
assert health["schema_version"] >= 8
assert health["model"]["provider"] == "gemma_api"
assert health["model"]["name"]
assert isinstance(health["model"]["compatibility_probe_passed"], bool)
assert health["warehouse"]["status"] == "healthy"
assert health["external_telemetry"] == "disabled"
serialized = json.dumps(health)
for forbidden in ("GEMINI_API_KEY", "Authorization", "Bearer ", "amos_demo_session"):
    assert forbidden not in serialized
PY

curl --fail --silent \
  -H "Authorization: Bearer analyst_001" \
  -H "Content-Type: application/json" \
  --data '{"request":"Why did SMB logo churn increase this week, and should the executive dashboard attribute it to the pricing email?","idempotency_key":"smoke-analysis"}' \
  "$smoke_url/v1/tasks" \
  >"$smoke_tmp/run.json"

read -r artifact_id atxn_id claim_id artifact_hash < <(
  python3 - "$smoke_tmp/run.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    run = json.load(handle)
claim = next(item for item in run["claims"] if item["support_execution_ids"])
print(
    run["artifact"]["artifact_id"],
    run["transaction"]["atxn_id"],
    claim["claim_id"],
    run["artifact"]["content_hash"],
)
PY
)

curl --fail --silent \
  -H "Authorization: Bearer analyst_001" \
  "$smoke_url/analyses/$artifact_id" \
  >"$smoke_tmp/analysis.html"
curl --fail --silent \
  -H "Authorization: Bearer analyst_001" \
  "$smoke_url/claims/$claim_id" \
  >"$smoke_tmp/claim.html"

if grep -Eq 'RESTRICTED_MEMORY_CANARY|WAREHOUSE_RAW_CANARY' \
  "$smoke_tmp/run.json" "$smoke_tmp/analysis.html" "$smoke_tmp/claim.html"; then
  echo "privacy canary escaped into an HTTP response" >&2
  exit 1
fi
grep -q "Permission-filtered model payload" "$smoke_tmp/analysis.html"
grep -q "Exact read-only SQL" "$smoke_tmp/analysis.html"
grep -q "Direct computational support" "$smoke_tmp/claim.html"

python3 - "$smoke_tmp/run.json" >"$smoke_tmp/review.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    run = json.load(handle)
print(json.dumps({
    "idempotency_key": "smoke-review",
    "claim_ids": [claim["claim_id"] for claim in run["claims"]],
    "decision": "approve",
    "comment": "Smoke rehearsal approves the cautious, evidence-bound language.",
    "correction": None,
    "authority": "reviewer_approved",
}))
PY

curl --fail --silent \
  -H "Authorization: Bearer reviewer_001" \
  -H "Content-Type: application/json" \
  --data-binary "@$smoke_tmp/review.json" \
  "$smoke_url/v1/artifacts/$artifact_id/reviews" \
  >"$smoke_tmp/review-response.json"
python3 - "$smoke_tmp/review-response.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    review = json.load(handle)
assert review["transaction"]["state"] == "published"
assert review["artifact"]["publication_validity"] == "valid_at_publication"
PY

curl --fail --silent \
  -H "Authorization: Bearer analyst_001" \
  -H "Content-Type: application/json" \
  --data '{"idempotency_key":"smoke-replay"}' \
  "$smoke_url/v1/artifacts/$artifact_id/replay" \
  >"$smoke_tmp/replay.json"
python3 - "$smoke_tmp/replay.json" <<'PY'
import json
import sys
with open(sys.argv[1], encoding="utf-8") as handle:
    replay = json.load(handle)
assert replay["status"] == "pass"
assert not replay["changed_execution_ids"]
assert all(item["comparison"] == "exact" for item in replay["comparisons"])
PY

curl --fail --silent \
  -H "Authorization: Bearer admin" \
  -H "Content-Type: application/x-www-form-urlencoded" \
  --data-urlencode "artifact_id=$artifact_id" \
  --data-urlencode "idempotency_key=smoke-source-successor" \
  "$smoke_url/demo/source-change" \
  >"$smoke_tmp/source-change.html"
curl --fail --silent \
  -H "Authorization: Bearer admin" \
  "$smoke_url/analyses/$artifact_id" \
  >"$smoke_tmp/stale-analysis.html"
grep -q "Stale after source change" "$smoke_tmp/stale-analysis.html"
grep -q "invalidation.processed" "$smoke_tmp/stale-analysis.html"

printf '{"artifact_id":"%s","atxn_id":"%s","artifact_hash":"%s","result":"pass"}\n' \
  "$artifact_id" "$atxn_id" "$artifact_hash"
