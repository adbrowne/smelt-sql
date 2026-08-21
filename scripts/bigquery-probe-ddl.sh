#!/usr/bin/env bash
# bigquery-probe-ddl.sh — which schema-evolution DDL forms does GoogleSQL accept?
#
#     bash scripts/bigquery-auth.sh        # mint a token (prompts for passphrase)
#     bash scripts/bigquery-probe-ddl.sh
#
# smelt's DuckDB and Spark DDL generators emit `ALTER TABLE` statements from
# backend-agnostic `SchemaOperation`s. BigQuery has no generator: the dispatch
# in `schema_tracking.rs` refuses and falls back to a full refresh. This probe
# establishes, against the live warehouse, exactly which forms a GoogleSQL
# generator could emit — the measured facts the generator is then written from.
#
# Every case gets its OWN table. BigQuery refuses repeated modification of one
# table with `exceeded quota for table update operations` after roughly eight
# rapid statements, and a quota refusal says nothing about the DDL form, so
# reusing a table would silently invalidate the tail of the run (the mistake
# bigquery-probe3.sh made).
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

i=0
# probe <label> <column-defs-for-CREATE> <alter-body-with-@T-placeholder>
#
# Creates `CREATE TABLE <fresh> (<defs>)`, runs the ALTER with @T substituted by
# the fresh table's backticked name, reports, then drops.
probe() {
  local label="$1" defs="$2" stmt="$3"
  i=$((i+1))
  local t="probe_ddl_$$_${i}" q
  local fq="\`${P}.${D}.${t}\`"
  q=$(err_of "$(run_q "CREATE TABLE ${fq} (${defs})")")
  if [[ -n "$q" ]]; then
    printf '  SETUP-FAIL  %-46s -- %s\n' "$label" "$(head -c 90 <<<"$q")"
    return
  fi
  q=$(err_of "$(run_q "${stmt//@T/${fq}}")")
  if [[ -z "$q" ]]; then
    printf '  ACCEPTED    %s\n' "$label"
  else
    printf '  REFUSED     %-46s -- %s\n' "$label" "$(head -c 110 <<<"$q")"
  fi
  run_q "DROP TABLE IF EXISTS ${fq}" >/dev/null 2>&1 || true
}

echo "== ADD COLUMN: type names smelt's generators emit =="
for ty in INTEGER BIGINT SMALLINT DOUBLE VARCHAR TEXT STRING BOOLEAN BOOL \
          "DECIMAL(10,2)" "NUMERIC(10,2)" TIMESTAMP DATE TIME DATETIME \
          INT64 FLOAT64 BYTES JSON "ARRAY<INT64>" "STRUCT<a INT64>"; do
  probe "ADD COLUMN c ${ty}" "id INT64" "ALTER TABLE @T ADD COLUMN c ${ty}"
done

echo
echo "== ADD COLUMN: modifiers =="
probe "ADD COLUMN ... NOT NULL"              "id INT64" "ALTER TABLE @T ADD COLUMN c INT64 NOT NULL"
probe "ADD COLUMN ... DEFAULT 0"             "id INT64" "ALTER TABLE @T ADD COLUMN c INT64 DEFAULT 0"
probe "ADD COLUMN ... NOT NULL DEFAULT 0"    "id INT64" "ALTER TABLE @T ADD COLUMN c INT64 NOT NULL DEFAULT 0"
probe "ADD COLUMN IF NOT EXISTS"             "id INT64" "ALTER TABLE @T ADD COLUMN IF NOT EXISTS c INT64"
probe "ADD COLUMN quoted \`c\`"              "id INT64" "ALTER TABLE @T ADD COLUMN \`c\` INT64"
probe "ADD COLUMN quoted \"c\" (dq)"         "id INT64" "ALTER TABLE @T ADD COLUMN \"c\" INT64"
probe "two ADD COLUMNs, one statement"       "id INT64" "ALTER TABLE @T ADD COLUMN c INT64, ADD COLUMN d INT64"

echo
echo "== DROP COLUMN =="
probe "DROP COLUMN"                          "id INT64, c INT64" "ALTER TABLE @T DROP COLUMN c"
probe "DROP COLUMN IF EXISTS (absent)"       "id INT64"          "ALTER TABLE @T DROP COLUMN IF EXISTS c"

echo
echo "== ALTER COLUMN SET DATA TYPE: the widening lattice =="
probe "INT64 -> FLOAT64"        "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE FLOAT64"
probe "INT64 -> NUMERIC"        "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE NUMERIC"
probe "INT64 -> BIGNUMERIC"     "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE BIGNUMERIC"
probe "INT64 -> STRING"         "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE STRING"
probe "NUMERIC -> BIGNUMERIC"   "c NUMERIC"        "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE BIGNUMERIC"
probe "NUMERIC -> FLOAT64"      "c NUMERIC"        "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE FLOAT64"
probe "NUMERIC(5,2)->NUMERIC(10,4)" "c NUMERIC(5,2)" "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE NUMERIC(10,4)"
probe "NUMERIC(5,2)->NUMERIC(10,2)" "c NUMERIC(5,2)" "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE NUMERIC(10,2)"
probe "STRING(10) -> STRING"    "c STRING(10)"     "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE STRING"
probe "STRING(10) -> STRING(20)" "c STRING(10)"    "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE STRING(20)"
probe "BIGNUMERIC -> FLOAT64"   "c BIGNUMERIC"     "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE FLOAT64"
probe "DATE -> DATETIME"        "c DATE"           "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE DATETIME"
probe "widen with alias BIGINT" "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE BIGINT"
probe "DuckDB spelling: ALTER COLUMN c TYPE X" "c INT64" "ALTER TABLE @T ALTER COLUMN c TYPE FLOAT64"
probe "USING clause"            "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE STRING USING CAST(c AS STRING)"

echo
echo "== ALTER COLUMN: nullability =="
probe "DROP NOT NULL"           "c INT64 NOT NULL" "ALTER TABLE @T ALTER COLUMN c DROP NOT NULL"
probe "SET NOT NULL"            "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET NOT NULL"
probe "SET DEFAULT"             "c INT64"          "ALTER TABLE @T ALTER COLUMN c SET DEFAULT 7"

echo
echo "== nested struct fields =="
probe "ADD COLUMN s.b (dotted)"      "s STRUCT<a INT64>" "ALTER TABLE @T ADD COLUMN s.b INT64"
probe "ADD COLUMN \`s\`.\`b\`"       "s STRUCT<a INT64>" "ALTER TABLE @T ADD COLUMN \`s\`.\`b\` INT64"
probe "DROP COLUMN s.a (dotted)"     "s STRUCT<a INT64, b INT64>" "ALTER TABLE @T DROP COLUMN s.a"
probe "widen struct: SET DATA TYPE"  "s STRUCT<a INT64>" "ALTER TABLE @T ALTER COLUMN s SET DATA TYPE STRUCT<a NUMERIC>"
probe "widen array elem: SET DATA TYPE" "c ARRAY<INT64>" "ALTER TABLE @T ALTER COLUMN c SET DATA TYPE ARRAY<NUMERIC>"

echo
echo "== backfill UPDATE =="
probe "UPDATE ... WHERE c IS NULL"  "id INT64, c INT64" "UPDATE @T SET c = 0 WHERE c IS NULL"
probe "UPDATE ... WHERE TRUE"       "id INT64, c INT64" "UPDATE @T SET c = 0 WHERE TRUE"

echo
echo "done — ${i} cases"
