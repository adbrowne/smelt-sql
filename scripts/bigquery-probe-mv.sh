#!/usr/bin/env bash
# bigquery-probe-mv.sh — what does GoogleSQL actually accept for MATERIALIZED VIEW?
#
#     bash scripts/bigquery-auth.sh        # mint a token (prompts for passphrase)
#     bash scripts/bigquery-probe-mv.sh
#
# smelt is about to emit the native maintained object for `refresh:
# materialized_view` on BigQuery (`docs/specs/materialized_view.md`). Every
# rule the emitter encodes is MEASURED here rather than read from docs — the
# same discipline `bigquery-probe-ddl.sh` applied to schema-evolution DDL,
# where three of the obvious guesses turned out to be wrong.
#
# Four questions this has to answer before the emitter can be written:
#
#   A. Which CREATE/DROP forms exist? In particular OR REPLACE and IF NOT
#      EXISTS, which decide whether the emitter can be idempotent across runs
#      or must drop first.
#   B. What do the OPTIONS look like, and is refresh on by default? This is
#      the "engine owns freshness" claim made concrete.
#   C. What does a REFUSAL look like? `materialized_view.md` §"No smelt-side
#      eligibility" requires smelt to relay the engine's own reason verbatim,
#      so the shape of that message is part of the contract.
#   D. Does an MV serve FRESH results immediately after a base-table write, or
#      only after a refresh cycle? This decides whether a parity test can
#      assert equivalence synchronously at all, or must wait/force a refresh.
#
# Every case gets its OWN base table and its own view name. BigQuery refuses
# repeated modification of one table with `exceeded quota for table update
# operations` after roughly eight rapid statements, and a quota refusal says
# nothing about the form under test, so reuse would silently invalidate the
# tail of the run.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
# shellcheck disable=SC1091
source scripts/bigquery-env.sh >/dev/null 2>&1 || true
[[ -n "${SMELT_BQ_ACCESS_TOKEN:-}" ]] || { echo "no valid token — run: bash scripts/bigquery-auth.sh" >&2; exit 1; }

P="${SMELT_BQ_PROJECT}"; D="${SMELT_BQ_DATASET}"
API="https://bigquery.googleapis.com/bigquery/v2/projects/${P}/queries"

run_q() {
  curl -sS -X POST "$API" \
    -H "Authorization: Bearer ${SMELT_BQ_ACCESS_TOKEN}" \
    -H "Content-Type: application/json" \
    -d "{\"query\":$(jq -Rs . <<<"$1"),\"useLegacySql\":false}"
}

err_of() { jq -r '.error.message // ""' <<<"$1"; }
# First column of the first row, for the freshness probe.
val_of() { jq -r '.rows[0].f[0].v // ""' <<<"$1"; }

i=0
fresh_base() {
  # Creates a fresh base TABLE with (k STRING, v INT64) and two rows.
  # Echoes its backticked fully-qualified name.
  i=$((i+1))
  local t="probe_mv_base_$$_${i}"
  local fq="\`${P}.${D}.${t}\`"
  run_q "CREATE TABLE ${fq} (k STRING, v INT64)" >/dev/null
  run_q "INSERT INTO ${fq} (k, v) VALUES ('a', 1), ('b', 2)" >/dev/null
  echo "$fq"
}

# probe <label> <mv-statement-with-@B-base-and-@V-view-placeholders>
probe() {
  local label="$1" stmt="$2"
  local b v fq_v q
  b=$(fresh_base)
  v="probe_mv_view_$$_${i}"
  fq_v="\`${P}.${D}.${v}\`"
  stmt="${stmt//@B/${b}}"
  stmt="${stmt//@V/${fq_v}}"
  q=$(err_of "$(run_q "$stmt")")
  if [[ -z "$q" ]]; then
    printf '  ACCEPTED    %s\n' "$label"
  else
    printf '  REFUSED     %-44s -- %s\n' "$label" "$(head -c 150 <<<"$q")"
  fi
  run_q "DROP MATERIALIZED VIEW IF EXISTS ${fq_v}" >/dev/null 2>&1 || true
  run_q "DROP TABLE IF EXISTS ${b}" >/dev/null 2>&1 || true
}

echo "== A. CREATE / DROP forms =="
probe "plain CREATE MATERIALIZED VIEW" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"
probe "CREATE OR REPLACE MATERIALIZED VIEW" \
  "CREATE OR REPLACE MATERIALIZED VIEW @V AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"
probe "CREATE MATERIALIZED VIEW IF NOT EXISTS" \
  "CREATE MATERIALIZED VIEW IF NOT EXISTS @V AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"

echo
echo "== B. OPTIONS =="
probe "OPTIONS(enable_refresh=true)" \
  "CREATE MATERIALIZED VIEW @V OPTIONS(enable_refresh=true) AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"
probe "OPTIONS(enable_refresh, refresh_interval_minutes)" \
  "CREATE MATERIALIZED VIEW @V OPTIONS(enable_refresh=true, refresh_interval_minutes=30) AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"
probe "OPTIONS(max_staleness)" \
  "CREATE MATERIALIZED VIEW @V OPTIONS(max_staleness=INTERVAL \"4:0:0\" HOUR TO SECOND) AS SELECT k, SUM(v) AS total FROM @B GROUP BY k"

echo
echo "== C. Refusal shapes (what smelt must relay verbatim) =="
probe "top-level ORDER BY" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, v FROM @B ORDER BY v"
probe "LIMIT" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, v FROM @B LIMIT 10"
probe "SELECT DISTINCT" \
  "CREATE MATERIALIZED VIEW @V AS SELECT DISTINCT k FROM @B"
probe "UNION ALL" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, v FROM @B UNION ALL SELECT k, v FROM @B"
probe "window function" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, ROW_NUMBER() OVER (ORDER BY v) AS rn FROM @B"
probe "self join" \
  "CREATE MATERIALIZED VIEW @V AS SELECT a.k, a.v FROM @B a JOIN @B b ON a.k = b.k"
probe "non-aggregate plain projection" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k, v FROM @B"
probe "over a non-existent base table" \
  "CREATE MATERIALIZED VIEW @V AS SELECT k FROM \`${P}.${D}.no_such_table_$$\` GROUP BY k"

echo
echo "== D. Freshness: does an MV reflect a base write immediately? =="
b=$(fresh_base)
v="probe_mv_fresh_$$"
fq_v="\`${P}.${D}.${v}\`"
e=$(err_of "$(run_q "CREATE MATERIALIZED VIEW ${fq_v} AS SELECT k, SUM(v) AS total FROM ${b} GROUP BY k")")
if [[ -n "$e" ]]; then
  printf '  SETUP-FAIL  could not create the freshness MV -- %s\n' "$(head -c 150 <<<"$e")"
else
  before=$(val_of "$(run_q "SELECT SUM(total) FROM ${fq_v}")")
  printf '  baseline SUM(total) over the MV            = %s   (expect 3)\n' "$before"
  run_q "INSERT INTO ${b} (k, v) VALUES ('c', 10)" >/dev/null
  after=$(val_of "$(run_q "SELECT SUM(total) FROM ${fq_v}")")
  printf '  after INSERT of v=10, SUM(total)           = %s   (13 => serves fresh; 3 => needs a refresh cycle)\n' "$after"
  # Does the base table refuse to drop while an MV depends on it? Matters for teardown.
  d=$(err_of "$(run_q "DROP TABLE ${b}")")
  if [[ -z "$d" ]]; then
    printf '  DROP of the base table with a live MV      = ACCEPTED\n'
  else
    printf '  DROP of the base table with a live MV      = REFUSED -- %s\n' "$(head -c 120 <<<"$d")"
  fi
fi
run_q "DROP MATERIALIZED VIEW IF EXISTS ${fq_v}" >/dev/null 2>&1 || true
run_q "DROP TABLE IF EXISTS ${b}" >/dev/null 2>&1 || true

echo
echo "done."
