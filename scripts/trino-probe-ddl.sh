#!/usr/bin/env bash
# trino-probe-ddl.sh — which schema-evolution DDL forms does Trino/Iceberg
# accept?
#
#     bash scripts/trino-up.sh
#     source scripts/trino-env.sh
#     bash scripts/trino-probe-ddl.sh
#
# smelt's Trino generator (`crates/smelt-state/src/ddl_trino/`) turns
# backend-agnostic `SchemaOperation`s into Trino SQL against an Iceberg table.
# This probe establishes, against a live coordinator, which forms Iceberg
# really accepts — the measured facts the generator's rules are written from.
# It reuses `trino-probe-state.sh`'s `run_stmt` (no transaction threading is
# needed here — every case commits its own DDL), cleanup trap and
# verbatim-error printing.
#
# Every case gets its own fresh table: a form that fails can leave the table
# in a state that makes the next form's answer meaningless.
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

CREATED_TABLES=()
cleanup() {
  for t in "${CREATED_TABLES[@]:-}"; do
    [[ -n "${t}" ]] || continue
    run_stmt "DROP TABLE IF EXISTS ${t}" >/dev/null 2>&1 || true
  done
}
trap cleanup EXIT

LAST_ERROR=""
LAST_ROWS="[]"

run_stmt() {
  local sql="$1"
  local url="${SMELT_TRINO_URL}/v1/statement"
  local first=1
  LAST_ERROR=""
  LAST_ROWS="[]"

  while true; do
    local -a curl_args=(-sS
      -H "X-Trino-User: ${TRINO_USER}"
      -H "X-Trino-Catalog: ${TRINO_CATALOG}"
      -H "X-Trino-Schema: ${TRINO_SCHEMA}")
    if [[ ${first} -eq 1 ]]; then
      curl_args+=(-X POST --data-binary "${sql}" "${url}")
      first=0
    else
      curl_args+=("${url}")
    fi

    local resp
    resp="$(curl "${curl_args[@]}")"

    local err
    err="$(printf '%s' "${resp}" | jq -r '.error.message // empty')"
    if [[ -n "${err}" ]]; then
      LAST_ERROR="${err}"
      return 0
    fi

    local rows
    rows="$(printf '%s' "${resp}" | jq -c '.data // empty')"
    [[ -n "${rows}" && "${rows}" != "null" ]] && LAST_ROWS="${rows}"

    local next
    next="$(printf '%s' "${resp}" | jq -r '.nextUri // empty')"
    if [[ -z "${next}" ]]; then
      break
    fi
    url="${next}"
  done
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
    printf '  REFUSED     %-52s -- %s\n' "${label}" "${LAST_ERROR%%$'\n'*}"
  else
    printf '  ACCEPTED    %-52s\n' "${label}"
  fi
}

row_count() {
  local table="$1"
  run_stmt "SELECT count(*) FROM ${table}"
  if [[ -n "${LAST_ERROR}" ]]; then
    echo "ERR(${LAST_ERROR%%$'\n'*})"
  else
    printf '%s' "${LAST_ROWS}" | jq -r '.[0][0] // "0"'
  fi
}

describe() {
  local table="$1"
  run_stmt "DESCRIBE ${table}"
  printf '%s' "${LAST_ROWS}" | jq -r '.[] | "\(.[0])\t\(.[1])"'
}

run_stmt "CREATE SCHEMA IF NOT EXISTS ${TRINO_CATALOG}.${TRINO_SCHEMA}"
if [[ -n "${LAST_ERROR}" ]]; then
  echo "FATAL: could not ensure schema ${TRINO_CATALOG}.${TRINO_SCHEMA} exists: ${LAST_ERROR}" >&2
  exit 1
fi

echo "=== A. type spellings: CREATE TABLE with one column per candidate type ==="
declare -A TYPES=(
  [boolean]="BOOLEAN"
  [smallint]="SMALLINT"
  [integer]="INTEGER"
  [bigint]="BIGINT"
  [real]="REAL"
  [double]="DOUBLE"
  [decimal]="DECIMAL(10,2)"
  [varchar_unbounded]="VARCHAR"
  [varchar_bounded]="VARCHAR(50)"
  [char]="CHAR(3)"
  [varbinary]="VARBINARY"
  [date]="DATE"
  [time]="TIME"
  [time6]="TIME(6)"
  [timestamp]="TIMESTAMP"
  [timestamp6]="TIMESTAMP(6)"
  [timestamptz6]="TIMESTAMP(6) WITH TIME ZONE"
  [interval_ds]="INTERVAL DAY TO SECOND"
  [row]="ROW(a INTEGER, b VARCHAR)"
  [array]="ARRAY(INTEGER)"
  [array_row]="ARRAY(ROW(a INTEGER))"
  [map]="MAP(VARCHAR, INTEGER)"
)
for key in "${!TYPES[@]}"; do
  t=$(fresh_table "ty_${key}")
  run_stmt "CREATE TABLE ${t} (c ${TYPES[${key}]})"
  verdict "CREATE TABLE with ${TYPES[${key}]}"
done

echo
echo "=== B. ADD COLUMN forms ==="
t=$(fresh_table b1)
run_stmt "CREATE TABLE ${t} (id INTEGER)"
run_stmt "ALTER TABLE ${t} ADD COLUMN amount BIGINT"
verdict "ADD COLUMN nullable, no default"

t=$(fresh_table b2)
run_stmt "CREATE TABLE ${t} (id INTEGER)"
run_stmt "ALTER TABLE ${t} ADD COLUMN amount BIGINT NOT NULL"
verdict "ADD COLUMN NOT NULL, no default"

t=$(fresh_table b3)
run_stmt "CREATE TABLE ${t} (id INTEGER)"
run_stmt "ALTER TABLE ${t} ADD COLUMN amount BIGINT NOT NULL DEFAULT 0"
verdict "ADD COLUMN NOT NULL DEFAULT 0"

t=$(fresh_table b4)
run_stmt "CREATE TABLE ${t} (id INTEGER)"
run_stmt "ALTER TABLE ${t} ADD COLUMN amount BIGINT DEFAULT 0"
verdict "ADD COLUMN nullable DEFAULT 0"
run_stmt "INSERT INTO ${t} (id) VALUES (1)"
run_stmt "SELECT amount FROM ${t} WHERE id = 1"
echo "  existing row's value after ADD COLUMN ... DEFAULT (expect NULL if DEFAULT governs only future inserts): $(printf '%s' "${LAST_ROWS}" | jq -c '.')"

echo
echo "=== C. nullability toggling ==="
t=$(fresh_table c1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET NOT NULL"
verdict "ALTER COLUMN ... SET NOT NULL (nullable column, may have NULLs)"

t=$(fresh_table c2)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT NOT NULL)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount DROP NOT NULL"
verdict "ALTER COLUMN ... DROP NOT NULL"

t=$(fresh_table c3)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT)"
run_stmt "INSERT INTO ${t} VALUES (1, 5)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET NOT NULL"
verdict "ALTER COLUMN ... SET NOT NULL (column has no NULLs)"

echo
echo "=== D. DROP COLUMN, top-level and struct field ==="
t=$(fresh_table d1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT)"
run_stmt "ALTER TABLE ${t} DROP COLUMN amount"
verdict "DROP COLUMN (top-level)"

t=$(fresh_table d2)
run_stmt "CREATE TABLE ${t} (id INTEGER, meta ROW(a INTEGER, b VARCHAR))"
run_stmt "ALTER TABLE ${t} DROP COLUMN meta.b"
verdict "DROP COLUMN (dotted struct field)"

echo
echo "=== E. widening via SET DATA TYPE ==="
t=$(fresh_table e1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount INTEGER)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET DATA TYPE BIGINT"
verdict "SET DATA TYPE INTEGER -> BIGINT"

t=$(fresh_table e2)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount REAL)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET DATA TYPE DOUBLE"
verdict "SET DATA TYPE REAL -> DOUBLE"

t=$(fresh_table e3)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount DECIMAL(10,2))"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET DATA TYPE DECIMAL(20,2)"
verdict "SET DATA TYPE DECIMAL(10,2) -> DECIMAL(20,2)"

t=$(fresh_table e4)
run_stmt "CREATE TABLE ${t} (id INTEGER, tags ARRAY(INTEGER))"
run_stmt "ALTER TABLE ${t} ALTER COLUMN tags SET DATA TYPE ARRAY(BIGINT)"
verdict "SET DATA TYPE ARRAY(INTEGER) -> ARRAY(BIGINT)"

t=$(fresh_table e5)
run_stmt "CREATE TABLE ${t} (id INTEGER, meta ROW(a INTEGER))"
run_stmt "ALTER TABLE ${t} ALTER COLUMN meta.a SET DATA TYPE BIGINT"
verdict "SET DATA TYPE on dotted struct field (meta.a INTEGER -> BIGINT)"

echo
echo "=== F. ALTER COLUMN ... USING (expected refused — no USING clause) ==="
t=$(fresh_table f1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount VARCHAR)"
run_stmt "ALTER TABLE ${t} ALTER COLUMN amount SET DATA TYPE INTEGER USING CAST(amount AS INTEGER)"
verdict "SET DATA TYPE ... USING CAST(...)"

echo
echo "=== G. backfill via UPDATE, and rewrite via UPDATE+CAST ==="
t=$(fresh_table g1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT)"
run_stmt "INSERT INTO ${t} VALUES (1, NULL), (2, NULL)"
run_stmt "UPDATE ${t} SET amount = 0 WHERE amount IS NULL"
verdict "UPDATE ... SET col = literal WHERE col IS NULL (backfill)"
echo "  row count after backfill: $(row_count "${t}")"

t=$(fresh_table g2)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount VARCHAR)"
run_stmt "INSERT INTO ${t} VALUES (1, '5')"
run_stmt "UPDATE ${t} SET amount = CAST(amount AS VARCHAR) WHERE TRUE"
verdict "UPDATE ... SET col = CAST(col AS t) WHERE TRUE (rewrite-by-update)"

echo
echo "=== H. ADD COLUMN inside a struct (dotted) and inside an array of structs ==="
t=$(fresh_table h1)
run_stmt "CREATE TABLE ${t} (id INTEGER, meta ROW(a INTEGER))"
run_stmt "ALTER TABLE ${t} ADD COLUMN meta.b VARCHAR"
verdict "ADD COLUMN meta.b (dotted struct field)"

t=$(fresh_table h2)
run_stmt "CREATE TABLE ${t} (id INTEGER, items ARRAY(ROW(a INTEGER)))"
run_stmt "ALTER TABLE ${t} ADD COLUMN items.element.b VARCHAR"
verdict "ADD COLUMN items.element.b (struct field inside array)"

echo
echo "=== I. RENAME COLUMN (column mapping / field-ID survival) ==="
t=$(fresh_table i1)
run_stmt "CREATE TABLE ${t} (id INTEGER, amount BIGINT)"
run_stmt "INSERT INTO ${t} VALUES (1, 5)"
run_stmt "ALTER TABLE ${t} RENAME COLUMN amount TO amt"
verdict "RENAME COLUMN"
run_stmt "SELECT amt FROM ${t} WHERE id = 1"
echo "  value read back via new name: $(printf '%s' "${LAST_ROWS}" | jq -c '.')"

echo
echo "=== J. schema-mismatched INSERT (merge-schema-on-write) ==="
t=$(fresh_table j1)
run_stmt "CREATE TABLE ${t} (id INTEGER)"
run_stmt "INSERT INTO ${t} (id, extra) VALUES (1, 'x')"
verdict "INSERT naming a column the table schema does not have"

echo
echo "Probe complete."
