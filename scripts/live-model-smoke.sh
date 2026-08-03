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
  echo "live model smoke requires AMOS_MODEL_PROVIDER=gemma_api" >&2
  exit 2
fi
if [[ -z "${GEMINI_API_KEY:-}" && -z "${GEMINI_API_KEY_FILE:-}" ]]; then
  echo "live model smoke requires GEMINI_API_KEY or GEMINI_API_KEY_FILE" >&2
  exit 2
fi

if [[ -n "${AMOS_PROBE_ROOT:-}" ]]; then
  probe_root="$AMOS_PROBE_ROOT"
  mkdir -p "$probe_root"
else
  probe_root="$(mktemp -d "${TMPDIR:-/tmp}/amos-live-probe.XXXXXX")"
  trap 'rm -rf "$probe_root"' EXIT
fi

cargo run --quiet --release --locked -- \
  --demo \
  --root "$probe_root" \
  model-probe
