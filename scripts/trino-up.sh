#!/usr/bin/env bash
#
# trino-up.sh — stand up the pinned Trino + Iceberg REST + MinIO tier for
# smelt's Trino target integration tests (docs/specs/multi_backend.md).
#
# Idempotent over leftovers from a previous run: unlike scripts/spark-up.sh's
# host-owned bind mounts (which hit a `chmod`-on-root-owned-leftover failure
# under `set -e`, aborting before `docker run` with no hint the server never
# started), this tier's writable state lives entirely in Docker-managed named
# volumes. `down -v` always removes them cleanly, so this script always starts
# from a known-empty state — there is no host path for a stale-ownership
# failure to hide in.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
COMPOSE_FILE="${REPO_ROOT}/scripts/trino-compose.yml"
PORT="${SMELT_TRINO_PORT:-18080}"

cd "${REPO_ROOT}/scripts"

# Never `set -e`-abort on a tier that isn't running yet.
docker compose -f "${COMPOSE_FILE}" down -v --remove-orphans >/dev/null 2>&1 || true

echo "Starting Trino tier (trinodb/trino:483, apache/iceberg-rest-fixture:1.10.1, minio) on :${PORT}"
docker compose -f "${COMPOSE_FILE}" up -d

echo -n "Waiting for Trino coordinator to come up"
for _ in $(seq 1 60); do
  info="$(curl -sf "http://localhost:${PORT}/v1/info" 2>/dev/null || true)"
  if [ -n "${info}" ] && printf '%s' "${info}" | grep -q '"starting":false'; then
    echo " — ready."
    echo "SMELT_TRINO_URL=http://localhost:${PORT}"
    echo "Run 'source scripts/trino-env.sh' to export it for tests."
    exit 0
  fi
  echo -n "."
  sleep 2
done

echo
echo "ERROR: Trino coordinator did not report ready within ~120s. Recent logs:" >&2
docker compose -f "${COMPOSE_FILE}" logs --tail 50 >&2
exit 1
