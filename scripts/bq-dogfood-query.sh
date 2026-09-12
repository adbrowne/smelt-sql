#!/usr/bin/env bash
# bq-dogfood-query.sh — run one BigQuery REST call using the short-lived token
# scripts/bigquery-auth.sh mints, without ever printing the token.
#
#   bash scripts/bq-dogfood-query.sh get   <project> <dataset> <table>
#   bash scripts/bq-dogfood-query.sh query <sql-file> [maxResults]
#   bash scripts/bq-dogfood-query.sh dry   <sql-file>
#
# Interim tool for the dogfood spine's schema/sample phase. The billing project
# is SMELT_BQ_PROJECT (the existing test project); the only thing read is the
# public `githubarchive` dataset. Once the dogfood project is provisioned this
# is superseded by that project's own credential path.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
# shellcheck disable=SC1091
. scripts/bigquery-env.sh >/dev/null 2>&1
: "${SMELT_BQ_ACCESS_TOKEN:?no valid token — run: bash scripts/bigquery-auth.sh}"
: "${SMELT_BQ_PROJECT:?no SMELT_BQ_PROJECT}"
AUTH="Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}"
API="https://bigquery.googleapis.com/bigquery/v2"

case "${1:?mode}" in
  get)
    curl -sS -H "$AUTH" "$API/projects/$2/datasets/$3/tables/$4"
    ;;
  query|dry)
    dry=false
    [ "$1" = dry ] && dry=true
    sql="$(cat "${2:?sql file}")"
    max="${3:-200}"
    req="$(mktemp)"
    trap 'rm -f "$req"' EXIT
    jq -n --arg q "$sql" --argjson dry "$dry" --argjson max "$max" \
      '{query:$q, useLegacySql:false, dryRun:$dry, maxResults:$max, timeoutMs:180000}' \
      > "$req"
    curl -sS -H "$AUTH" -H 'Content-Type: application/json' \
      -X POST "$API/projects/${SMELT_BQ_PROJECT}/queries" \
      --data-binary @"$req"
    ;;
  *) echo "unknown mode $1" >&2; exit 2 ;;
esac
