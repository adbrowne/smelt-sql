#!/usr/bin/env bash
# bigquery-probe4.sh — which DuckDB-canonical type names does GoogleSQL accept?
#
# Re-run of the type leg of bigquery-probe3.sh, which was invalidated partway
# through by BigQuery's per-table update-operation quota: reusing one table name
# for every CREATE OR REPLACE trips the limit and the refusals stop meaning
# anything about the type. Each type therefore gets its own table.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || { echo "no valid token" >&2; exit 1; }

P="${SMELT_BQ_PROJECT}"; D="${SMELT_BQ_DATASET}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

run_q() {
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}

i=0
for ty in INTEGER BIGINT SMALLINT TINYINT DOUBLE REAL FLOAT VARCHAR TEXT STRING \
          BOOLEAN BOOL DECIMAL NUMERIC BIGNUMERIC TIMESTAMP DATE TIME DATETIME \
          INT64 FLOAT64 BYTES JSON INTERVAL; do
  i=$((i+1))
  t="probe4_$$_${i}"
  msg=$(jq -r '.error.message // ""' <<<"$(run_q "CREATE TABLE \`${P}.${D}.${t}\` (c ${ty})")")
  if [[ -z "$msg" ]]; then
    printf '  ACCEPTED  %s\n' "$ty"
  else
    printf '  REFUSED   %-12s -- %s\n' "$ty" "$(head -c 80 <<<"$msg")"
  fi
  run_q "DROP TABLE IF EXISTS \`${P}.${D}.${t}\`" >/dev/null 2>&1 || true
done
echo "done"
