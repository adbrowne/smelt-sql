#!/usr/bin/env bash
#
# spark-down.sh — stop and remove the local Spark Connect server started by
# scripts/spark-up.sh. Leaves the warehouse directory in place (delete it
# manually if you want a clean slate).
set -euo pipefail
NAME="${SMELT_SPARK_CONTAINER:-smelt-spark}"
docker rm -f "${NAME}" >/dev/null 2>&1 && echo "Removed container ${NAME}." || echo "No container ${NAME} running."
