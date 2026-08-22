#!/usr/bin/env bash
# bigquery-probe-multiset.sh — does GoogleSQL accept the multiset-difference
# emulation the conformance oracle emits?
#
#     bash scripts/bigquery-probe-multiset.sh [report-path]
#
# BigQuery has no `EXCEPT ALL`, so the oracle emulates multiset difference by
# ranking each row's duplicate copies within its own identical-row group and
# differencing the ranked rows. That emulation partitions by a whole-row table
# alias — a STRUCT — and BigQuery restricts which types may be grouped or
# partitioned by. Whether it is accepted is a fact about the warehouse, so it
# is measured here rather than assumed from the shape compiling elsewhere.
#
# A dry run is enough: it type-checks and plans the query without reading a
# table or incurring cost, and it still rejects invalid SQL.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
REPORT="${1:-/tmp/bigquery-probe-multiset.txt}"

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

# try <label> <sql> — dry-run; prints ACCEPTED or the rejection verbatim.
try() {
  local label="$1" sql="$2" resp msg
  resp=$(curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$sql"),\"useLegacySql\":false,\"dryRun\":true}")
  msg=$(jq -r '.error.message // ""' <<<"$resp")
  if [[ -n "$msg" ]]; then
    say "  ${label}: REJECTED — $(tr '\n' ' ' <<<"$msg" | head -c 220)"
  else
    say "  ${label}: ACCEPTED"
  fi
}

LEFT="SELECT 1 AS id, 10.0 AS val UNION ALL SELECT 1, 10.0"
RIGHT="SELECT 1 AS id, 10.0 AS val"

say "# GoogleSQL acceptance of the multiset-difference emulation — $(date '+%Y-%m-%d %H:%M:%S')"
say "# project=${P}"
say ""

say "## The emulation exactly as oracle::bigquery_multiset_diff_sql emits it"
try "whole-row PARTITION BY t" \
  "SELECT count(*) FROM (SELECT * FROM (SELECT t.*, ROW_NUMBER() OVER (PARTITION BY t) AS __smelt_dup_rank FROM (${LEFT}) AS t) EXCEPT DISTINCT SELECT * FROM (SELECT t.*, ROW_NUMBER() OVER (PARTITION BY t) AS __smelt_dup_rank FROM (${RIGHT}) AS t)) AS d"
say ""

say "## Isolating each ingredient"
try "PARTITION BY whole-row alias alone" \
  "SELECT ROW_NUMBER() OVER (PARTITION BY t) AS r FROM (${LEFT}) AS t"
try "EXCEPT DISTINCT alone" \
  "SELECT * FROM (${LEFT}) EXCEPT DISTINCT SELECT * FROM (${RIGHT})"
try "EXCEPT ALL (expected absent)" \
  "SELECT * FROM (${LEFT}) EXCEPT ALL SELECT * FROM (${RIGHT})"
say ""

say "## Fallback shape, if partitioning by a STRUCT is refused"
try "PARTITION BY TO_JSON_STRING(t)" \
  "SELECT count(*) FROM (SELECT * FROM (SELECT t.*, ROW_NUMBER() OVER (PARTITION BY TO_JSON_STRING(t)) AS __smelt_dup_rank FROM (${LEFT}) AS t) EXCEPT DISTINCT SELECT * FROM (SELECT t.*, ROW_NUMBER() OVER (PARTITION BY TO_JSON_STRING(t)) AS __smelt_dup_rank FROM (${RIGHT}) AS t)) AS d"
say ""
say "Report written to ${REPORT}"
