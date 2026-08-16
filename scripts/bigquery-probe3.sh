#!/usr/bin/env bash
# bigquery-probe3.sh — verify the GoogleSQL shapes the maintenance emitters need.
#
# `MaintenanceDialect` gains a BigQuery variant, and each of its emitter branches
# must be GoogleSQL that the warehouse actually accepts rather than a guess
# inherited from the DuckDB or Spark branch.
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
report() {
  local name="$1" sql="$2" resp msg val
  resp=$(run_q "$sql")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -z "$msg" ]]; then
    val=$(jq -r '.rows[0].f[0].v // "<no rows>"' <<<"$resp")
    printf '  ACCEPTED  %-46s -> %s\n' "$name" "$(head -c 40 <<<"$val")"
  else
    printf '  REFUSED   %-46s -- %s\n' "$name" "$(head -c 95 <<<"$msg")"
  fi
}

T="probe3_$$"
TBL="\`${P}.${D}.${T}\`"
run_q "CREATE OR REPLACE TABLE ${TBL} AS SELECT 1 AS k, 'a' AS s UNION ALL SELECT 2, 'b'" >/dev/null

echo "## probe_dialect_string_type — the unsized string cast"
report "CAST(k AS STRING)"  "SELECT CAST(k AS STRING) FROM ${TBL} LIMIT 1"
report "CAST(k AS VARCHAR)" "SELECT CAST(k AS VARCHAR) FROM ${TBL} LIMIT 1"

echo
echo "## probe_dialect_sample_agg — join sampled keys into one string"
report "STRING_AGG(s, ', ')" "SELECT STRING_AGG(s, ', ') FROM ${TBL}"

echo
echo "## agg_fingerprint — order-insensitive digest over row hashes"
report "SHA256(STRING_AGG(x,'' ORDER BY x))" \
  "SELECT TO_HEX(SHA256(STRING_AGG(TO_HEX(SHA256(s)), '' ORDER BY TO_HEX(SHA256(s))))) FROM ${TBL}"
report "TO_HEX(SHA256(s)) row fingerprint" "SELECT TO_HEX(SHA256(s)) FROM ${TBL} LIMIT 1"

echo
echo "## emit_create_table_as — bootstrap needs no format clause"
report "CREATE TABLE ... AS SELECT (no USING)" \
  "CREATE OR REPLACE TABLE \`${P}.${D}.${T}_boot\` AS SELECT k FROM ${TBL}"
report "MERGE against that plain table" \
  "MERGE \`${P}.${D}.${T}_boot\` t USING (SELECT 1 AS k) d ON t.k = d.k
   WHEN MATCHED THEN UPDATE SET k = d.k"

echo
echo "## bootstrap_column_sql_type — which DuckDB type names does GoogleSQL accept?"
for ty in INTEGER BIGINT DOUBLE VARCHAR TEXT BOOLEAN DECIMAL TIMESTAMP DATE INT64 FLOAT64 STRING BOOL NUMERIC; do
  report "CREATE TABLE (c ${ty})" "CREATE OR REPLACE TABLE \`${P}.${D}.${T}_ty\` (c ${ty})"
done

run_q "DROP TABLE IF EXISTS ${TBL}" >/dev/null 2>&1 || true
run_q "DROP TABLE IF EXISTS \`${P}.${D}.${T}_boot\`" >/dev/null 2>&1 || true
run_q "DROP TABLE IF EXISTS \`${P}.${D}.${T}_ty\`" >/dev/null 2>&1 || true
echo
echo "cleaned up"
