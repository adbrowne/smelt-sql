#!/usr/bin/env bash
#
# trino-down.sh — tear down the Trino tier started by scripts/trino-up.sh,
# removing its named volumes. Tolerant of nothing running.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="${REPO_ROOT}/scripts/trino-compose.yml"

cd "${REPO_ROOT}/scripts"
docker compose -f "${COMPOSE_FILE}" down -v --remove-orphans && echo "Trino tier removed." || echo "No Trino tier running."
