#!/bin/sh
set -eu

DEPLOY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$DEPLOY_DIR"
umask 077
mkdir -p backups

timestamp=$(date -u '+%Y%m%dT%H%M%SZ')
archive="backups/amos-evaluation-${timestamp}.tar.gz"
restart_required=false

cleanup() {
  if [ "$restart_required" = true ]; then
    docker compose start amos >/dev/null
  fi
}
trap cleanup EXIT HUP INT TERM

if docker compose ps --status running --services | grep -qx amos; then
  docker compose stop amos >/dev/null
  restart_required=true
fi

if ! docker compose run --rm --no-deps -T --entrypoint /bin/tar amos \
  -C /var/lib/amos -czf - . > "$archive"; then
  rm -f "$archive"
  echo "Failed to create the AMOS backup archive." >&2
  exit 1
fi

chmod 600 "$archive"
echo "Created ${DEPLOY_DIR}/${archive}"
