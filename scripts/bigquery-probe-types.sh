#!/usr/bin/env bash
# bigquery-probe-types.sh — establish two facts the type oracle is built on.
#
#     bash scripts/bigquery-probe-types.sh [report-path]
#
# 1. Does a *dry run* carry the output schema? If it does, the oracle can ask
#    BigQuery for a query's column types without executing anything: no bytes
#    scanned, no cost, no table touched. If it does not, the oracle has to run
#    the query and read the Arrow schema instead.
# 2. Which CAST spellings does GoogleSQL accept? The property-test generators
#    emit DuckDB/Spark spellings (INTEGER, DOUBLE, TIMESTAMPTZ, DECIMAL(10,2));
#    the BigQuery leg needs the per-type spelling the warehouse actually takes,
#    established here rather than read off documentation.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-types.txt}"

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

# run_q <sql> [dryRun] — echoes the raw JSON response.
run_q() {
  local dry="${2:-false}"
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false,\"dryRun\":${dry}}"
}

say "# BigQuery type-oracle probe — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P} location=${SMELT_BQ_LOCATION:-?}"
say ""

# ------------------------------------------------------------- dry-run schema
say "## Does a dry run carry the output schema?"
DRY_SQL="WITH data AS (SELECT CAST(42 AS INT64) AS a, CAST('hi' AS STRING) AS b)
SELECT a AS expr_0, b AS expr_1, a + 1 AS expr_2 FROM data"

start=$(date +%s%N)
dry_resp=$(run_q "$DRY_SQL" true)
dry_ms=$(( ($(date +%s%N) - start) / 1000000 ))

dry_err=$(jq -r '.error.message // ""' <<<"$dry_resp")
if [[ -n "$dry_err" ]]; then
  say "  DRY RUN REJECTED — ${dry_err}"
else
  say "  dry run accepted in ${dry_ms}ms"
  say "  schema present : $(jq -r 'if .schema then "YES" else "NO" end' <<<"$dry_resp")"
  say "  columns        : $(jq -c '[.schema.fields[]? | {name, type, mode}]' <<<"$dry_resp")"
  say "  bytes billed   : $(jq -r '.totalBytesProcessed // "<absent>"' <<<"$dry_resp")"
  say "  top-level keys : $(jq -c 'keys' <<<"$dry_resp")"
fi
say ""

# For comparison: the same query executed for real, so the latency difference
# between the two paths is measured rather than assumed.
say "## The same query executed (comparison)"
start=$(date +%s%N)
run_resp=$(run_q "$DRY_SQL" false)
run_ms=$(( ($(date +%s%N) - start) / 1000000 ))
run_err=$(jq -r '.error.message // ""' <<<"$run_resp")
if [[ -n "$run_err" ]]; then
  say "  REJECTED — ${run_err}"
else
  say "  executed in ${run_ms}ms"
  say "  columns : $(jq -c '[.schema.fields[]? | {name, type, mode}]' <<<"$run_resp")"
fi
say ""

# -------------------------------------------------------------- cast spellings
# probe_cast <label> <cast-expression> — records the reported column type when
# the warehouse accepts the spelling, or the refusal when it does not.
probe_cast() {
  local label="$1" expr="$2" resp msg type
  resp=$(run_q "SELECT ${expr} AS c" true)
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    printf '  \033[31m✗\033[0m %-46s %s\n' "$label" "$(head -c 90 <<<"$msg")"
    printf '  NO   %-46s %s\n' "$label" "$(head -c 90 <<<"$msg")" >>"$REPORT"
  else
    type=$(jq -r '.schema.fields[0].type // "<no schema>"' <<<"$resp")
    printf '  \033[32m✓\033[0m %-46s → %s\n' "$label" "$type"
    printf '  YES  %-46s → %s\n' "$label" "$type" >>"$REPORT"
  fi
}

say "## CAST spellings — canonical GoogleSQL"
probe_cast "BOOL"            "CAST(TRUE AS BOOL)"
probe_cast "INT64"           "CAST(42 AS INT64)"
probe_cast "FLOAT64"         "CAST(3.14 AS FLOAT64)"
probe_cast "STRING"          "CAST('hello' AS STRING)"
probe_cast "DATE"            "CAST('2024-01-01' AS DATE)"
probe_cast "TIMESTAMP"       "CAST('2024-01-01 12:00:00' AS TIMESTAMP)"
probe_cast "DATETIME"        "CAST('2024-01-01 12:00:00' AS DATETIME)"
probe_cast "TIME"            "CAST('12:00:00' AS TIME)"
probe_cast "NUMERIC"         "CAST(99.99 AS NUMERIC)"
probe_cast "NUMERIC(10,2)"   "CAST(99.99 AS NUMERIC(10,2))"
probe_cast "BIGNUMERIC(38,9)" "CAST(99.99 AS BIGNUMERIC(38,9))"
probe_cast "BYTES"           "CAST('ab' AS BYTES)"
say ""

say "## CAST spellings — the aliases the generators currently emit"
probe_cast "BOOLEAN (alias)"      "CAST(TRUE AS BOOLEAN)"
probe_cast "INTEGER (alias)"      "CAST(42 AS INTEGER)"
probe_cast "INT (alias)"          "CAST(42 AS INT)"
probe_cast "BIGINT (alias)"       "CAST(100 AS BIGINT)"
probe_cast "DOUBLE (alias)"       "CAST(3.14 AS DOUBLE)"
probe_cast "FLOAT (alias)"        "CAST(3.14 AS FLOAT)"
probe_cast "VARCHAR (alias)"      "CAST('hello' AS VARCHAR)"
probe_cast "TEXT (alias)"         "CAST('hello' AS TEXT)"
probe_cast "DECIMAL (alias)"      "CAST(99.99 AS DECIMAL)"
probe_cast "DECIMAL(10,2) (alias)" "CAST(99.99 AS DECIMAL(10,2))"
probe_cast "TIMESTAMPTZ"          "CAST('2024-01-01 12:00:00+00' AS TIMESTAMPTZ)"
say ""

say "## INTERVAL — is there any cast form at all?"
probe_cast "CAST(str AS INTERVAL)" "CAST('1 day' AS INTERVAL)"
probe_cast "INTERVAL literal"      "INTERVAL 1 DAY"
probe_cast "INTERVAL 1 DAY + date" "DATE '2024-01-01' + INTERVAL 1 DAY"
probe_cast "TIMESTAMP - TIMESTAMP" "TIMESTAMP '2024-01-02 00:00:00' - TIMESTAMP '2024-01-01 00:00:00'"
say ""

say "## Timestamp/array/struct reporting — what names come back"
probe_cast "ARRAY<INT64>"     "[1,2,3]"
probe_cast "STRUCT"           "STRUCT(1 AS a, 'x' AS b)"
probe_cast "ARRAY<STRUCT>"    "[STRUCT(1 AS a)]"
probe_cast "parameterised STRING(10)" "CAST('hi' AS STRING(10))"
say ""

say "## Does a dry run reject genuinely invalid SQL?"
bad=$(run_q "SELECT nosuchfunction(1) AS c" true)
say "  nosuchfunction : $(jq -r '.error.message // "<ACCEPTED — dry run does not validate!>"' <<<"$bad" | head -c 120)"
say ""

say "Report written to ${REPORT}"
