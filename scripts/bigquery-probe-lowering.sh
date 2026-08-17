#!/usr/bin/env bash
# bigquery-probe-lowering.sh — what GoogleSQL accepts in place of the two
# constructs smelt emits that it rejects: the `FROM (VALUES …)` table
# constructor and `MEDIAN`.
#
#     bash scripts/bigquery-probe-lowering.sh [report-path]
#
# The generative conformance leg found both by running. Before the printer can
# lower them, the candidate replacements have to be seen accepted — and for
# MEDIAN, seen to return the same value, since BigQuery's nearest equivalents
# differ in whether they are exact and whether they are an aggregate at all.
#
# Dry runs settle acceptance for free; the MEDIAN candidates additionally run
# for real, because acceptance is not the question there — agreement is.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-lowering.txt}"

# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || {
  echo "no valid SMELT_BQ_ACCESS_TOKEN — run: bash scripts/bigquery-auth.sh" >&2
  exit 1
}

P="${SMELT_BQ_PROJECT:?SMELT_BQ_PROJECT unset}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

: >"$REPORT"
say() { printf '%s\n' "$*" | tee -a "$REPORT"; }

try() {
  local label="$1" sql="$2" resp msg
  resp=$(curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$sql"),\"useLegacySql\":false,\"dryRun\":true}")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    say "  ${label}: REJECTED — $(tr '\n' ' ' <<<"$msg" | head -c 200)"
  else
    say "  ${label}: ACCEPTED"
  fi
}

# run <label> <sql> — executes for real and prints the first row, because for
# MEDIAN the question is agreement, not acceptance.
run() {
  local label="$1" sql="$2" resp msg
  resp=$(curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$sql"),\"useLegacySql\":false,\"timeoutMs\":60000}")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    say "  ${label}: REJECTED — $(tr '\n' ' ' <<<"$msg" | head -c 200)"
  else
    say "  ${label}: $(jq -c '[.rows[0].f[].v]' <<<"$resp")"
  fi
}

say "# GoogleSQL lowering candidates — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P}"
say ""

say "## A. The row-set constructor smelt emits, and candidate replacements"
try "FROM (VALUES ...) AS t(cols)   [what smelt emits today]" \
  "SELECT * FROM (VALUES (DATE '2020-01-01', 1, 10), (DATE '2020-01-02', 2, 20)) AS t(d, id, val)"
try "UNNEST([STRUCT(...)])" \
  "SELECT * FROM UNNEST([STRUCT(DATE '2020-01-01' AS d, 1 AS id, 10 AS val), STRUCT(DATE '2020-01-02' AS d, 2 AS id, 20 AS val)])"
try "SELECT ... UNION ALL SELECT ..." \
  "SELECT * FROM (SELECT DATE '2020-01-01' AS d, 1 AS id, 10 AS val UNION ALL SELECT DATE '2020-01-02', 2, 20) AS t"
try "single-row UNION ALL form (the zero-row guard case)" \
  "SELECT d, id, val FROM (SELECT DATE '1970-01-01' AS d, 0 AS id, 0 AS val) AS t WHERE 1=0"
say ""

say "## B. MEDIAN, and what BigQuery offers instead"
say "   Fixture: val in (1,2,3,4) — an EVEN count, so an interpolating median"
say "   returns 2.5 and a nearest-rank one returns 2 or 3. That distinction is"
say "   the whole question."
FIX="SELECT 1 AS val UNION ALL SELECT 2 UNION ALL SELECT 3 UNION ALL SELECT 4"
try "MEDIAN(val)   [what smelt emits today]" \
  "SELECT MEDIAN(val) AS m FROM (${FIX})"
run "PERCENTILE_CONT(val, 0.5) OVER ()  [analytic, needs DISTINCT/LIMIT]" \
  "SELECT DISTINCT PERCENTILE_CONT(val, 0.5) OVER () AS m FROM (${FIX})"
run "PERCENTILE_DISC(val, 0.5) OVER ()" \
  "SELECT DISTINCT PERCENTILE_DISC(val, 0.5) OVER () AS m FROM (${FIX})"
run "APPROX_QUANTILES(val, 2)[OFFSET(1)]  [aggregate, but approximate]" \
  "SELECT APPROX_QUANTILES(val, 2)[OFFSET(1)] AS m FROM (${FIX})"
say ""

say "## C. Is the analytic form usable where an aggregate is required?"
say "   A GROUP BY query is the shape the recipe pool generates."
try "PERCENTILE_CONT inside GROUP BY (expected to fail — analytic, not aggregate)" \
  "SELECT id, PERCENTILE_CONT(val, 0.5) OVER (PARTITION BY id) AS m FROM (SELECT 1 AS id, 1 AS val) GROUP BY id"
try "grouped APPROX_QUANTILES" \
  "SELECT id, APPROX_QUANTILES(val, 2)[OFFSET(1)] AS m FROM (SELECT 1 AS id, 1 AS val) GROUP BY id"
try "grouped PERCENTILE_CONT via a DISTINCT subquery" \
  "SELECT DISTINCT id, PERCENTILE_CONT(val, 0.5) OVER (PARTITION BY id) AS m FROM (SELECT 1 AS id, 1 AS val)"
say ""
say "Report written to ${REPORT}"
