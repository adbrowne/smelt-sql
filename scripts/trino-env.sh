# shellcheck shell=bash  # sourced, not executed — no shebang, mirrors spark-env.sh
#
# trino-env.sh — source this (`source scripts/trino-env.sh`) to point smelt's
# Trino integration tests at the local tier started by scripts/trino-up.sh.
#
#   SMELT_TRINO_URL — gate + connect URL for the Trino backend tests
#                     (docs/specs/multi_backend.md §"Session initialization").
#                     When UNSET, all Trino-targeted tests skip (green).
#
# The tier runs with no authentication configured, so SMELT_TRINO_USER is any
# non-empty name Trino's session protocol requires; SMELT_TRINO_CATALOG /
# SMELT_TRINO_SCHEMA name the Iceberg catalog (scripts/trino-catalog/
# iceberg.properties is registered as the `iceberg` catalog) and the schema
# integration tests materialize into.
export SMELT_TRINO_URL="http://localhost:${SMELT_TRINO_PORT:-18080}"
export SMELT_TRINO_USER="${SMELT_TRINO_USER:-smelt}"
export SMELT_TRINO_CATALOG="${SMELT_TRINO_CATALOG:-iceberg}"
export SMELT_TRINO_SCHEMA="${SMELT_TRINO_SCHEMA:-smelt_dev}"

echo "SMELT_TRINO_URL=${SMELT_TRINO_URL}"
echo "SMELT_TRINO_USER=${SMELT_TRINO_USER}"
echo "SMELT_TRINO_CATALOG=${SMELT_TRINO_CATALOG}"
echo "SMELT_TRINO_SCHEMA=${SMELT_TRINO_SCHEMA}"
