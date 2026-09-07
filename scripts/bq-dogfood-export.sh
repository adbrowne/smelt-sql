#!/usr/bin/env bash
# bq-dogfood-export.sh — thin wrapper that loads the BigQuery token and runs
# scripts/bq_dogfood_export.py. See that file for what it does.
#
#     bash scripts/bq-dogfood-export.sh <sql-file> <out.ndjson>
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# shellcheck disable=SC1091
. scripts/bigquery-env.sh >/dev/null 2>&1
: "${SMELT_BQ_ACCESS_TOKEN:?no valid token — run: bash scripts/bigquery-auth.sh}"
exec python3 scripts/bq_dogfood_export.py "$@"
