#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"

base_port="${AMOS_REHEARSAL_BASE_PORT:-18100}"
for pass in 1 2; do
  rehearsal_root="$(mktemp -d "${TMPDIR:-/tmp}/amos-recording-${pass}.XXXXXX")"
  trap 'rm -rf "$rehearsal_root"' EXIT
  echo "Recording rehearsal ${pass}/2"
  AMOS_SMOKE_ROOT="$rehearsal_root" \
  AMOS_SMOKE_PORT="$((base_port + pass))" \
    scripts/demo-smoke.sh
  rm -rf "$rehearsal_root"
  trap - EXIT
done
