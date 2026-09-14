#!/usr/bin/env bash
# trino-probe-state.sh — does Iceberg give Trino cross-table transaction
# atomicity?
#
#     bash scripts/trino-up.sh
#     source scripts/trino-env.sh
#     bash scripts/trino-probe-state.sh
#
# `docs/outcomes/20260913-trino-ledger/outcome.md` (T3) starts from the
# assumption that Trino's transaction support is equivalent to Spark
# (Delta)'s: per-table atomic commits, no cross-table transaction. Unlike
# Spark, Trino has explicit `START TRANSACTION`/`COMMIT` syntax, so that
# absence is a property of the Iceberg connector rather than of the SQL
# surface, and this script measures it against a live coordinator rather than
# assuming it.
#
# T1 (`docs/outcomes/20260913-trino-target-spine`) measured
# `supports_transactional_ddl = false` through smelt's stateless
# `/v1/statement` client, which opens no session and so can never carry a
# transaction across statements — that measured the client, not Iceberg. This
# script instead speaks the statement protocol directly and threads the
# transaction id the coordinator hands back (`X-Trino-Started-Transaction-Id`
# on the response that starts one) forward as `X-Trino-Transaction-Id` on
# every following statement, so every case here genuinely shares one
# transaction the way a session-holding client would.
#
# Every case gets its own fresh table(s) — a case that leaves a table in a
# torn state must never make the next case's answer meaningless — and every
# verdict is confirmed by an out-of-transaction row count, not by trusting a
# statement's own reported success.
set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]:-$0}")/.."
[[ -n "${SMELT_TRINO_URL:-}" ]] || {
  echo "no SMELT_TRINO_URL — run: source scripts/trino-env.sh" >&2
  exit 1
}

TRINO_USER="${SMELT_TRINO_USER:-smelt}"
TRINO_CATALOG="${SMELT_TRINO_CATALOG:-iceberg}"
TRINO_SCHEMA="${SMELT_TRINO_SCHEMA:-smelt_dev}"
RUN="$(date +%s)_$$"

# Tables created by this run, for best-effort cleanup on exit.
CREATED_TABLES=()
cleanup() {
  for t in "${CREATED_TABLES[@]:-}"; do
    [[ -n "${t}" ]] || continue
    TXN_ID="" run_stmt "DROP TABLE IF EXISTS ${t}" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

# Set by run_stmt: LAST_ERROR (empty on success), LAST_ROWS (JSON
# array-of-arrays of the last data page seen, "[]" if none). TXN_ID persists
# across calls once the coordinator hands one out, so callers thread a
# transaction by simply calling run_stmt repeatedly without resetting it.
TXN_ID=""
LAST_ERROR=""
LAST_ROWS="[]"

run_stmt() {
  local sql="$1"
  local headers_file url first=1
  headers_file="$(mktemp)"
  LAST_ERROR=""
  LAST_ROWS="[]"
  url="${SMELT_TRINO_URL}/v1/statement"

  while true; do
    local -a curl_args=(-sS -D "${headers_file}"
      -H "X-Trino-User: ${TRINO_USER}"
      -H "X-Trino-Catalog: ${TRINO_CATALOG}"
      -H "X-Trino-Schema: ${TRINO_SCHEMA}")
    # Trino refuses `START TRANSACTION` with "Client does not support
    # transactions" unless every request — including the very first,
    # transaction-less one — carries this header. A client that never sends
    # it at all is assumed incapable of reading back
    # `X-Trino-Started-Transaction-Id` and threading it forward, so the
    # server won't open a transaction it can never learn was committed.
    # `NONE` is the literal the protocol expects for "no transaction yet".
    curl_args+=(-H "X-Trino-Transaction-Id: ${TXN_ID:-NONE}")
    if [[ ${first} -eq 1 ]]; then
      curl_args+=(-X POST --data-binary "${sql}" "${url}")
      first=0
    else
      curl_args+=("${url}")
    fi

    local resp
    resp="$(curl "${curl_args[@]}")"

    local started
    started="$(grep -i '^X-Trino-Started-Transaction-Id:' "${headers_file}" \
      | tr -d '\r' | awk '{print $2}' || true)"
    [[ -n "${started}" ]] && TXN_ID="${started}"

    local err
    err="$(printf '%s' "${resp}" | jq -r '.error.message // empty')"
    if [[ -n "${err}" ]]; then
      LAST_ERROR="${err}"
      rm -f "${headers_file}"
      return 0
    fi

    local rows
    rows="$(printf '%s' "${resp}" | jq -c '.data // empty')"
    [[ -n "${rows}" && "${rows}" != "null" ]] && LAST_ROWS="${rows}"

    local next
    next="$(printf '%s' "${resp}" | jq -r '.nextUri // empty')"
    if [[ -z "${next}" ]]; then
      rm -f "${headers_file}"
      break
    fi
    url="${next}"
  done
}

# Row count via a fresh, non-transactional statement — never trusts the
# in-transaction statement's own reported success. Returns "ERR(<message>)"
# if the count query itself fails (e.g. the table never existed).
row_count() {
  local table="$1" saved_txn="${TXN_ID}"
  TXN_ID=""
  run_stmt "SELECT count(*) FROM ${table}"
  TXN_ID="${saved_txn}"
  if [[ -n "${LAST_ERROR}" ]]; then
    echo "ERR(${LAST_ERROR%%$'\n'*})"
  else
    printf '%s' "${LAST_ROWS}" | jq -r '.[0][0] // "0"'
  fi
}

table_exists() {
  local table="$1" saved_txn="${TXN_ID}"
  TXN_ID=""
  run_stmt "SELECT 1 FROM ${table} LIMIT 1"
  TXN_ID="${saved_txn}"
  [[ -z "${LAST_ERROR}" ]]
}

fresh_table() {
  local leaf="$1"
  local t="${TRINO_CATALOG}.${TRINO_SCHEMA}.probe_${RUN}_${leaf}"
  CREATED_TABLES+=("${t}")
  printf '%s' "${t}"
}

verdict() {
  local label="$1"
  if [[ -n "${LAST_ERROR}" ]]; then
    printf '  REFUSED     %-40s -- %s\n' "${label}" "${LAST_ERROR%%$'\n'*}"
  else
    printf '  ACCEPTED    %-40s\n' "${label}"
  fi
}

run_stmt "CREATE SCHEMA IF NOT EXISTS ${TRINO_CATALOG}.${TRINO_SCHEMA}"
if [[ -n "${LAST_ERROR}" ]]; then
  echo "FATAL: could not ensure schema ${TRINO_CATALOG}.${TRINO_SCHEMA} exists: ${LAST_ERROR}" >&2
  exit 1
fi

echo "=== A. baseline: single-statement INSERT outside any transaction ==="
t=$(fresh_table a)
run_stmt "CREATE TABLE ${t} (id INTEGER, label VARCHAR)"
verdict "CREATE TABLE"
run_stmt "INSERT INTO ${t} VALUES (1, 'x')"
verdict "INSERT (no transaction)"
echo "  row count: $(row_count "${t}")"

echo "=== B. syntax: START TRANSACTION; COMMIT with nothing between ==="
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "COMMIT"
verdict "COMMIT"
TXN_ID=""

echo "=== C. cross-table happy path: START TRANSACTION; INSERT t1; INSERT t2; COMMIT ==="
t1=$(fresh_table c1)
t2=$(fresh_table c2)
run_stmt "CREATE TABLE ${t1} (id INTEGER, label VARCHAR)"
run_stmt "CREATE TABLE ${t2} (id INTEGER, label VARCHAR)"
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "INSERT INTO ${t1} VALUES (1, 'x')"
verdict "INSERT t1"
run_stmt "INSERT INTO ${t2} VALUES (1, 'x')"
verdict "INSERT t2"
run_stmt "COMMIT"
verdict "COMMIT"
TXN_ID=""
echo "  t1 row count: $(row_count "${t1}")"
echo "  t2 row count: $(row_count "${t2}")"

echo "=== D. cross-table, second statement fails: does t1's row survive? ==="
t1=$(fresh_table d1)
t2=$(fresh_table d2)
run_stmt "CREATE TABLE ${t1} (id INTEGER, label VARCHAR)"
run_stmt "CREATE TABLE ${t2} (id INTEGER, label VARCHAR)"
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "INSERT INTO ${t1} VALUES (1, 'x')"
verdict "INSERT t1 (valid)"
run_stmt "INSERT INTO ${t2} VALUES ('not-an-integer', 'x')"
verdict "INSERT t2 (type mismatch, expected to fail)"
run_stmt "COMMIT"
verdict "COMMIT (after a failed statement in the transaction)"
TXN_ID=""
echo "  THE ATOMICITY QUESTION -- t1 row count: $(row_count "${t1}")"
echo "  t2 row count: $(row_count "${t2}")"

echo "=== E. explicit rollback: START TRANSACTION; INSERT t1; ROLLBACK ==="
t1=$(fresh_table e1)
run_stmt "CREATE TABLE ${t1} (id INTEGER, label VARCHAR)"
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "INSERT INTO ${t1} VALUES (1, 'x')"
verdict "INSERT t1"
run_stmt "ROLLBACK"
verdict "ROLLBACK"
TXN_ID=""
echo "  t1 row count (expect 0): $(row_count "${t1}")"

echo "=== F. DDL in a transaction: START TRANSACTION; CREATE TABLE; ROLLBACK ==="
t1=$(fresh_table f1)
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "CREATE TABLE ${t1} (id INTEGER, label VARCHAR)"
verdict "CREATE TABLE (in transaction)"
run_stmt "ROLLBACK"
verdict "ROLLBACK"
TXN_ID=""
if table_exists "${t1}"; then
  echo "  table survives ROLLBACK: yes"
else
  echo "  table survives ROLLBACK: no"
fi

echo "=== G. same-table two writes: START TRANSACTION; two INSERTs; COMMIT ==="
t1=$(fresh_table g1)
run_stmt "CREATE TABLE ${t1} (id INTEGER, label VARCHAR)"
TXN_ID=""
run_stmt "START TRANSACTION"
verdict "START TRANSACTION"
run_stmt "INSERT INTO ${t1} VALUES (1, 'x')"
verdict "INSERT #1"
run_stmt "INSERT INTO ${t1} VALUES (2, 'y')"
verdict "INSERT #2"
run_stmt "COMMIT"
verdict "COMMIT"
TXN_ID=""
echo "  row count (expect 2 if multi-write transactions work at all): $(row_count "${t1}")"

echo
echo "Probe complete."
