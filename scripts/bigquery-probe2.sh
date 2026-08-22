#!/usr/bin/env bash
# bigquery-probe2.sh — corrective probes for the three flags the first pass got wrong.
#
# `supports_alter_column_using` names the `ALTER COLUMN ... TYPE ... USING <expr>`
# clause, not type relaxation; the first pass probed a widening that BigQuery
# allows and wrongly read it as support. `supports_merge_schema_write` and
# `requires_schema_init` were not probed at all.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe2.txt}"

# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || { echo "no valid token" >&2; exit 1; }

P="${SMELT_BQ_PROJECT}"; D="${SMELT_BQ_DATASET}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

: >"$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }
run_q() {
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}
report() {
  local name="$1" sql="$2" msg
  msg=$(jq -r '.error.message // ""' <<<"$(run_q "$sql")")
  if [[ -z "$msg" ]]; then say "  ACCEPTED  ${name}"; else say "  REFUSED   ${name} — $(head -c 120 <<<"$msg")"; fi
}

T="probe2_$$"
TBL="\`${P}.${D}.${T}\`"
run_q "CREATE OR REPLACE TABLE ${TBL} (a INT64, s STRING)" >/dev/null
run_q "INSERT INTO ${TBL} VALUES (1,'x')" >/dev/null

say "# BigQuery corrective probe — $(date '+%Y-%m-%d %H:%M:%S')"
say ""
say "## supports_alter_column_using — the USING clause specifically"
report "ALTER COLUMN ... SET DATA TYPE ... USING (explicit USING clause)" \
  "ALTER TABLE ${TBL} ALTER COLUMN a SET DATA TYPE STRING USING CAST(a AS STRING)"
report "ALTER COLUMN ... SET DATA TYPE STRING (narrowing conversion, no USING)" \
  "ALTER TABLE ${TBL} ALTER COLUMN a SET DATA TYPE STRING"
report "ALTER COLUMN ... SET DATA TYPE NUMERIC (widening relaxation only)" \
  "ALTER TABLE ${TBL} ALTER COLUMN a SET DATA TYPE NUMERIC"

say ""
say "## supports_merge_schema_write — does a write invent a missing column?"
report "INSERT naming a column that does not exist" \
  "INSERT INTO ${TBL} (a, s, brand_new) VALUES (2,'y',3)"

say ""
say "## requires_schema_init — must the dataset pre-exist?"
report "CREATE TABLE in a dataset that does not exist" \
  "CREATE TABLE \`${P}.${D}_absent_$$.t\` AS SELECT 1 AS x"

say ""
say "## native IVM refresh semantics (informational)"
run_q "CREATE MATERIALIZED VIEW \`${P}.${D}.${T}_mv\` AS SELECT a, COUNT(*) c FROM ${TBL} GROUP BY a" >/dev/null 2>&1 || true
say "  enable_refresh/refresh_interval options:"
report "CREATE MATERIALIZED VIEW with explicit refresh options" \
  "CREATE MATERIALIZED VIEW \`${P}.${D}.${T}_mv2\`
     OPTIONS (enable_refresh = true, refresh_interval_minutes = 30)
     AS SELECT a, COUNT(*) c FROM ${TBL} GROUP BY a"
report "MATERIALIZED VIEW over a non-aggregate query" \
  "CREATE MATERIALIZED VIEW \`${P}.${D}.${T}_mv3\` AS SELECT a, s FROM ${TBL}"

say ""
say "## Cleanup"
for v in "${T}_mv" "${T}_mv2" "${T}_mv3"; do
  run_q "DROP MATERIALIZED VIEW IF EXISTS \`${P}.${D}.${v}\`" >/dev/null 2>&1 || true
done
run_q "DROP TABLE IF EXISTS ${TBL}" >/dev/null 2>&1 || true
say "  dropped probe fixtures"
