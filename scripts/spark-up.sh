#!/usr/bin/env bash
#
# spark-up.sh — stand up a local Spark Connect server for smelt parity tests.
#
# Brings up a single detached container running the Spark Connect service on
# :15002, with a host-shared warehouse directory bind-mounted so host-side
# DuckDB can read Spark-produced Parquet for the cross-engine path
# (docs/specs/multi_backend.md §"Cross-engine data exchange").
#
# This is a long-lived service: start it ONCE, then export SPARK_CONNECT_URL
# (see scripts/spark-env.sh) into the environment of the autonomy loop / test
# runner. The loop's stateless iterations do not stand Spark up themselves.
#
# Pinned to whatever `apache/spark` image is local (Spark 4.1.x, Scala 2.13,
# Java 21, Python 3.10 in-container). The Connect jar is bundled in the image,
# so no --packages is needed for the core server. The host pyspark client must
# match the server version — see scripts/spark-env.sh / README-spark.md.
set -euo pipefail

IMAGE="${SMELT_SPARK_IMAGE:-apache/spark:latest}"
NAME="${SMELT_SPARK_CONTAINER:-smelt-spark}"
PORT="${SMELT_SPARK_PORT:-15002}"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WAREHOUSE="${SMELT_SPARK_WAREHOUSE:-${REPO_ROOT}/.smelt-spark-warehouse}"

mkdir -p "${WAREHOUSE}"
# The apache/spark image runs as uid 185 (spark); the bind-mounted host
# warehouse is owned by the host user. Make it group/other-writable so the
# container can create managed-table dirs in it. Files Spark writes stay
# world-readable, so host-side DuckDB can read them for cross-engine Parquet.
chmod 777 "${WAREHOUSE}"
docker rm -f "${NAME}" >/dev/null 2>&1 || true

echo "Starting Spark Connect (${IMAGE}) on :${PORT}, warehouse=${WAREHOUSE}"
docker run -d --name "${NAME}" -p "${PORT}:15002" \
  -v "${WAREHOUSE}":/opt/spark/work-dir/warehouse \
  "${IMAGE}" \
  /opt/spark/bin/spark-submit \
    --class org.apache.spark.sql.connect.service.SparkConnectServer \
    --name "smelt-spark-connect" \
    --conf spark.connect.grpc.binding.port=15002 \
    --conf spark.sql.warehouse.dir=/opt/spark/work-dir/warehouse \
  >/dev/null

# Poll the container log until the Connect service reports it is listening.
echo -n "Waiting for Spark Connect to come up"
for _ in $(seq 1 60); do
  if docker logs "${NAME}" 2>&1 | grep -q "Spark Connect server started"; then
    echo " — ready."
    echo "SPARK_CONNECT_URL=sc://localhost:${PORT}"
    echo "warehouse: ${WAREHOUSE}"
    echo "Run 'source scripts/spark-env.sh' to export it for tests."
    exit 0
  fi
  echo -n "."
  sleep 2
done

echo
echo "ERROR: Spark Connect did not start within ~120s. Recent logs:" >&2
docker logs "${NAME}" 2>&1 | tail -30 >&2
exit 1
