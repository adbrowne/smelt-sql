#!/usr/bin/env bash
# bigquery-probe-schema.sh — how BigQuery reports a query's output schema.
#
#     bash scripts/bigquery-probe-schema.sh [report-path]
#
# The first type probe established that a dry run carries the schema, but also
# that the schema speaks *legacy* type names (INT64 comes back as "INTEGER") and
# that `ARRAY<INT64>` reports the element type with no array wrapper. Before a
# type mapper can be written, the array marker, the decimal precision/scale
# reporting, and the struct field shape have to be seen rather than guessed.
#
# It probes both routes at once: the REST API the shell scripts use, and the
# google-cloud-bigquery client the PyO3 adapter uses, so the oracle's chosen
# route is known to report what this says it reports.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-schema.txt}"

# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || {
  echo "no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
}

P="${SMELT_BQ_PROJECT}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

: >"$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }

# full_schema <label> <select-list> — dumps every field object verbatim, so an
# attribute this probe did not think to ask for is still visible in the report.
full_schema() {
  local label="$1" sel="$2" resp msg
  resp=$(curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"SELECT ${sel}"),\"useLegacySql\":false,\"dryRun\":true}")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    say "  ${label}: REJECTED — $(head -c 100 <<<"$msg")"
  else
    say "  ${label}: $(jq -c '.schema.fields' <<<"$resp")"
  fi
}

say "# BigQuery output-schema shape — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P}"
say ""

say "## REST route — full field objects"
full_schema "int64"            "CAST(1 AS INT64) AS c"
full_schema "array of int64"   "[1,2,3] AS c"
full_schema "array of string"  "['a','b'] AS c"
full_schema "empty array"      "CAST([] AS ARRAY<INT64>) AS c"
full_schema "nested array"     "[STRUCT([1,2] AS inner)] AS c"
full_schema "struct"           "STRUCT(1 AS a, 'x' AS b) AS c"
full_schema "numeric literal"  "NUMERIC '99.99' AS c"
full_schema "numeric cast"     "CAST(99.99 AS NUMERIC) AS c"
full_schema "bignumeric"       "CAST(99.99 AS BIGNUMERIC) AS c"
full_schema "numeric division" "CAST(1 AS NUMERIC) / CAST(3 AS NUMERIC) AS c"
full_schema "sum of numeric"   "(SELECT SUM(x) FROM UNNEST([CAST(1 AS NUMERIC)]) x) AS c"
full_schema "timestamp"        "CURRENT_TIMESTAMP() AS c"
full_schema "datetime"         "CURRENT_DATETIME() AS c"
full_schema "interval"         "INTERVAL 1 DAY AS c"
full_schema "json"             "JSON '{\"a\":1}' AS c"
full_schema "geography"        "ST_GEOGPOINT(0,0) AS c"
say ""

say "## Python client route — does the adapter see the same thing?"
PY="${SMELT_BQ_PY:-python3}"
"$PY" scripts/bigquery_probe_schema.py 2>&1 | tee -a "$REPORT"

say ""
say "Report written to ${REPORT}"
