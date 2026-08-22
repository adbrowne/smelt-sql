#!/usr/bin/env bash
# bigquery-probe.sh — establish the BigQuery capability matrix empirically.
#
#     bash scripts/bigquery-probe.sh [report-path]
#
# The multi_backend spec calls its capability matrix the *honest* matrix and a
# conformance test asserts it against the code constructors, so BigQuery's
# column has to come from a live warehouse rather than from documentation.
# This script runs one probe statement per capability flag, records which the
# warehouse accepts, and measures the per-table DML rate limit that sizes the
# generative conformance suite.
#
# The report it writes is the input to the capability constructor, so the
# implementation work does not need a live token held for its whole duration.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe.txt}"

# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || {
  echo "no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
}

P="${SMELT_BQ_PROJECT}"
D="${SMELT_BQ_DATASET}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

: >"$REPORT"
say()  { printf '%s\n' "$*" | tee -a "$REPORT"; }
ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; printf '  YES  %s\n' "$1" >>"$REPORT"; }
no()   { printf '  \033[31m✗\033[0m %s — %s\n' "$1" "$2"; printf '  NO   %s — %s\n' "$1" "$2" >>"$REPORT"; }

# Run one statement. Echoes the raw JSON response.
run_q() {
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}

# probe <flag-name> <sql> — records YES when the warehouse accepts the statement.
probe() {
  local name="$1" sql="$2" resp msg
  resp=$(run_q "$sql")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -z "$msg" ]]; then ok "$name"; else no "$name" "$(head -c 110 <<<"$msg")"; fi
}

say "# BigQuery capability probe — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P} dataset=${D} location=${SMELT_BQ_LOCATION:-?}"
say ""

# ---------------------------------------------------------------- control dataset
# A "not found" error is indistinguishable from a permission refusal unless the
# control dataset genuinely exists, so report both raw errors rather than
# classifying them.
say "## Least-privilege control (raw errors, classified by hand)"
CTL="${D}_notgranted"
sel_msg=$(jq -r '.error.message // "<no error>"' <<<"$(run_q "SELECT 1 FROM \`${P}.${CTL}.__probe__\`")")
crt_msg=$(jq -r '.error.message // "<no error — WROTE SUCCESSFULLY>"' <<<"$(run_q "CREATE TABLE \`${P}.${CTL}.nope\` AS SELECT 1")")
say "  SELECT against control : ${sel_msg}"
say "  CREATE against control : ${crt_msg}"
say ""

# ---------------------------------------------------------------- fixtures
T="probe_$$"
S="${T}_src"
run_q "CREATE OR REPLACE TABLE \`${P}.${D}.${T}\` (k INT64, v INT64, s STRING)" >/dev/null
run_q "INSERT INTO \`${P}.${D}.${T}\` VALUES (1,10,'a'),(2,20,'b')" >/dev/null
run_q "CREATE OR REPLACE TABLE \`${P}.${D}.${S}\` (k INT64, v INT64, s STRING)" >/dev/null
run_q "INSERT INTO \`${P}.${D}.${S}\` VALUES (2,99,'z'),(3,30,'c')" >/dev/null

TBL="\`${P}.${D}.${T}\`"
SRC="\`${P}.${D}.${S}\`"

say "## Capability flags"
probe supports_qualify \
  "SELECT k FROM ${TBL} QUALIFY ROW_NUMBER() OVER (ORDER BY k) = 1"
probe supports_create_or_replace_table \
  "CREATE OR REPLACE TABLE \`${P}.${D}.${T}_cor\` AS SELECT 1 AS x"
probe supports_create_or_replace_view \
  "CREATE OR REPLACE VIEW \`${P}.${D}.${T}_v\` AS SELECT 1 AS x"
probe supports_merge \
  "MERGE ${TBL} t USING ${SRC} s ON t.k = s.k
   WHEN MATCHED THEN UPDATE SET v = s.v
   WHEN NOT MATCHED THEN INSERT (k,v,s) VALUES (s.k, s.v, s.s)"
probe supports_column_scoped_merge \
  "MERGE ${TBL} t USING (SELECT k, v FROM ${SRC}) s ON t.k = s.k
   WHEN MATCHED THEN UPDATE SET v = s.v"
probe supports_merge_not_matched_by_source \
  "MERGE ${TBL} t USING ${SRC} s ON t.k = s.k
   WHEN MATCHED THEN UPDATE SET v = s.v
   WHEN NOT MATCHED BY SOURCE THEN DELETE"
probe supports_staged_relation_group \
  "BEGIN
     CREATE TEMP TABLE staged AS SELECT k, v FROM ${TBL};
     INSERT INTO ${TBL} (k,v,s) SELECT k, v, 'staged' FROM staged WHERE FALSE;
     DROP TABLE staged;
   END"
probe supports_pivot \
  "SELECT * FROM (SELECT k, v, s FROM ${TBL}) PIVOT (SUM(v) FOR s IN ('a','b'))"
probe supports_date_literal \
  "SELECT DATE '2024-01-01' AS d"
probe supports_concat_operator \
  "SELECT 'a' || 'b' AS c"
probe supports_array_literal \
  "SELECT [1,2,3] AS a"
probe supports_transactional_ddl \
  "BEGIN TRANSACTION;
   INSERT INTO ${TBL} (k,v,s) VALUES (99,99,'txn');
   DELETE FROM ${TBL} WHERE k = 99;
   COMMIT TRANSACTION"
probe supports_double_colon_cast \
  "SELECT 1::INT64 AS x"
probe supports_trailing_commas \
  "SELECT k, v, FROM ${TBL}"
probe supports_insert_overwrite \
  "INSERT OVERWRITE ${TBL} SELECT k, v, s FROM ${SRC}"
probe supports_native_ivm \
  "CREATE MATERIALIZED VIEW \`${P}.${D}.${T}_mv\` AS SELECT k, SUM(v) AS sv FROM ${TBL} GROUP BY k"
probe supports_struct_field_ddl \
  "ALTER TABLE ${TBL} ADD COLUMN st STRUCT<a INT64, b STRING>"
probe supports_alter_column_using \
  "ALTER TABLE ${TBL} ALTER COLUMN v SET DATA TYPE NUMERIC"
probe supports_nested_array_ddl \
  "ALTER TABLE ${TBL} ADD COLUMN arr ARRAY<STRUCT<a INT64>>"
probe supports_column_mapping \
  "ALTER TABLE ${TBL} RENAME COLUMN s TO s_renamed"
probe supports_pipe_syntax \
  "FROM ${TBL} |> SELECT k, v |> WHERE k = 1"

say ""
say "## Per-table DML rate limit (sequential UPDATEs against one table)"
# maintenance_conformance applies repeated DML to a single table, which is the
# shape BigQuery throttles. Drive it until it either throttles or clears a run
# long enough to show the limit does not bind at this scale.
N=25
fails=0
start_all=$(date +%s%N)
for i in $(seq 1 $N); do
  st=$(date +%s%N)
  resp=$(run_q "UPDATE ${TBL} SET v = v + 1 WHERE k = 1")
  el=$(( ($(date +%s%N) - st) / 1000000 ))
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    fails=$((fails+1))
    say "  stmt ${i}: FAILED after ${el}ms — $(head -c 130 <<<"$msg")"
  else
    say "  stmt ${i}: ok ${el}ms"
  fi
done
total=$(( ($(date +%s%N) - start_all) / 1000000 ))
say "  ${N} sequential UPDATEs on one table: ${fails} failures, ${total}ms total ($((total/N))ms/stmt)"

say ""
say "## Cleanup"
for t in "$T" "${T}_cor" "${T}_src"; do
  run_q "DROP TABLE IF EXISTS \`${P}.${D}.${t}\`" >/dev/null 2>&1 || true
done
run_q "DROP VIEW IF EXISTS \`${P}.${D}.${T}_v\`" >/dev/null 2>&1 || true
run_q "DROP MATERIALIZED VIEW IF EXISTS \`${P}.${D}.${T}_mv\`" >/dev/null 2>&1 || true
say "  dropped probe fixtures (the dataset's 24h expiry backstops any leak)"

say ""
say "Report written to ${REPORT}"
