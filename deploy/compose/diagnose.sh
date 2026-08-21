#!/bin/sh
set -eu

DEPLOY_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$DEPLOY_DIR"

docker compose run --rm --no-deps --entrypoint /usr/local/bin/amosctl amos \
  preflight --config /etc/amos/server.json --require-initialized
docker compose run --rm --no-deps --entrypoint /usr/local/bin/amosctl amos \
  status --config /etc/amos/server.json
docker compose ps
docker compose exec -T amos /usr/local/bin/amosctl health --host 127.0.0.1 --port 8000
docker compose exec -T toolbox python -c \
  "import urllib.request; urllib.request.urlopen('http://127.0.0.1:9000/health', timeout=3).read()"
docker compose exec -T amos /usr/local/bin/amosctl tools smoke \
  --endpoint toolbox:9000 \
  --capability-key-file /run/secrets/capability_key
