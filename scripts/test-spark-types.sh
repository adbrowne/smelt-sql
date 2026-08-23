#!/usr/bin/env bash
# Run type property tests against DuckDB + Spark locally.
#
# Usage:
#   ./scripts/test-spark-types.sh          # 256 cases (default)
#   ./scripts/test-spark-types.sh 1000     # custom case count

set -euo pipefail

CASES="${1:-256}"
CONTAINER="smelt-spark-test"
# Pinned, never `:latest` — an unpinned tag lets an upstream Spark release change
# what the tests assert with no code change. Keep in sync with scripts/spark-up.sh.
IMAGE="${SMELT_SPARK_IMAGE:-apache/spark:4.0.0}"

cleanup() {
    echo "Cleaning up Spark container..."
    docker rm -f "$CONTAINER" >/dev/null 2>&1 || true
}
trap cleanup EXIT

echo "Pulling $IMAGE..."
docker pull "$IMAGE"

echo "Starting Spark container..."
docker run -d --name "$CONTAINER" "$IMAGE" tail -f /dev/null

echo "Running type property tests ($CASES cases, DuckDB + Spark)..."
SPARK_CONTAINER_ID="$CONTAINER" PROPTEST_CASES="$CASES" \
    cargo test -p smelt-db --test type_property_tests -- --nocapture
