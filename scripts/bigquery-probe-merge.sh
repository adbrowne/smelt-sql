#!/usr/bin/env bash
# bigquery-probe-merge.sh — establish which MERGE arm shapes GoogleSQL accepts.
#
#     bash scripts/bigquery-probe-merge.sh [report-path]
#
# `merge_parity`'s BigQuery leg fails on the whole-row MERGE that
# `smelt_logical::maintenance::emit::emit_column_scoped_merge` emits for every
# backend (`WHEN MATCHED THEN UPDATE SET *`, `WHEN NOT MATCHED THEN INSERT *`).
# The spec's rule is that a capability value comes from the warehouse and never
# from documentation, so this probe runs each candidate arm shape and records
# which the warehouse takes — that answer is the input to whatever emitter or
# capability change follows.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-merge.txt}"

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
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }

run_q() {
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}

probe() {
  local name="$1" sql="$2" resp msg
  resp=$(run_q "$sql")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -z "$msg" ]]; then
    printf '  \033[32m✓\033[0m %s\n' "$name"
    printf '  YES  %s\n' "$name" >>"$REPORT"
  else
    printf '  \033[31m✗\033[0m %s — %s\n' "$name" "$(head -c 120 <<<"$msg")"
    printf '  NO   %s — %s\n' "$name" "$(head -c 120 <<<"$msg")" >>"$REPORT"
  fi
}

say "# BigQuery MERGE arm-shape probe"
say "# project=${P} dataset=${D}"
say ""

# Distinct target table per shape: BigQuery's per-table modification quota binds
# on repeated writes to one table name (see the research doc's rate finding).
mk_target() {
  run_q "CREATE OR REPLACE TABLE \`${P}.${D}.$1\` (user_id STRING, total_score INT64)" >/dev/null
  run_q "INSERT INTO \`${P}.${D}.$1\` VALUES ('A',100),('B',200)" >/dev/null
}

SRC="SELECT 'A' AS user_id, CAST(300 AS INT64) AS total_score UNION ALL SELECT 'C', CAST(50 AS INT64)"

say "## Matched arm (update)"
T1="mrg_set_star_$$"; mk_target "$T1"
probe "WHEN MATCHED THEN UPDATE SET *" \
  "MERGE INTO \`${P}.${D}.${T1}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN MATCHED THEN UPDATE SET *"

T2="mrg_set_cols_$$"; mk_target "$T2"
probe "WHEN MATCHED THEN UPDATE SET <col> = source.<col>" \
  "MERGE INTO \`${P}.${D}.${T2}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN MATCHED THEN UPDATE SET total_score = source.total_score"

say ""
say "## Not-matched arm (insert)"
T3="mrg_ins_star_$$"; mk_target "$T3"
probe "WHEN NOT MATCHED THEN INSERT *" \
  "MERGE INTO \`${P}.${D}.${T3}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN NOT MATCHED THEN INSERT *"

T4="mrg_ins_row_$$"; mk_target "$T4"
probe "WHEN NOT MATCHED THEN INSERT ROW" \
  "MERGE INTO \`${P}.${D}.${T4}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN NOT MATCHED THEN INSERT ROW"

say ""
say "## Combined shapes"
T5="mrg_both_star_$$"; mk_target "$T5"
probe "UPDATE SET * + INSERT * (the emitter's current text)" \
  "MERGE INTO \`${P}.${D}.${T5}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN MATCHED THEN UPDATE SET * WHEN NOT MATCHED THEN INSERT *"

T6="mrg_both_bq_$$"; mk_target "$T6"
probe "UPDATE SET <cols> + INSERT ROW (candidate GoogleSQL form)" \
  "MERGE INTO \`${P}.${D}.${T6}\` AS target USING (${SRC}) AS source ON target.user_id = source.user_id WHEN MATCHED THEN UPDATE SET total_score = source.total_score WHEN NOT MATCHED THEN INSERT ROW"

say ""
say "## Teardown"
for t in "$T1" "$T2" "$T3" "$T4" "$T5" "$T6"; do
  run_q "DROP TABLE IF EXISTS \`${P}.${D}.${t}\`" >/dev/null
done
say "  dropped ${T1} … ${T6}"
say ""
say "report: ${REPORT}"
